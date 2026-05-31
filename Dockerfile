FROM rust:1-bookworm AS builder

WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY web ./web

RUN cargo build --release -p gateway

FROM debian:bookworm-slim AS runtime

WORKDIR /app

LABEL org.opencontainers.image.title="castkit gateway" \
      org.opencontainers.image.description="Castkit gateway server" \
      org.opencontainers.image.source="https://github.com/vyvhouse/castkit" \
      org.opencontainers.image.licenses="MIT"

COPY --from=builder /app/target/release/gateway ./gateway
COPY --from=builder /app/web ./web

EXPOSE 8080

ENV RUST_LOG=info

ENTRYPOINT ["./gateway"]
CMD ["--port", "8080", "--web-dir", "/app/web"]
