ARG RUST_VERSION=1.87
ARG ALPINE_VERSION=3.21

# Build stage - builds natively for the target platform
FROM rust:${RUST_VERSION}-alpine${ALPINE_VERSION} AS builder

# Install build dependencies
RUN apk add --no-cache \
    musl-dev \
    pkgconfig \
    openssl-dev \
    openssl-libs-static \
    git \
    build-base

WORKDIR /usr/crates/infrarust

# Copy source code
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/

# Set environment variables for static linking
ENV OPENSSL_STATIC=1
ENV RUSTFLAGS="-C target-feature=+crt-static"

# Determine target based on architecture and build
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
    cargo build --release --target "$TARGET" && \
    cp "target/$TARGET/release/infrarust" /usr/local/bin/infrarust && \
    strip /usr/local/bin/infrarust && \
    echo "Build completed successfully"

# Runtime stage - use scratch for minimal image since we have static binary
FROM scratch AS runtime

# Copy CA certificates for HTTPS
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/

# Copy only the binary
COPY --from=builder /usr/local/bin/infrarust /sbin/infrarust

# Set up the runtime environment
WORKDIR /app
VOLUME ["/app/config"]
EXPOSE 25565

ENTRYPOINT ["/sbin/infrarust"]
CMD ["--config-path", "/app/config/config.yaml", "--proxies-path", "/app/config/proxies", "--no-interactive"]