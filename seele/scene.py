"""Turn machine state into one frame: {zone index: [RGB per LED]}.

Layers, bottom to top:
  1. base     load colour (ramp) per role, with a gentle breathing effect
  2. tiers    fans tinted by the local/remote traffic mix; GPU breathes green while a local model generates
  3. pulses   one travelling pulse per request on the strip (teal = local, violet = remote, red = error)
  4. spend    Kraken logo fills violet with today's remote spend
  5. alerts   a critical event flashes red everywhere for 1.5 s
  6. status   persistent, overrides the rest:
              outage   (gateway unreachable on both tiers, or >= half the core services missing/bad)
                       → every zone breathes red slowly until it recovers
              degraded (one tier down, some services bad, or a collector down)
                       → amber tint on every zone; request pulses stay visible on the strip
"""
import math
import time
from dataclasses import dataclass

from .collectors.traefik import is_error
from .colors import lerp, ramp, scale

ALERT_SECONDS = 1.5


@dataclass
class Pulse:
    born: float
    color: tuple
    width: float


def severity(state) -> str:
    """'outage' | 'degraded' | 'ok', from gateway reachability, container health and collector health."""
    gw, bad = state.get("gateway_down"), state.get("containers_bad")
    if gw > 0.75 or bad >= 0.5:
        return "outage"
    collectors_bad = any(v != "ok" for v in state.health.values())
    if gw > 0.25 or bad > 0.05 or collectors_bad:
        return "degraded"
    return "ok"


def _add(a, b):
    return tuple(min(255, x + y) for x, y in zip(a, b))


class Scene:
    def __init__(self, bindings, sizes, brightness: float = 0.8, quiet: str | None = None, quiet_level: float = 0.15,
                 preview: str | None = None, look=None):
        self.b, self.sizes, self.brightness = bindings, sizes, brightness
        self.pulses: list[Pulse] = []
        self.alert_until = 0.0
        self.quiet, self.quiet_level = _parse_quiet(quiet), quiet_level
        self.preview = preview  # force "ok" | "degraded" | "outage" | "traffic" to look at the lighting
        from .config import Look

        self.L = look or Look()
        self._next_demo = 0.0

    def _ingest(self, state, now):
        for e in state.events.drain():
            if e["kind"] == "request":
                bad = is_error(e.get("status", 200))
                L = self.L
                color = L.error if bad else (L.remote if e.get("tier") == "remote" else L.local)
                slow = min(1.0, e.get("ms", 0) / L.pulse_slow_ms)  # longer requests → wider pulse
                width = L.pulse_width + (L.pulse_width_max - L.pulse_width) * slow
                self.pulses.append(Pulse(now, color, width))
            elif e["kind"] == "alert" and e.get("level") == "crit":
                self.alert_until = now + ALERT_SECONDS
        self.pulses = [p for p in self.pulses if now - p.born < self.L.pulse_seconds][-self.L.pulse_max:]

    def _gain(self) -> float:
        if not self.quiet or self.preview:  # previews ignore quiet hours, so you can actually see them
            return self.brightness
        h = time.localtime()
        m = h.tm_hour * 60 + h.tm_min
        a, b = self.quiet
        inside = a <= m < b if a < b else (m >= a or m < b)
        return self.brightness * (self.quiet_level if inside else 1.0)

    def render(self, state, t: float, now: float | None = None):
        """t: seconds since start (animation phase). now: monotonic clock for pulses/alerts (injectable for replay)."""
        now = time.monotonic() if now is None else now
        self._ingest(state, now)
        g = state.get
        cpu, gpu = g("cpu"), g("gpu")
        ambient = max(cpu, gpu, 0.6 * g("mem"))
        loc = g("req_local") + g("tok_local")
        rem = g("req_remote") + g("tok_remote")
        mix = rem / (loc + rem) if loc + rem > 0.02 else None  # 0 = all local, 1 = all remote
        gain = self._gain()
        alert = now < self.alert_until
        flash = alert and int(now * 6) % 2 == 0
        if self.preview == "traffic" and now >= self._next_demo:
            import random

            self._next_demo = now + random.expovariate(self.L.demo_rps)
            r = random.random()
            tier = "remote" if r < 0.33 else "local"
            status = 500 if r > 0.9 else 200
            state.events.emit("request", tier=tier, status=status, ms=random.uniform(100, 8000))
        sev = self.preview if self.preview in ("ok", "degraded", "outage") else severity(state)
        self.severity = state.severity = sev
        outage_k = 0.2 + 0.8 * (0.5 + 0.5 * math.sin(t * 2 * math.pi / 3))  # 3 s red breathing

        frame = {}
        for i, (b, n) in enumerate(zip(self.b, self.sizes)):
            if n == 0:
                continue
            level = {"cpu": max(cpu, 0.8 * g("cpu_temp")), "gpu": max(gpu, 0.8 * g("gpu_temp"))}.get(b.role, ambient)
            L = self.L
            base = ramp(level, L.ramp)
            k = 1 - L.breathe_depth + L.breathe_depth * math.sin(t * (L.breathe_speed + 3 * level) + i)

            if b.role == "fans" and mix is not None:
                base = lerp(base, lerp(L.local, L.remote, mix), min(1.0, 0.4 + loc + rem))
            if b.role == "gpu" and g("local_busy") > 0.5:
                base, k = lerp(base, L.local, 0.7), 0.6 + 0.4 * math.sin(t * 5)
            if b.role == "ambient" and g("spend_today") > 0:
                base = lerp(base, L.remote, min(1.0, g("spend_today")))
            if sev == "degraded":
                base = lerp(base, L.amber, 0.85)  # 0.6 over idle blue read as mauve, not amber

            leds = [scale(base, k * (L.strip_base if b.role == "strip" else 1.0))] * n
            if b.role == "strip":
                for p in self.pulses:
                    pos = (now - p.born) / L.pulse_seconds * (n + 2 * p.width) - p.width
                    leds = [_add(c, scale(p.color, max(0.0, 1 - abs(j - pos) / p.width))) for j, c in enumerate(leds)]
            if sev == "outage":
                leds = [scale(L.error, outage_k)] * n
            elif flash:
                leds = [L.error] * n
            frame[i] = [scale(c, gain) for c in leds]
        return frame


def _parse_quiet(spec):
    if not spec:
        return None
    a, b = spec.split("-")
    to_m = lambda s: int(s.split(":")[0]) * 60 + int(s.split(":")[1])
    return to_m(a), to_m(b)
