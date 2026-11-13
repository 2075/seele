# Plan: seele, a machine-state monitor shown through OpenRGB lighting

Status: implemented (phases 1–5; service install pending) · 2026-11-12 · Inputs: [local model metrics finding](../findings/2026-11-12-local-model-metrics.md), survey of `../machine`

## Goal
Show how loaded the machine is, how much traffic it handles, and whether LLM calls go to **local** (Ollama) or **remote** (OpenRouter) models, using the lighting on the machine's RGB hardware.

## Architecture
```
collectors (async, one per source) ──▶ state model (smoothed, normalised 0..1)
                                           │
                                     scene mapper (signals → zones/colours/effects)
                                           │
                                     OpenRGB SDK client (TCP :6742) ──▶ devices
```
- A single Python daemon (asyncio). Libraries: `openrgb-python`, `psutil`, `pynvml`, `httpx`, `docker`/raw socket, `asyncpg`.
- Collectors publish to a common state dict. The mapper renders at about 20–30 fps and sends updates only when colours change, so the SDK isn't flooded.
- Smoothing: an exponential moving average (EMA) per signal, plus hysteresis on alert states, so lights don't flicker.
- Configured from a YAML file that maps signals to devices and zones. A `--dry-run` mode renders the lighting to the terminal so we can develop without hardware.

## Tapping into OpenRGB
- Run `openrgb --server` (SDK on port 6742) as a systemd user service. seele connects as a client and never drives the hardware itself.
- **Blocker found:** `openrgb --list-devices` currently shows **no devices**, because the `i2c-dev` module isn't loaded, so motherboard and RAM lighting over SMBus is invisible to it. Fix: `modprobe i2c-dev`, make it persistent via `/etc/modules-load.d/i2c.conf`, add udev rules for OpenRGB (`60-openrgb.rules`), and add the user to the `i2c` group. USB and GPU devices may need their own udev rules.
- Set each device to its Direct/Custom mode, which supports per-LED updates, and use `update_leds` / `set_colors(fast=True)`.

## Signal sources
| Signal | Source | How | Notes |
|---|---|---|---|
| CPU / RAM / load avg | host | `psutil`, `/proc/loadavg` | 32 threads (Ryzen/Matisse) |
| GPU util, VRAM, temperature, power | RTX 3070 | NVML (`pynvml`) | VRAM also shows how much room local models take |
| Network throughput | host NICs | `psutil.net_io_counters(pernic=True)` | |
| Container health and state | Docker | Engine API, `/events` stream plus `/containers/json` | Use **docker-socket-proxy** (read-only); the raw socket gives root-equivalent access |
| Per-tier traffic (bytes) | Docker | `/containers/{id}/stats` for `egress-local-1` and `egress-open-1` | Local-vs-remote split **without changing `machine`** |
| Requests: rate, status, latency, local vs remote | Traefik access log | follow `docker logs -f research-stack-edge-1` (JSON); path `/v1/*` = local, `/open/v1/*` = remote | Already enabled. No Prometheus metrics today |
| Tokens and spend per model | LiteLLM Postgres `LiteLLM_SpendLogs` | read-only role, poll for new rows every 2–5 s | Only metadata is stored (prompts are off), which suits privacy |
| Which local models are loaded | Ollama `/api/ps` | poll | stable API |
| Local model currently generating | `llama-server` runner `/slots` (`is_processing`) | find ports via `pgrep` | best effort; the port changes when a model reloads |

### Optional changes to `machine` (that repo's decision, so ADRs go there)
- Add a Traefik `metrics.prometheus` endpoint on an internal entrypoint, bound to loopback only (`smoke-test.sh` currently asserts that `/metrics` is blocked, so the test would need updating).
- A LiteLLM Prometheus callback, which would give tokens/sec and time-to-first-token (TTFT) per model. Check whether that feature is OSS or enterprise in the deployed version.
- Running a standalone `llama-server --metrics` instead of Ollama runners would give real tokens/sec for local models.

## Visual language (first draft)
- **Base layer, machine load:** a colour gradient on the case and RAM zones from calm blue (idle) through amber to red (saturated), with CPU and GPU blended. Brightness rises slightly with power draw.
- **Traffic layer:** a pulse runs along the LED strips once per request. **Local = warm green or teal**, **remote = violet**. Pulse speed follows tokens/sec and pulse width follows request size.
- **Local inference:** the GPU's own LEDs "breathe" while a runner slot is processing. Each loaded model lights a segment (a VRAM bar).
- **Remote spend:** a slow violet fill on one zone that resets daily, showing spend against budget.
- **Alerts take priority:** 5xx errors or a 429 give a short red or orange flash. An unhealthy container or a guard rejection holds amber on the affected zone. When the edge is down, everything pulses dim red slowly.
- **Quiet mode:** a schedule or idle detection dims the lighting.

## Phases
1. **Hardware readiness:** fix i2c-dev and udev, enumerate devices and zones, and record them in a finding.
2. **Skeleton:** the daemon, the OpenRGB client, the terminal dry-run renderer, and the host plus NVML collectors driving the base load layer.
3. **Docker and traffic:** a socket proxy, health and egress stats, and a Traefik access-log tailer that splits local from remote.
4. **LLM layer:** LiteLLM SpendLogs (read-only DB role), Ollama `/api/ps` and `/slots`.
5. **Polish:** scene config, alert priorities, quiet hours, a systemd unit, and a small HTTP `/state` endpoint for debugging.
6. *(optional)* The Prometheus changes in `machine`.

## Open questions
- Which RGB devices exist and how many addressable LEDs each has (this depends on phase 1)?
- Does seele run on the same host as `machine` (it appears to), or does it need remote access?
- Should seele get read access to the LiteLLM database, or is the Traefik log plus egress stats enough?
- What are the colour and brightness preferences, for example for nighttime?
