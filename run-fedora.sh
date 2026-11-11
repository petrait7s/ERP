#!/usr/bin/env bash
# One-shot setup and launcher for Fedora Workstation.
#   ./run-fedora.sh            install what is missing, build, start on http://localhost:8080
#   ./run-fedora.sh --demo     same, and load the demo company (owner@petra.example / Demo@12345)
# Safe to run again. Needs sudo for installing packages and configuring PostgreSQL.
set -euo pipefail
cd "$(dirname "$0")"
ROOT="$PWD"
DB_PASS="${DB_PASS:-erp_local_pass}"
PORT="${PORT:-8080}"

say() { printf '\n\033[1;36m==> %s\033[0m\n' "$*"; }

say "1/5 System packages (PostgreSQL, compiler, curl, python)"
sudo dnf install -y postgresql-server postgresql-contrib gcc gcc-c++ make curl python3 git openssl >/dev/null

say "2/5 PostgreSQL"
if [ ! -f /var/lib/pgsql/data/PG_VERSION ]; then sudo postgresql-setup --initdb; fi
HBA=/var/lib/pgsql/data/pg_hba.conf
# Fedora defaults to ident on localhost; the app signs in with a password.
sudo sed -i -E 's/^(host\s+all\s+all\s+(127\.0\.0\.1\/32|::1\/128)\s+)ident/\1scram-sha-256/' "$HBA"
sudo systemctl enable --now postgresql
sudo systemctl reload postgresql
if ! sudo -u postgres psql -tAc "SELECT 1 FROM pg_roles WHERE rolname='erp'" | grep -q 1; then
  sudo -u postgres psql -c "CREATE ROLE erp LOGIN PASSWORD '$DB_PASS'"
fi
if ! sudo -u postgres psql -tAc "SELECT 1 FROM pg_database WHERE datname='erp'" | grep -q 1; then
  sudo -u postgres psql -c "CREATE DATABASE erp OWNER erp"
fi

say "3/5 Rust toolchain"
if ! command -v cargo >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
fi
# shellcheck disable=SC1091
source "$HOME/.cargo/env"

say "4/5 Web app"
if [ ! -f frontend/dist/index.html ]; then
  rustup target add wasm32-unknown-unknown
  command -v trunk >/dev/null 2>&1 || cargo install trunk --locked
  (cd frontend && trunk build --release)
else
  echo "Using the prebuilt web app in frontend/dist (delete that folder to rebuild it)."
fi

say "5/5 Build and start the server (first build takes a few minutes)"
(cd backend && cargo build --release)

SECRET_FILE="$ROOT/.jwt_secret"
[ -f "$SECRET_FILE" ] || { openssl rand -hex 32 > "$SECRET_FILE"; chmod 600 "$SECRET_FILE"; }
export DATABASE_URL="postgres://erp:${DB_PASS}@127.0.0.1:5432/erp"
export JWT_SECRET="$(cat "$SECRET_FILE")"
export STATIC_DIR="$ROOT/frontend/dist"
export BIND_ADDR="0.0.0.0:${PORT}"
export RUST_LOG="${RUST_LOG:-info}"

"$ROOT/backend/target/release/weft-erp" &
SERVER=$!
trap 'kill $SERVER 2>/dev/null || true' EXIT INT TERM
for _ in $(seq 1 60); do curl -fsS "http://127.0.0.1:${PORT}/health" >/dev/null 2>&1 && break; sleep 1; done

if [ "${1:-}" = "--demo" ]; then
  say "Loading demo data"
  python3 backend/scripts/seed_demo.py "http://127.0.0.1:${PORT}" || echo "(demo data was already loaded)"
fi

say "WEFT ERP is running: http://localhost:${PORT}   (Ctrl+C to stop)"
echo "Open the address, then either sign in with the demo user, or use 'First time here? Set up the company'."
sudo -n true 2>/dev/null && command -v firewall-cmd >/dev/null 2>&1 && echo "To reach it from another machine: sudo firewall-cmd --add-port=${PORT}/tcp"
wait $SERVER
