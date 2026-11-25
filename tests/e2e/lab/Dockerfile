# syntax=docker/dockerfile:1
ARG RUST_VERSION=1.99.0
ARG ALPINE_VERSION=3.24
ARG NODE_VERSION=24.21.0

FROM docker.io/library/node:${NODE_VERSION}-alpine${ALPINE_VERSION} AS frontend
WORKDIR /frontend
COPY plugins/infrarust-plugin-admin-api/frontend/ ./
RUN yarn install --frozen-lockfile
RUN yarn generate

FROM docker.io/library/rust:${RUST_VERSION}-alpine${ALPINE_VERSION} AS builder
RUN apk add --no-cache musl-dev pkgconfig openssl-dev openssl-libs-static git build-base
RUN rustup target add x86_64-unknown-linux-musl
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/
COPY plugins/ ./plugins/
COPY tools/ ./tools/
COPY --from=frontend /frontend/.output/public/ ./plugins/infrarust-plugin-admin-api/frontend/.output/public/

ARG LAB_BASE_VERSION
ARG LAB_VERSION
ARG LAB_PUBKEY
ARG LAB_OFFICIAL=1
ARG LAB_DOCKER=0
ARG LAB_PATCH=none
ARG LAB_LTO=fat

RUN set -eu; \
    sed -i "s/\"${LAB_BASE_VERSION}\"/\"${LAB_VERSION}\"/g" Cargo.toml; \
    grep -q "^version = \"${LAB_VERSION}\"" Cargo.toml; \
    printf 'untrusted comment: minisign public key of the e2e lab, never a release key\n%s\n' "${LAB_PUBKEY}" \
      > crates/infrarust-core/src/update/keys/release-1.pub; \
    case "${LAB_PATCH}" in \
      none) ;; \
      envelope) sed -i 's/^pub const VERSION: u32 = 1;/pub const VERSION: u32 = 2;/' \
          crates/infrarust-core/src/hot_restart/platform/linux/envelope.rs; \
        grep -q '^pub const VERSION: u32 = 2;' crates/infrarust-core/src/hot_restart/platform/linux/envelope.rs ;; \
      protocol) sed -i 's/^pub const HANDOVER_PROTOCOL: u32 = 1;/pub const HANDOVER_PROTOCOL: u32 = 2;/' \
          crates/infrarust-core/src/binary/mod.rs; \
        grep -q '^pub const HANDOVER_PROTOCOL: u32 = 2;' crates/infrarust-core/src/binary/mod.rs ;; \
      *) echo "unknown LAB_PATCH ${LAB_PATCH}" >&2; exit 1 ;; \
    esac

ENV OPENSSL_STATIC=1
ENV RUSTFLAGS="-C target-feature=+crt-static"

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target \
    set -eu; \
    features=""; \
    if [ "${LAB_DOCKER}" = "1" ]; then export INFRARUST_DOCKER_IMAGE=1; features="--features docker"; fi; \
    if [ "${LAB_OFFICIAL}" = "1" ]; then export INFRARUST_OFFICIAL_BUILD=1; fi; \
    if [ "${LAB_LTO}" != "fat" ]; then export CARGO_PROFILE_RELEASE_LTO="${LAB_LTO}" CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16; fi; \
    cargo build --release --target x86_64-unknown-linux-musl -p infrarust $features; \
    mkdir -p /out; \
    cp target/x86_64-unknown-linux-musl/release/infrarust /out/infrarust

RUN mkdir -p /image-root/app/cache/wasm && \
    chown -R 65532:65532 /image-root/app/cache && \
    chmod 0700 /image-root/app/cache /image-root/app/cache/wasm

FROM scratch AS binary
COPY --from=builder /out/infrarust /infrarust

FROM scratch AS runtime
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
COPY --from=builder /out/infrarust /sbin/infrarust
COPY --from=builder /image-root/app /app
WORKDIR /app
VOLUME ["/app/config"]
USER 65532:65532
ENTRYPOINT ["/sbin/infrarust"]
CMD ["--config", "/app/config/infrarust.toml", "--plugins-dir", "/app/config/plugins", "--servers-dir", "/app/config/servers"]
