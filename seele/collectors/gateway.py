"""End-to-end gateway reachability: Traefik → guard → LiteLLM liveliness, for both tiers.

health["gateway"] = "ok" (both tiers answer), "degraded" (one tier down), "down" (neither).
Signal `gateway_down`: 0 ok, 0.5 degraded, 1 down. Needs 2 consecutive failures, so one slow reply isn't an outage.
"""
import asyncio

import httpx

PROBES = {"local": "/health/liveliness", "remote": "/open/health/liveliness"}


def classify(results: dict[str, bool]) -> tuple[str, float]:
    up = sum(results.values())
    if up == len(results):
        return "ok", 0.0
    return ("degraded", 0.5) if up else ("down", 1.0)


async def run(state, base: str = "http://127.0.0.1:4000", interval: float = 2.0) -> None:
    fails = {k: 0 for k in PROBES}
    async with httpx.AsyncClient(base_url=base, timeout=2.0) as c:
        while True:
            for tier, path in PROBES.items():
                try:
                    r = await c.get(path)
                    fails[tier] = 0 if r.status_code < 500 and r.status_code != 404 else fails[tier] + 1
                except Exception:
                    fails[tier] += 1
            health, level = classify({t: f < 2 for t, f in fails.items()})
            if health != state.health.get("gateway") and health != "ok":
                state.events.emit("alert", source="gateway", level="crit" if health == "down" else "warn", msg=health)
            state.health["gateway"] = health
            state.push("gateway_down", level, tau=0.3)
            await asyncio.sleep(interval)
