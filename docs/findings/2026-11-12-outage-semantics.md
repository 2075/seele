# Outage semantics, and gw-open has no upstream credentials

## Bug: `docker compose down` looked healthy
`compose down` *removes* containers. `containers_bad` was the bad share of the *listed* containers, so an empty list gave 0, which read as healthy. The only visible sign was the 1.5 s flash from the `die` events.

## Fix: status is now a persistent state (scene layer 6)
- `containers_bad` counts expected core services (`edge, guard, gw-local, gw-open, litellm-db, egress-local, egress-open`) that are **missing**, not running, or unhealthy.
- New `gateway` collector: probes `/health/liveliness` and `/open/health/liveliness` through Traefik every 2 s; 2 consecutive failures = down. Signal `gateway_down` is 0, 0.5 or 1.
- `severity()`: **outage** (both tiers unreachable, or ≥ 50% of core services bad) → every zone breathes red slowly (3 s period) until it recovers. **degraded** (one tier down, any service bad, or any collector down) → 60% amber tint, pulses stay visible. `/state` reports `severity`.
- Verified on synthetic state. The live drill (`scripts/outage-drill.sh`) has to be run by the user: the agent isn't permitted to stop containers.

## gw-open has no working upstream key (affects ../machine)
A remote-tier call with a valid gw-open virtual key (`seele-stress-open`, eu-reason only, $1/30d, `.secrets/gw-open.key`) fails at OpenRouter: `401 No cookie auth credentials found`. The OpenRouter API key in machine's config is missing or invalid, so **every remote call fails for every client**. Nothing was spent. `spend_today` and `tok_remote` can't be tested until it's fixed.
Side note: these upstream 401s reach the client as 401, so `err_rate` (5xx and 429 only) doesn't show them. **Decided 2026-11-12: 401 and 403 now count as errors**, for both err_rate and the red pulses.

## Live drill (2026-11-13, `scripts/outage-drill.sh`, run by the user)
- egress-open stopped: `containers_bad` went 0 → 0.14 after about 4 s (1 of 7), which is **degraded**. Restored after about 6 s.
- edge stopped: Traefik collector `down` immediately, `gateway_down` = 1 after about 4 s (2 failed probes), which is **outage**. Recovered 2 s after start.
- `/state` showed `severity=None` during the drill. Two edits (scene → `state.severity`, endpoint field) had silently not applied, because the string replacements didn't match. Fixed and verified over HTTP, with regression tests in `tests/test_debug_http.py`.

## Visual check (2026-11-13, user watching)
- `preview` in `config.toml` (hot-reloaded) forces a severity without breaking anything. The sequence was ok → degraded → outage → ok, confirmed by `/state` and by the user's eyes: **amber and red both clearly visible**.
- Why amber wasn't noticed in the drill: (1) quiet hours (23:00–07:00) dim everything to 15%; (2) a 60% amber blend over idle blue gave mauve (105,61,70). It's now an 85% blend, giving (149,74,26). Previews ignore quiet hours.
- Decided 2026-11-13: real outage/degraded states **stay dimmed** during quiet hours (for now). Only `preview` bypasses them.

## Look sign-off (2026-11-13)
The palette and pulse parameters were moved into `config.toml` (`[palette]`, `[pulses]`, hot-reloaded). With `preview = "traffic"` (fake requests at 2/s), the user approved the current defaults as they are.
