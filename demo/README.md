# Client demo bundle

One-click demo for showing WEFT ERP to a client. No installs: it carries its own PostgreSQL, the API and the web app, preloaded with a sample Bangladesh importer company (mock data from `backend/scripts/seed_demo.py`).

## Get it
GitHub → **Actions → "Client demo (Windows installer + Linux AppImage)" → Run workflow**. Download from the run's *Artifacts* (or from **Releases** when you push a tag such as `v1.0.0`):

| File | For |
|---|---|
| `WEFT-ERP-Demo-Setup.exe` | Windows installer |
| `WEFT-ERP-Demo-windows-portable.zip` | Windows, unzip and run `WEFT-ERP-Demo.exe` |
| `WEFT-ERP-Demo-x86_64.AppImage` | Linux: `chmod +x` and run |

## Use it
Start it, the browser opens at `http://localhost:8080`. Sign in `owner@petra.example` / `Demo@12345`. Other roles for role demos: `sales@`, `store@`, `hr@` `petra.example`, same password. Keep the window open; close it or press Ctrl+C to stop.
Data lives in `%LOCALAPPDATA%\WEFT-ERP-Demo` (Windows) or `~/.local/share/WEFT-ERP-Demo` (Linux); changes made during a demo persist. Run with `--reset` to restore the original sample data.

## Notes
- First start takes ~30–60 s (creates the database), later starts a few seconds.
- Windows may show a SmartScreen warning (unsigned): "More info → Run anyway". Needs the Microsoft Visual C++ runtime (already on most PCs).
- Listens on localhost only. PostgreSQL is a private copy (trust auth on 127.0.0.1), not for production use.
- Build locally: `cd demo && cargo build --release`, then place `pg/` (PostgreSQL binaries), `web/` (`frontend/dist`) and `demo.sql` (pg_dump of a seeded DB) next to the binary, or point `WEFT_HOME` at that folder.
- Linux AppImage needs glibc 2.36 or newer (Ubuntu 24.04+, Debian 12+, Fedora 37+) because of the bundled PostgreSQL.
