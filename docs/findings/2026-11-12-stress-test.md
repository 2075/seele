# Stress test (scripts/stress.py)

Phases: idle → CPU burn on all 32 threads → local LLM (Ollama `qwen3.5:4b` directly) → about 40 req/s flood through Traefik on both tiers (401s, no key, so no cost) → cooldown. Each phase peak comes from `GET :6743/state`.

| phase | cpu | gpu | vram_models | local_busy | req_local | req_remote | health |
|---|---|---|---|---|---|---|---|
| idle | 0.06 | 0 | 0.36 | 0 | 0 | 0 | ok |
| cpu | 1.00 | 0 | 0.36 | 0 | 0 | 0 | ok |
| llm | 0.56 | 0.77 | 0.36 | 1.00 | 0 | 0 | ok (after fix 1) |
| flood | 0.06 | 0.53 | 0.36 | 0.02 | 1.00 | 1.00 | ok |
| cooldown | 0.05 | 0 | 0.36 | 0 | 1.00 → decays | 1.00 → decays | ok |

## Fixes from the test
1. **Ollama went "degraded" under generation.** A runner's `/slots` blocks while it's generating, so the request timed out and was counted as an error. A timeout is now read as busy (`local_busy=1`).
2. **seele itself used about 5.8% of a core (4.8% of it kernel time).** `psutil.sensors_temperatures()` read every hwmon chip (asusec, nct6798, nzxtsmart2, x53) twice a second, with about 16 ms of blocking each time. Reading only `k10temp/temp1_input` brought it down to **0.9%** (0.8% user, 0.1% sys). Render plus SDK send is 0.35 ms per frame.
3. The stress script's `pkill -f` matched its own shell. Now it auto-detects the process by PID.

## Not covered / known limits
- `req_*` saturates at 2 req/s (`min(1, rps/2)`). That's fine for real LLM traffic, but the flood pins it at 1. Pulses are capped at 24 alive at once.
- `tok_*`, `spend_today`, `err_rate`, `egress_*` and the alert flash weren't exercised. They need authenticated LLM calls through LiteLLM (a key; remote calls cost money) or a failing container. Errors only count 5xx and 429, so the 401 flood is correctly 0.
- ~~The LLM phase called Ollama directly~~. Fixed: it now goes through the gateway, see the rerun below.

## Rerun through the gateway (2026-11-12)
The LLM phase now calls `127.0.0.1:4000/v1/chat/completions` (model `local-small`). It goes through the real path: Traefik → guard → LiteLLM gw-local → Ollama. The key is `seele-stress`, created on gw-local only via `../machine/scripts/new-key.sh` (models: local-small, $1/30d). It's stored in `.secrets/gw-local.key` (mode 600, gitignored).

| phase | gpu | local_busy | req_local | egress_local | tok_local | health |
|---|---|---|---|---|---|---|
| llm | 0.78 | 1.00 | 0.15 | 0.12 | 0.55 | ok |

So request pulses, egress, tokens/s and "generating" now all fire together on one real call. `tok_local` lingers about 30 s afterwards (its window). seele used 2–4% of a core under load.
Still untested: `spend_today` (local tier has no cost), `err_rate`, alert flash, remote tier.

## Run 3, against the systemd services (2026-11-12 evening)
- Found `openrgb-server.service` in a restart loop (14 restarts): port 6742 was held by the temporary server started earlier in a shell. Because `seele.service` `Requires=` it, seele restarted along with it. The stale server was killed and the service recovered.
- **Security:** OpenRGB's SDK listens on `0.0.0.0` by default (unauthenticated, reachable over the LAN and Tailscale). The unit now passes `--server-host 127.0.0.1`. Applied and verified at 22:43: bound to `127.0.0.1:6742`, closed on the LAN.
- Full-length results match run 2 (gpu 0.79, local_busy 1, tok_local 0.56, req both 1.0 under flood). seele used 1–4% of a core, with 0 restarts.
- Ollama "degraded" blip: an idle model unloads, and the new runner refuses connections for a moment. Health now degrades only after 3 consecutive failures. `stress.py` takes seele's PID from systemd and follows it across restarts.
