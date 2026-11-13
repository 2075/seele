# Collector contract (for implementers)

- One module per source in `seele/collectors/<name>.py`, exposing `async def run(state, **opts) -> None`. It loops forever and must **never raise**: catch errors, log at most once a minute, set `state.health[<name>] = "down"`, back off, retry.
- Only stdlib, `httpx`, `psutil` and `docker` CLI/socket access. Don't block the loop: use `asyncio.create_subprocess_exec` or `asyncio.to_thread`.
- `state.push(name, x, tau=...)`: continuous signals, **pre-normalised to 0..1** (clamped).
- `state.events.emit(kind, **attrs)`: discrete events.
- `state.health[name] = "ok" | "degraded" | "down"`.
- Privacy: never read or log prompt or response bodies, keys or secrets. Metadata only.
- Don't change anything in `../machine` (read-only consumer).

## Names (keep these exact)
| name | kind | meaning |
|---|---|---|
| `req_local`, `req_remote` | signal | request rate per tier, normalised (`min(1, rps/2)`) |
| `err_rate` | signal | share of recent requests that are 5xx, 429, 401 or 403 (auth failures count, ADR-less user decision 2026-11-12) |
| `egress_local`, `egress_remote` | signal | egress container rx+tx bytes/s, log-normalised (1 KB/s→0, 10 MB/s→1) |
| `containers_bad` | signal | unhealthy/exited share of research-stack containers |
| `local_busy` | signal | 1 if any local llama-server slot `is_processing`, else 0 |
| `vram_models` | signal | Ollama `/api/ps` size_vram sum / GPU VRAM total |
| `tok_local`, `tok_remote` | signal | tokens/s per tier, log-normalised (1→0, 500→1) |
| `spend_today` | signal | today's remote spend / budget (config `budget_eur_day`, default 5) |
| `gateway_down` | signal | 0 both tiers reachable, 0.5 one tier down, 1 both down (via `/health/liveliness` through Traefik) |
| `containers_bad` note | | **missing** expected core services count as bad (compose down removes containers) |
| `request` | event | attrs: `tier` ("local"/"remote"), `status` (int), `ms` (float) |
| `alert` | event | attrs: `source`, `level` ("warn"/"crit"), `msg` |
| `container` | event | attrs: `name`, `state` (e.g. "unhealthy", "die", "start") |
