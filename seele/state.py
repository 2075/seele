"""Shared machine state: collectors write raw values, and the scene reads smoothed ones."""
import time


class Signal:
    """Exponential moving average with a time constant, independent of the update rate."""

    def __init__(self, tau: float = 1.5):
        self.tau, self.value, self._t = tau, 0.0, None

    def push(self, x: float) -> None:
        now = time.monotonic()
        if self._t is None:
            self.value = x
        else:
            a = 1 - pow(2.718281828, -(now - self._t) / self.tau)
            self.value += a * (x - self.value)
        self._t = now


class State:
    def __init__(self):
        self.signals: dict[str, Signal] = {}
        self.events = Events()
        self.health: dict[str, str] = {}  # source -> "ok" | "degraded" | "down" (for alerts)

    def push(self, name: str, x: float, tau: float = 1.5) -> None:
        self.signals.setdefault(name, Signal(tau)).push(max(0.0, min(1.0, x)))

    def get(self, name: str, default: float = 0.0) -> float:
        s = self.signals.get(name)
        return s.value if s else default


class Events:
    """Discrete happenings (e.g. one HTTP request) that the scene renders as pulses or flashes."""

    def __init__(self, maxlen: int = 256):
        from collections import deque

        self.q: deque[dict] = deque(maxlen=maxlen)

    def emit(self, kind: str, **attrs) -> None:
        self.q.append({"t": attrs.pop("t", None) or time.monotonic(), "kind": kind, **attrs})

    def drain(self) -> list[dict]:
        out = list(self.q)
        self.q.clear()
        return out
