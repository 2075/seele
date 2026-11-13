# 0003: Package as the `seele-rgb` crate with a self-installing binary

Status: accepted (2026-11-13)

## Context
The Rust build (ADR 0002) is in production, but installing it depends on repo scripts (`scripts/install-services.sh`, `setup-i2c.sh`, `setup-hid.sh`, udev rules) and a config file in the working directory. `cargo install` has no post-install hooks, so it cannot set up systemd units, udev rules or kernel modules. More "seele" tools may follow, so the name should leave room for a family.

## Decision
- The crate and binary are `seele-rgb`, installed with `cargo install` (`--path rust` / `--git` now; crates.io publishing is the user's call).
- The binary sets up the system itself: `seele-rgb install [--dry-run]` (idempotent, escalates with sudo only for root steps, then verifies), `seele-rgb uninstall`, `seele-rgb doctor` (checks only).
- The config moves to `~/.config/seele/rgb.toml`, written from a default compiled into the binary if it doesn't exist. `--config` still overrides it.
- The unit is renamed `seele-rgb.service`. `install` migrates the old `seele.service`.
- An original ASCII banner (a monolith with `SEELE`, red on black, not the show's logo) is shown on install, `--version` and interactive `run`. It's skipped when output isn't a terminal and honours `NO_COLOR`.

## Consequences
- `scripts/install-services.sh`, `setup-*.sh` and `deploy/` are superseded once `install` is verified; they are kept until then.
- Parity tests must not depend on files outside the crate.
- Zone mappings are machine-specific; the default config ships an example only.
