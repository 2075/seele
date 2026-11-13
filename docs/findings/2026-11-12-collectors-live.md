# Collectors: how each source is read, and live check

| Collector | Access | Signals |
|---|---|---|
| host | psutil, NVML | cpu, mem, cpu_temp, gpu, vram, gpu_power, gpu_temp |
| docker | Engine API over `/var/run/docker.sock` (user is in the docker group). Persistent `stats?stream=true` for both egress containers, plus `/events` | containers_bad, egress_local/remote; `container` and `alert` events |
| traefik | `docker logs -f research-stack-edge-1` (JSON access log). `/v1/*` = local, `/open/*` = remote | req_local/remote, err_rate (5xx+429); a `request` event per call, 4xx included |
| ollama | `/api/ps`; runner ports from `pgrep -af llama-server` (executable must be llama-server), then `/slots` | vram_models, local_busy |
| litellm | `docker exec -e PGOPTIONS='-c default_transaction_read_only=on' research-stack-litellm-db-1 psql -U gw_local\|gw_open`. The local socket is trusted, so no secrets are used. Only `startTime, total_tokens, spend` are selected | tok_local/remote, spend_today (USD treated as EUR) |

Live check, 2026-11-12: all four collectors `ok`. curl to `/v1/models` and `/open/v1/models` gave req_local = req_remote ≈ 0.2. Not yet seen non-zero: tok_*, local_busy, spend_today, egress_* (no LLM traffic during the test).

The debug endpoint `GET 127.0.0.1:6743/state` returns every signal plus collector health.
Gotcha: `pgrep -af llama-server` also matches shells whose command line contains that string, so match on the executable's basename.
