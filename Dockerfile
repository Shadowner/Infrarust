FROM rust:1.84-slim AS builder
WORKDIR /usr/src/infrarust
RUN apt update && apt install -y pkg-config libssl-dev
COPY Cargo.toml Cargo.lock ./
COPY src/ ./src/
RUN cargo build --release


FROM alpine:3.14
WORKDIR /app

COPY --from=builder /usr/src/infrarust/target/release/infrarust /app/
RUN mkdir /app/config

VOLUME ["/app/config"]
EXPOSE 25565

ENTRYPOINT ["/app/infrarust"]
CMD ["--config-path", "/app/config/config.yaml", "--proxies-path", "/app/config/proxies"]