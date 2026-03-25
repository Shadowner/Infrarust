ARG RUST_VERSION=1.94
ARG ALPINE_VERSION=3.21
ARG NODE_VERSION=20

# Stage 1: Build the admin frontend
FROM node:${NODE_VERSION}-alpine AS frontend-builder

WORKDIR /frontend
COPY plugins/infrarust-plugin-admin-api/frontend/package.json \
     plugins/infrarust-plugin-admin-api/frontend/yarn.lock ./
RUN yarn install --frozen-lockfile

COPY plugins/infrarust-plugin-admin-api/frontend/ ./
RUN yarn generate

# Stage 2: Build Rust binary
FROM rust:${RUST_VERSION}-alpine${ALPINE_VERSION} AS builder

RUN apk add --no-cache \
    musl-dev \
    pkgconfig \
    openssl-dev \
    openssl-libs-static \
    git \
    build-base

WORKDIR /usr/crates/infrarust

COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/
COPY plugins/ ./plugins/
COPY tools/ ./tools/
COPY data/ ./data/

COPY --from=frontend-builder /frontend/.output/public/ \
     ./plugins/infrarust-plugin-admin-api/frontend/.output/public/

ENV OPENSSL_STATIC=1
ENV RUSTFLAGS="-C target-feature=+crt-static"

RUN ARCH=$(uname -m) && \
    case "$ARCH" in \
        x86_64) \
            TARGET="x86_64-unknown-linux-musl" \
            ;; \
        aarch64) \
            TARGET="aarch64-unknown-linux-musl" \
            ;; \
        armv7l) \
            TARGET="armv7-unknown-linux-musleabihf" \
            ;; \
        *) \
            echo "Unsupported architecture: $ARCH" && exit 1 \
            ;; \
    esac && \
    echo "Building for target: $TARGET on architecture: $ARCH" && \
    rustup target add "$TARGET" && \
    cargo build --release --target "$TARGET" -p infrarust-proxy && \
    cp "target/$TARGET/release/infrarust" /usr/local/bin/infrarust && \
    strip /usr/local/bin/infrarust && \
    echo "Build completed successfully"

# Stage 3: Runtime
FROM scratch AS runtime

COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
COPY --from=builder /usr/local/bin/infrarust /sbin/infrarust

WORKDIR /app
VOLUME ["/app/config"]
EXPOSE 25565

ENTRYPOINT ["/sbin/infrarust"]
CMD ["--config-path", "/app/config/config.yaml", "--proxies-path", "/app/config/proxies", "--no-interactive"]