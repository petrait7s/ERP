# WEFT ERP backend

Rust (Axum, Tokio, SQLx, PostgreSQL) backend for a Bangladesh-first import and distribution business.

## Run

```bash
cp .env.example .env            # edit JWT_SECRET (32+ characters) and DATABASE_URL
createdb erp                    # or: docker compose up db
cargo run --release             # applies migrations on start, listens on :8080
python3 scripts/seed_demo.py    # optional: load a realistic demo company
```

First run: `POST /api/v1/setup` creates the company, its chart of accounts, roles, numbering, approval rules and
fiscal periods, and the owner account (the demo script does this for you).

## Test

```bash
cargo test                      # unit tests + integration tests against a throw-away PostgreSQL database per test
```

Integration tests need a reachable PostgreSQL (`TEST_ADMIN_URL`, default `postgres://erp:erp@localhost:5432/postgres`,
a role that can `CREATE DATABASE`).

## API

* Machine-readable: `GET /api/v1/openapi.json` (generated from the route table, so it cannot drift).
* Human-readable: [`docs/API.md`](docs/API.md), regenerate with `weft-erp --print-api-docs`.
* Conventions: lists return `{data,total,page,per_page}`, single records `{data}`, errors `{error:{code,message,details}}`.

## Design rules

* **All money is `NUMERIC`/`rust_decimal`.** Paisa rounding is half away from zero, applied per line.
* **Every financial effect goes through one posting function** (`modules/accounting.rs`). Journals must balance
  (enforced again by a deferred database constraint), are immutable once posted, and are corrected by reversal.
* **Closed accounting periods reject postings** (database trigger).
* **Every document carries its journal entry**, and ledger sub-totals reconcile to sub-ledgers (stock valuation = GL inventory,
  customer statement = receivable).
* **Tenant isolation:** every query filters by `company_id` taken from the verified session, never from the request.
* **Permissions** are `module.action` strings carried by roles; each route declares the one it needs and the OpenAPI document shows it.
