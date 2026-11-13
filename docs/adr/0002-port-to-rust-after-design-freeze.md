# 0002: Port seele to Rust, but only after the design is frozen

Status: accepted · 2026-11-12 · **Freeze declared 2026-11-13** (user), with remote-tier signals (spend_today, tok_remote) still unverified live: gw-open has no upstream key. Verify them on the Rust build. · Amends [0001](0001-python-daemon-via-openrgb-sdk.md) (Python is the prototype language)

## Context
The user prefers Rust long term. Measurements (findings/2026-11-12-stress-test.md): 0.9% of a core idle, 2–4% under load, 0.35 ms per frame for render plus send, about 63 MB RSS. Performance is not a driver. A single static binary, stronger types (signal names as an enum instead of strings) and less memory are. The scene, colours and thresholds aren't tuned yet. Spend, err_rate, alerts and the remote tier haven't been tested live. The codebase is about 950 lines plus 17 tests.

## Decision
- Keep iterating on the design in Python, where hot-reload makes it fast, until it's frozen. Frozen means: every signal has been exercised live, the scene layers and colours are signed off by the user, and the collector contract has stopped changing.
- Then port in one go (not piecemeal), with the Python tests and `docs/agents/collector-contract.md` as the spec. Candidate crates: tokio, nvml-wrapper, sysinfo, bollard. For OpenRGB, check the maturity of the existing crates; the SDK protocol is simple, so a small client of our own is the fallback.
- Validate by running both implementations side by side in `--dry-run` and diffing their frames before switching the systemd unit.

## Consequences
- No new Python-only dependencies or tricks that would be hard to port.
- Design changes go into the contract doc and the tests, so the port has an exact spec.
