ARG RUST_VERSION=1.85
ARG DEBIAN_VERSION=bookworm
ARG DEBIAN_VERSION_NUMBER=12

FROM rust:${RUST_VERSION}-slim-${DEBIAN_VERSION} AS builder
WORKDIR /usr/src/infrarust

# Prevent deletion of apt cache
RUN rm -f /etc/apt/apt.conf.d/docker-clean

RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt update && apt install -y pkg-config libssl-dev

# Create config directory for runtime
RUN mkdir -p /rootfs/app/config

COPY Cargo.toml Cargo.lock ./
COPY src/ ./src/
RUN mkdir sbin

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git/db \
    --mount=type=cache,target=/usr/src/infrarust/target \
    cargo build --release & \
    cp target/release/infrarust sbin/


# FROM debian:${DEBIAN_VERSION}-slim
FROM gcr.io/distroless/cc-debian${DEBIAN_VERSION_NUMBER}
WORKDIR /app

COPY --from=builder /rootfs/ /
COPY --from=builder /usr/src/infrarust/sbin /sbin

VOLUME ["/app/config"]
EXPOSE 25565

ENTRYPOINT ["/sbin/infrarust"]
CMD ["--config-path", "/app/config/config.yaml", "--proxies-path", "/app/config/proxies"]
