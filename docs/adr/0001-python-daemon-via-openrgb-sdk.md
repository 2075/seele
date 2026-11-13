# 0001: Python asyncio daemon driving OpenRGB through its SDK server

Status: accepted · 2026-11-12

## Context
seele needs to read several metric sources (host, NVML, Docker, Traefik logs, LiteLLM, Ollama) and drive 47 LEDs across 4 devices (49 detected, minus the mouse) at about 20 fps. OpenRGB can be embedded (C++ plugin) or controlled over its network SDK (port 6742).

## Decision
- seele is a **Python asyncio** daemon: collectors → `State` (EMA-smoothed 0..1 signals) → `scene.render` → sink.
- Hardware access goes **only through `openrgb --server`**, using `openrgb-python`. seele never touches i2c or hidraw itself.
- Config in TOML (stdlib `tomllib`). Each zone is bound to a *role*, and the scene maps roles to signals.
- A terminal sink (`--dry-run`) mirrors the hardware sink so we can develop without the lights.
- The mouse and the unsized Aura ARGB headers are out of scope (user decision, 2026-11-12).

## Consequences
- The server holds the device permissions (it needs the `i2c` and `plugdev` groups); seele can run unprivileged.
- At startup the server detects devices asynchronously, so the sink waits up to 30 s for every configured device.
- Python's per-frame cost is fine at this LED count. The sink skips zones that haven't changed.
