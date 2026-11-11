# syntax=docker/dockerfile:1
FROM rust:1.90-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home erp
COPY --from=build /app/target/release/weft-erp /usr/local/bin/weft-erp
USER erp
ENV BIND_ADDR=0.0.0.0:8080 RUST_LOG=info
EXPOSE 8080
HEALTHCHECK --interval=15s --timeout=3s --retries=5 CMD curl -fsS http://127.0.0.1:8080/health || exit 1
CMD ["weft-erp"]
