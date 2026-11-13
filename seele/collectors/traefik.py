"""Traefik collector: tails the edge access log (JSON on stdout) for request events and rates."""
import asyncio
import json
import time
from collections import deque

CONTAINER = "research-stack-edge-1"
WINDOW = 10.0



ERROR_STATUSES = {401, 403, 429}


def is_error(status: int) -> bool:
    """5xx, rate limits, and auth failures. A 401 can be a bad client key or the provider rejecting
    gw-open's upstream credentials; both are failures worth seeing (user decision, 2026-11-12)."""
    return status >= 500 or status in ERROR_STATUSES

def parse_line(line: str | bytes) -> dict | None:
    """Parse one access-log line into {tier, status, ms}; None if not JSON or not a gateway request."""
    try:
        d = json.loads(line)
        path = d["RequestPath"]
        status = int(d["DownstreamStatus"])
        ms = float(d["Duration"]) / 1e6
    except (ValueError, KeyError, TypeError):
        return None
    if path.startswith("/open/"):
        tier = "remote"
    elif path.startswith("/v1/"):
        tier = "local"
    else:
        return None
    return {"tier": tier, "status": status, "ms": ms}


class Window:
    def __init__(self, span: float = WINDOW):
        self.span, self.q = span, deque()

    def add(self, t: float, tier: str, status: int) -> None:
        self.q.append((t, tier, status))

    def stats(self, now: float) -> tuple[float, float, float]:
        """Return (req_local, req_remote, err_rate), each 0..1."""
        while self.q and now - self.q[0][0] > self.span:
            self.q.popleft()
        n = {"local": 0, "remote": 0}
        err = 0
        for _, tier, st in self.q:
            n[tier] += 1
            err += is_error(st)
        total = len(self.q)
        return (min(1.0, n["local"] / self.span / 2), min(1.0, n["remote"] / self.span / 2),
                err / total if total else 0.0)


async def _tail(state, win: Window) -> None:
    p = await asyncio.create_subprocess_exec(
        "docker", "logs", "-f", "--since", "0s", CONTAINER,
        stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.DEVNULL)
    try:
        state.health["traefik"] = "ok"
        while (line := await p.stdout.readline()):
            r = parse_line(line)
            if r:
                win.add(time.monotonic(), r["tier"], r["status"])
                state.events.emit("request", **r)
    finally:
        if p.returncode is None:
            p.kill()
        await p.wait()


async def _push_loop(state, win: Window) -> None:
    while True:
        a, b, e = win.stats(time.monotonic())
        state.push("req_local", a, tau=1.0)
        state.push("req_remote", b, tau=1.0)
        state.push("err_rate", e, tau=1.0)
        await asyncio.sleep(0.5)


async def run(state, **opts) -> None:
    win = Window()
    asyncio.create_task(_push_loop(state, win))
    last_log = 0.0
    while True:
        try:
            await _tail(state, win)
            raise RuntimeError("docker logs exited")
        except asyncio.CancelledError:
            raise
        except Exception as e:
            state.health["traefik"] = "down"
            if time.monotonic() - last_log > 60:
                last_log = time.monotonic()
                print(f"traefik collector: {type(e).__name__}: {e}")
        await asyncio.sleep(3)
