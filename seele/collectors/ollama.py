"""Local model activity from Ollama: loaded-model VRAM share and llama-server slot busyness."""
import asyncio
import logging
import re
import time

import httpx

log = logging.getLogger(__name__)
PS_URL = "http://127.0.0.1:11434/api/ps"
DEFAULT_VRAM = 8 * 1024**3


def parse_ports(pgrep_out: str) -> list[int]:
    """Ports of real runners only: the executable itself must be llama-server (not a shell mentioning it)."""
    ports = set()
    for line in pgrep_out.splitlines():
        parts = line.split(maxsplit=2)  # pid, exe, args
        if len(parts) == 3 and parts[1].rsplit("/", 1)[-1] == "llama-server":
            ports.update(int(m) for m in re.findall(r"--port[ =](\d+)", parts[2]))
    return sorted(ports)


def vram_fraction(ps: dict, total: int) -> float:
    used = sum(int(m.get("size_vram", 0) or 0) for m in ps.get("models", []) or [])
    return max(0.0, min(1.0, used / total)) if total > 0 else 0.0


def any_busy(slots) -> bool:
    return isinstance(slots, list) and any(isinstance(s, dict) and s.get("is_processing") for s in slots)


def _gpu_total() -> int:
    try:
        import pynvml

        pynvml.nvmlInit()
        return int(pynvml.nvmlDeviceGetMemoryInfo(pynvml.nvmlDeviceGetHandleByIndex(0)).total)
    except Exception:
        return DEFAULT_VRAM


async def _pgrep() -> list[int]:
    p = await asyncio.create_subprocess_exec(
        "pgrep", "-af", "llama-server", stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.DEVNULL
    )
    out, _ = await p.communicate()
    return parse_ports(out.decode(errors="replace"))


async def run(state, **opts) -> None:
    total = await asyncio.to_thread(_gpu_total)
    ps_ok = slots_ok = True
    last_log = 0.0

    def warn(msg):
        nonlocal last_log
        if time.monotonic() - last_log > 60:
            last_log = time.monotonic()
            log.warning(msg)

    async with httpx.AsyncClient(timeout=1.5) as http:

        async def ps_loop():
            nonlocal ps_ok
            while True:
                try:
                    r = await http.get(PS_URL)
                    r.raise_for_status()
                    state.push("vram_models", vram_fraction(r.json(), total), tau=2)
                    ps_ok = True
                except Exception as e:
                    ps_ok = False
                    state.push("vram_models", 0.0, tau=2)
                    warn(f"ollama /api/ps: {e!r}")
                _set_health()
                await asyncio.sleep(2)

        def _set_health():
            state.health["ollama"] = "ok" if ps_ok and slots_ok else ("degraded" if ps_ok else "down")

        async def slots_loop():
            nonlocal slots_ok
            ports: list[int] = []
            found = 0.0
            fails = 0
            while True:
                try:
                    if not ports or time.monotonic() - found > 10:
                        ports, found = await _pgrep(), time.monotonic()
                    busy, err = False, False
                    for port in ports:
                        try:
                            r = await http.get(f"http://127.0.0.1:{port}/slots")
                            r.raise_for_status()
                            busy = busy or any_busy(r.json())
                        except httpx.TimeoutException:
                            busy = True  # a runner too busy to answer /slots is generating
                        except Exception:
                            err = True
                    if err:
                        found = 0.0  # re-discover next round
                    # A freshly spawned runner (model just loaded) refuses connections briefly,
                    # so only degrade after 3 failed rounds in a row (~1.5 s).
                    fails = fails + 1 if err else 0
                    slots_ok = fails < 3
                    state.push("local_busy", 1.0 if busy else 0.0, tau=0.3)
                except Exception as e:
                    slots_ok = False
                    warn(f"ollama slots: {e!r}")
                _set_health()
                await asyncio.sleep(0.5)

        try:
            await asyncio.gather(ps_loop(), slots_loop())
        except Exception as e:  # never raise
            state.health["ollama"] = "down"
            warn(f"ollama collector crashed: {e!r}")
            await asyncio.sleep(5)
