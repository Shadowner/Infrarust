ARG RUST_VERSION=1.87
ARG ALPINE_VERSION=3.21

# Build stage
FROM --platform=$BUILDPLATFORM rust:${RUST_VERSION}-alpine${ALPINE_VERSION} AS builder

# Install build dependencies
RUN apk add --no-cache \
    musl-dev \
    pkgconfig \
    openssl-dev \
    openssl-libs-static \
    git \
    build-base \
    wget \
    tar

# Set up cross-compilation based on target architecture
ARG TARGETPLATFORM

# Add Rust targets and install cross-compilers
RUN case "$TARGETPLATFORM" in \
    "linux/amd64") \
    rustup target add x86_64-unknown-linux-musl \
    ;; \
    "linux/arm64") \
    rustup target add aarch64-unknown-linux-musl && \
    cd /tmp && \
    wget -q https://musl.cc/aarch64-linux-musl-cross.tgz && \
    tar -xzf aarch64-linux-musl-cross.tgz -C /opt && \
    rm aarch64-linux-musl-cross.tgz \
    ;; \
    "linux/arm/v7") \
    rustup target add armv7-unknown-linux-musleabihf && \
    cd /tmp && \
    wget -q https://musl.cc/armv7l-linux-musleabihf-cross.tgz && \
    tar -xzf armv7l-linux-musleabihf-cross.tgz -C /opt && \
    rm armv7l-linux-musleabihf-cross.tgz \
    ;; \
    esac

WORKDIR /usr/crates/infrarust

# Copy source code
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/

# Create directories
RUN mkdir -p /rootfs/app/config sbin

# Set environment variables for static linking and cross-compilation
ENV OPENSSL_STATIC=1
ENV PKG_CONFIG_ALLOW_CROSS=1

# Configure cross-compilation environment variables and build
RUN case "$TARGETPLATFORM" in \
    "linux/amd64") \
    cargo build --release --target x86_64-unknown-linux-musl && \
    cp target/x86_64-unknown-linux-musl/release/infrarust sbin/ \
    ;; \
    "linux/arm64") \
    export PATH="/opt/aarch64-linux-musl-cross/bin:$PATH" && \
    export CC_aarch64_unknown_linux_musl=aarch64-linux-musl-gcc && \
    export CXX_aarch64_unknown_linux_musl=aarch64-linux-musl-g++ && \
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-linux-musl-gcc && \
    cargo build --release --target aarch64-unknown-linux-musl && \
    cp target/aarch64-unknown-linux-musl/release/infrarust sbin/ \
    ;; \
    "linux/arm/v7") \
    export PATH="/opt/armv7l-linux-musleabihf-cross/bin:$PATH" && \
    export CC_armv7_unknown_linux_musleabihf=armv7l-linux-musleabihf-gcc && \
    export CXX_armv7_unknown_linux_musleabihf=armv7l-linux-musleabihf-g++ && \
    export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_MUSLEABIHF_LINKER=armv7l-linux-musleabihf-gcc && \
    cargo build --release --target armv7-unknown-linux-musleabihf && \
    cp target/armv7-unknown-linux-musleabihf/release/infrarust sbin/ \
    ;; \
    esac

# Runtime stage - use scratch for minimal image since we have static binary
FROM scratch AS runtime

# Copy CA certificates for HTTPS
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/

# Copy application structure
COPY --from=builder /rootfs/ /
COPY --from=builder /usr/crates/infrarust/sbin/infrarust /sbin/infrarust

# Set up the runtime environment
WORKDIR /app
VOLUME ["/app/config"]
EXPOSE 25565

ENTRYPOINT ["/sbin/infrarust"]
CMD ["--config-path", "/app/config/config.yaml", "--proxies-path", "/app/config/proxies", "--no-interactive"]