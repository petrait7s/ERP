# syntax=docker/dockerfile:1
# One image: the Rust API plus the Leptos web app it serves. Build from the repository root:
#   docker build -t weft-erp .
# NOTE: written carefully but not executed in the authoring sandbox (no Docker daemon there).

# ---- web app (Rust -> WebAssembly) ----
FROM rust:1.90-bookworm AS web
RUN rustup target add wasm32-unknown-unknown \
 && ( curl -fsSL https://github.com/trunk-rs/trunk/releases/download/v0.21.14/trunk-x86_64-unknown-linux-gnu.tar.gz | tar -xz -C /usr/local/bin || cargo install trunk --locked )
WORKDIR /web
COPY frontend/Cargo.toml frontend/Cargo.lock frontend/Trunk.toml frontend/index.html frontend/style.css ./
COPY frontend/src ./src
RUN trunk build --release

# ---- API ----
FROM rust:1.90-bookworm AS api
WORKDIR /app
COPY backend/Cargo.toml backend/Cargo.lock ./
COPY backend/src ./src
COPY backend/migrations ./migrations
RUN cargo build --release --locked

# ---- runtime ----
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home erp
COPY --from=api /app/target/release/weft-erp /usr/local/bin/weft-erp
COPY --from=web /web/dist /srv/web
USER erp
ENV BIND_ADDR=0.0.0.0:8080 STATIC_DIR=/srv/web RUST_LOG=info
EXPOSE 8080
HEALTHCHECK --interval=15s --timeout=3s --retries=5 CMD curl -fsS http://127.0.0.1:8080/health || exit 1
CMD ["weft-erp"]
