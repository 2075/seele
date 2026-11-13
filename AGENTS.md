# seele

## Agent skills

### Issue tracker

Issues live as local markdown files under `.scratch/<feature>/` (no git remote yet). See `docs/agents/issue-tracker.md`.

### Domain docs

Single-context: one `GLOSSARY.md` at the repo root plus `docs/adr/`. See `docs/agents/domain.md`.

## Development

- Setup: `python3 -m venv .venv && .venv/bin/pip install -e .`
- Preview without hardware: `.venv/bin/seele run --dry-run`
- Hardware: requires `openrgb --server` (port 6742) running with the `i2c` and `plugdev` groups (e.g. `sg i2c -c 'openrgb --server'`), then `.venv/bin/seele run`
- Map zones physically: `scripts/identify.sh` (pauses the service, lights each zone white in turn, restarts it)
- `~/.config/seele/rgb.toml` hot-reloads every 2 s (zones, roles, brightness, quiet hours); changing collectors needs `sudo systemctl restart seele-rgb`
- Device inventory and permission setup: `docs/findings/2026-11-12-openrgb-devices.md`
- Tests: `.venv/bin/python -m pytest -q`
- Live state: `curl -s 127.0.0.1:6743/state`
- Services: `seele-rgb install` (see Rust section)
- Lighting layers and colours: `seele/scene.py` docstring. Collector names: `docs/agents/collector-contract.md`
- Stress test: `.venv/bin/python scripts/stress.py [--scale 0.5]` (needs seele running)

## Rust (production, ADR 0002)

- Crate/binary: `seele-rgb` (ADR 0003). Build: `cd rust && cargo build --release` → `rust/target/release/seele-rgb`; install with `cargo install --path rust`
- Setup: `seele-rgb install [--dry-run]` / `uninstall` / `doctor`; config default `~/.config/seele/rgb.toml`, written from `rust/assets/default-config.toml`; see `rust/README.md`
- Tests: `cd rust && cargo test` (parity vs `rust/tests/parity/golden.json`); live checks: `cargo test -- --ignored --nocapture --test-threads=1`
- Side by side: `rust/target/release/seele-rgb run --dry-run --state-port 6744`
- Changing the scene: edit Python and Rust together and regenerate golden via `.venv/bin/seele replay tests/parity/fixture.json > tests/parity/golden.json`, then copy fixture/golden to `rust/tests/parity/`, until Python is retired
