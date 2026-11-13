"""seele: show machine load through OpenRGB lighting.

  seele run [--dry-run]   start the monitor (dry run = terminal preview only)
  seele identify          light each configured zone in turn (to map strips vs fans)
"""
import argparse
import asyncio
import importlib
import os
import logging
import time

from . import config as cfgmod
from .collectors import host
from .scene import Scene
from .state import State

# Real LED counts, from docs/findings/2026-11-12-openrgb-devices.md (used by the dry run)
KNOWN_SIZES = {"Unknown": 4, "Aura Mainboard": 8, "Hue 2 Channel 3": 1}


def _dry_sizes(cfg):
    sizes = []
    for b in cfg.zones:
        if "Smart Device" in b.device:
            sizes.append(16 if b.zone.endswith("1") else 10)
        elif "Kraken" in b.device and b.zone.endswith("2"):
            sizes.append(8)
        else:
            sizes.append(KNOWN_SIZES.get(b.zone, 8))
    return sizes


def _sink(cfg, dry: bool):
    if dry:
        from .sinks.terminal import TerminalSink

        return TerminalSink(cfg, _dry_sizes(cfg))
    from .sinks.openrgb_sink import OpenRGBSink

    return OpenRGBSink(cfg)


async def _run(cfg, dry: bool, path: str = "config.toml") -> None:
    state = State()
    from .debug_http import serve

    tasks = [asyncio.create_task(host.run(state)), asyncio.create_task(serve(state))]
    for name in cfg.collectors:
        try:
            mod = importlib.import_module(f".collectors.{name}", __package__)
        except ImportError as e:
            logging.warning("collector %s unavailable: %s", name, e)
            continue
        tasks.append(asyncio.create_task(mod.run(state, **cfg.collector_opts.get(name, {}))))

    def build(c):
        sink = _sink(c, dry)
        return sink, Scene(c.zones, sink.sizes, c.brightness, c.quiet, preview=c.preview, look=c.look)

    sink, scene = build(cfg)
    mtime, last_check = _mtime(path), 0.0
    t0, dt = time.monotonic(), 1 / cfg.fps
    try:
        while True:
            t = time.monotonic() - t0
            if t - last_check > 2:  # hot-reload zones, roles, brightness and quiet hours (collectors need a restart)
                last_check, m = t, _mtime(path)
                if m != mtime:
                    mtime = m
                    try:
                        new = cfgmod.load(path)
                        sink.close()
                        cfg, (sink, scene), dt = new, build(new), 1 / new.fps
                        logging.info("config reloaded from %s", path)
                    except Exception as e:
                        logging.warning("config reload failed, keeping old config: %s", e)
            frame = scene.render(state, t)
            if dry:
                g = state.get
                sink.show(frame, f"cpu {g('cpu'):.2f}  gpu {g('gpu'):.2f}  vram {g('vram'):.2f}  "
                                 f"loc {g('req_local'):.2f}/{g('tok_local'):.2f}  rem {g('req_remote'):.2f}/{g('tok_remote'):.2f}  "
                                 f"busy {g('local_busy'):.0f}  spend {g('spend_today'):.2f}  bad {g('containers_bad'):.2f}  "
                                 f"health {','.join(f'{k}:{v}' for k, v in state.health.items())}")
            else:
                sink.show(frame)
            await asyncio.sleep(dt)
    finally:
        for task in tasks:
            task.cancel()
        sink.close()


def _mtime(path) -> float:
    try:
        return os.stat(path).st_mtime
    except OSError:
        return 0.0


def _identify(cfg, dry: bool, hold: float) -> None:
    sink = _sink(cfg, dry)
    try:
        for i, b in enumerate(cfg.zones):
            frame = {j: [(0, 0, 0)] * n for j, n in enumerate(sink.sizes)}
            frame[i] = [(255, 255, 255)] * sink.sizes[i]
            print(f"[{i}] {b.device} / {b.zone}  (role={b.role}, {sink.sizes[i]} LEDs)") if not dry else None
            sink.show(frame)
            time.sleep(hold)
    finally:
        sink.close()


def main() -> None:
    p = argparse.ArgumentParser(prog="seele", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("command", choices=["run", "identify", "replay"])
    p.add_argument("fixture", nargs="?", help="replay: fixture JSON path")
    p.add_argument("-c", "--config", default="config.toml")
    p.add_argument("--dry-run", action="store_true", help="render to the terminal instead of OpenRGB")
    p.add_argument("--hold", type=float, default=4.0, help="identify: seconds per zone")
    p.add_argument("-v", "--verbose", action="store_true")
    a = p.parse_args()
    logging.basicConfig(level=logging.DEBUG if a.verbose else logging.INFO, format="%(levelname)s %(name)s: %(message)s")
    logging.getLogger("httpx").setLevel(logging.WARNING)
    cfg = cfgmod.load(a.config)
    try:
        if a.command == "replay":
            from .replay import main as replay_main

            replay_main(a.fixture, a.config)
        elif a.command == "run":
            asyncio.run(_run(cfg, a.dry_run, a.config))
        else:
            _identify(cfg, a.dry_run, a.hold)
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
