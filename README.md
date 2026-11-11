# WEFT ERP

A Bangladesh-first ERP for product-importing businesses. Rust backend (Axum, Tokio, SQLx, PostgreSQL) and a Rust/WebAssembly
web app (Leptos). BDT is the base currency, Bangladesh Standard Time and Bangla/English are built in, and VAT, Mushak forms and
customs duty (CD, RD, SD, VAT, AIT, AT) are configurable.

| | |
|---|---|
| `backend/` | REST API, database migrations, business logic, automated tests, API reference (`backend/docs/API.md`) |
| `frontend/` | Leptos web app, end-to-end browser test (`frontend/e2e/e2e.js`) |
| `ARCHITECTURE.md` | Design notes (the ledger-first architecture) |
| `web/overview/` | The original clickable UI prototype (static, mock data) |

## Run locally

Requirements: Rust (stable) with the `wasm32-unknown-unknown` target, [`trunk`](https://trunkrs.dev), PostgreSQL 16.

```bash
createdb erp
rustup target add wasm32-unknown-unknown && cargo install trunk --locked

# build the web app, then start the API, which also serves it
(cd frontend && trunk build --release)
cd backend
export DATABASE_URL=postgres://erp:erp@localhost/erp JWT_SECRET=$(openssl rand -hex 32) STATIC_DIR=../frontend/dist
cargo run --release                       # http://localhost:8080 (migrations run at start)
```

```bash
# optional: load a realistic demo company (owner@meghna.example / Demo@12345)
python3 backend/scripts/seed_demo.py http://localhost:8080
```

Or on a fresh database open http://localhost:8080 and use "First time here? Set up the company".

## Docker

```bash
JWT_SECRET=$(openssl rand -hex 32) docker compose up --build        # http://localhost:8080
```

## Tests

```bash
cd backend && cargo test                  # 13 unit + 13 integration tests against throw-away PostgreSQL databases
cd frontend && BASE=http://localhost:8080 node e2e/e2e.js     # 40 browser checks (needs the demo data and Playwright)
```

## Development

* Frontend hot rebuild: `cd frontend && trunk serve --proxy-backend=http://localhost:8080/api`
  (or open the app with `?api=http://localhost:8080` while serving the frontend separately).
* The OpenAPI document is generated from the routes: `GET /api/v1/openapi.json`; the Markdown reference with
  `weft-erp --print-api-docs > backend/docs/API.md`.
