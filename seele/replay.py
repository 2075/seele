"""Deterministic scene replay: the parity spec for the Rust port.

Input (JSON): {"sizes": [...], "steps": [{"t": float, "signals": {name: value}, "health": {name: str},
                                          "events": [{"kind": ..., ...}]}, ...]}
Signals are set directly (no EMA), quiet hours are off, and `now` = t. Brightness 0.8, preview off.
Output (JSON): [{"t": t, "severity": str, "frame": {"<zone index>": [[r, g, b], ...]}}, ...]
"""
import json

from .config import load
from .scene import Scene
from .state import Signal, State


def replay(fixture: dict, config_path: str = "config.toml") -> list[dict]:
    cfg = load(config_path)
    scene = Scene(cfg.zones, fixture["sizes"], 0.8, None, preview=None, look=cfg.look)
    state, out = State(), []
    for step in fixture["steps"]:
        t = step["t"]
        for k, v in step.get("signals", {}).items():
            state.signals.setdefault(k, Signal()).value = max(0.0, min(1.0, v))
        state.health.update(step.get("health", {}))
        for e in step.get("events", []):
            state.events.q.append({"t": t, **e})
        frame = scene.render(state, t, now=t)
        out.append({"t": t, "severity": state.severity, "frame": {str(i): [list(c) for c in v] for i, v in frame.items()}})
    return out


def main(fixture_path: str, config_path: str = "config.toml") -> None:
    print(json.dumps(replay(json.load(open(fixture_path)), config_path)))
