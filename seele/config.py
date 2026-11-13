import tomllib
from dataclasses import dataclass, field
from pathlib import Path


@dataclass
class ZoneBinding:
    device: str
    zone: str
    role: str


def _rgb(v) -> tuple[int, int, int]:
    """Accept "#rrggbb" or [r, g, b]."""
    if isinstance(v, str):
        v = v.lstrip("#")
        return tuple(int(v[i:i + 2], 16) for i in (0, 2, 4))  # type: ignore[return-value]
    return tuple(int(x) for x in v)  # type: ignore[return-value]


@dataclass
class Look:
    """Everything visual that is tuned by eye. Defaults are the current design."""
    ramp: list = field(default_factory=lambda: [(0.0, (0, 40, 255)), (0.35, (0, 200, 160)), (0.7, (255, 150, 0)), (1.0, (255, 0, 0))])
    local: tuple = (0, 255, 150)
    remote: tuple = (170, 0, 255)
    error: tuple = (255, 0, 0)
    amber: tuple = (255, 120, 0)
    pulse_seconds: float = 0.9      # time for one pulse to cross the strip
    pulse_width: float = 1.0        # LEDs, for an instant request
    pulse_width_max: float = 4.0    # LEDs, for a request taking >= pulse_slow_ms
    pulse_slow_ms: float = 6000
    pulse_max: int = 24             # pulses alive at once
    strip_base: float = 0.55        # strip background brightness under the pulses
    breathe_depth: float = 0.25     # 0 = steady, 0.5 = deep breathing
    breathe_speed: float = 0.6      # rad/s at idle; adds 3 rad/s at full load
    demo_rps: float = 2.0           # preview = "traffic": fake requests per second


def _look(raw: dict) -> Look:
    pal, pul = raw.get("palette", {}), raw.get("pulses", {})
    lk = Look()
    if "ramp" in pal:
        lk.ramp = [(float(p), _rgb(c)) for p, c in pal["ramp"]]
    for k in ("local", "remote", "error", "amber"):
        if k in pal:
            setattr(lk, k, _rgb(pal[k]))
    for k, v in pul.items():
        if hasattr(lk, k):
            setattr(lk, k, type(getattr(lk, k))(v))
    return lk


@dataclass
class Config:
    host: str = "127.0.0.1"
    port: int = 6742
    fps: int = 20
    brightness: float = 0.8
    zones: list[ZoneBinding] = field(default_factory=list)
    quiet: str | None = None
    preview: str | None = None
    look: Look = field(default_factory=Look)
    collectors: list[str] = field(default_factory=list)
    collector_opts: dict = field(default_factory=dict)


def load(path: str | Path) -> Config:
    raw = tomllib.loads(Path(path).read_text())
    o, r = raw.get("openrgb", {}), raw.get("render", {})
    return Config(
        host=o.get("host", "127.0.0.1"),
        port=o.get("port", 6742),
        fps=r.get("fps", 20),
        brightness=r.get("brightness", 0.8),
        zones=[ZoneBinding(**z) for z in raw.get("zone", [])],
        quiet=r.get("quiet"),
        preview=r.get("preview") or None,
        look=_look(raw),
        collectors=raw.get("collectors", {}).get("enabled", []),
        collector_opts={k: v for k, v in raw.get("collectors", {}).items() if isinstance(v, dict)},
    )
