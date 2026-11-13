"""Docker collector: container health, container events and egress throughput via the Engine API."""
import asyncio
import json
import math
import time

import httpx

SOCK = "/var/run/docker.sock"
PROJECT = "research-stack"
LABEL = f"com.docker.compose.project={PROJECT}"
EGRESS = {"research-stack-egress-local-1": "egress_local", "research-stack-egress-open-1": "egress_remote"}
WATCH = {"health_status", "die", "start", "oom"}
CRIT = {"unhealthy", "die", "oom"}


def norm_bytes_rate(bps: float) -> float:
    """1 KB/s -> 0, 10 MB/s -> 1, log scale."""
    if bps <= 1000:
        return 0.0
    return min(1.0, math.log10(bps / 1000) / 4)


def net_bytes(stats: dict) -> int:
    return sum(n.get("rx_bytes", 0) + n.get("tx_bytes", 0) for n in (stats.get("networks") or {}).values())


# Core research-stack services. Optional profiles (qdrant, open-webui) are not expected.
EXPECTED = ("edge", "guard", "gw-local", "gw-open", "litellm-db", "egress-local", "egress-open")


def bad_share(containers: list[dict], expected=EXPECTED) -> float:
    """Share of expected services that are missing, not running, or unhealthy.
    Missing counts as bad: `docker compose down` *removes* containers, so an empty list means an outage."""
    seen = {}
    for c in containers:
        svc = (c.get("Labels") or {}).get("com.docker.compose.service")
        ok = c.get("State") == "running" and "unhealthy" not in c.get("Status", "")
        if svc:
            seen[svc] = seen.get(svc, False) or ok
    names = set(expected) | set(seen)
    if not names:
        return 0.0
    return sum(1 for n in names if not seen.get(n, False)) / len(names)


def parse_event(ev: dict) -> tuple[str, str] | None:
    """Return (container name, state) for an event worth reporting, else None."""
    action = ev.get("Action") or ev.get("status") or ""
    base, _, detail = action.partition(":")
    base = base.strip()
    if base not in WATCH:
        return None
    state = detail.strip() if base == "health_status" else base
    if not state:
        return None
    name = (ev.get("Actor") or {}).get("Attributes", {}).get("name", "?")
    return name, state


def _client() -> httpx.AsyncClient:
    return httpx.AsyncClient(transport=httpx.AsyncHTTPTransport(uds=SOCK), base_url="http://docker", timeout=None)


async def _poll_containers(state) -> None:
    async with _client() as c:
        while True:
            try:
                r = await c.get("/containers/json", params={"all": "true", "filters": json.dumps({"label": [LABEL]})}, timeout=5)
                r.raise_for_status()
                state.push("containers_bad", bad_share(r.json()), tau=1.0)
                state.health["docker"] = "ok"
            except Exception:
                state.health["docker"] = "down"
                raise
            await asyncio.sleep(5)


async def _events(state) -> None:
    flt = json.dumps({"type": ["container"], "label": [LABEL]})
    async with _client() as c:
        async with c.stream("GET", "/events", params={"filters": flt, "since": str(int(time.time()))}) as r:
            r.raise_for_status()
            async for line in r.aiter_lines():
                try:
                    p = parse_event(json.loads(line))
                except ValueError:
                    continue
                if not p:
                    continue
                name, st = p
                state.events.emit("container", name=name, state=st)
                if st in CRIT:
                    state.events.emit("alert", source="docker", level="crit", msg=f"{name}: {st}")


async def _stats(state, name: str, signal: str) -> None:
    prev = None
    async with _client() as c:
        async with c.stream("GET", f"/containers/{name}/stats", params={"stream": "true"}) as r:
            if r.status_code != 200:
                raise RuntimeError(f"stats {name}: {r.status_code}")
            async for line in r.aiter_lines():
                try:
                    now, total = time.monotonic(), net_bytes(json.loads(line))
                except ValueError:
                    continue
                if prev and now > prev[0]:
                    state.push(signal, norm_bytes_rate(max(0, total - prev[1]) / (now - prev[0])), tau=1.0)
                prev = (now, total)


async def _forever(state, fn, *args) -> None:
    last_log, delay = 0.0, 2.0
    while True:
        try:
            await fn(state, *args)
        except asyncio.CancelledError:
            raise
        except Exception as e:
            state.health["docker"] = "down"
            if time.monotonic() - last_log > 60:
                last_log = time.monotonic()
                print(f"docker collector {fn.__name__}: {type(e).__name__}: {e}")
            for sig in args[1:]:
                state.push(sig, 0.0)
        await asyncio.sleep(delay)


async def run(state, **opts) -> None:
    tasks = [_forever(state, _poll_containers), _forever(state, _events)]
    tasks += [_forever(state, _stats, n, s) for n, s in EGRESS.items()]
    await asyncio.gather(*tasks)
