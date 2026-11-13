"""Host and GPU load. Every value is normalised to 0..1."""
import asyncio
import logging

import psutil

log = logging.getLogger(__name__)


async def run(state, interval: float = 0.5) -> None:
    psutil.cpu_percent(None)  # prime the counter
    gpu = _Gpu()
    while True:
        await asyncio.sleep(interval)
        state.push("cpu", psutil.cpu_percent(None) / 100)
        state.push("mem", psutil.virtual_memory().percent / 100, tau=5)
        t = _cpu_temp()
        if t is not None:
            state.push("cpu_temp", (t - 35) / 55, tau=3)  # 35 °C → 0, 90 °C → 1
        gpu.sample(state)


_TEMP_FILE: str | None = None


def _find_cpu_temp_file() -> str | None:
    """Locate the CPU die sensor once. psutil.sensors_temperatures() reads *every* hwmon chip
    (ASUS EC, Super I/O, NZXT USB) on each call: about 16 ms of blocking kernel work, and contention with OpenRGB."""
    import glob

    for d in glob.glob("/sys/class/hwmon/hwmon*"):
        try:
            if open(f"{d}/name").read().strip() in ("k10temp", "zenpower", "coretemp"):
                return f"{d}/temp1_input"
        except OSError:
            continue
    return None


def _cpu_temp() -> float | None:
    global _TEMP_FILE
    if _TEMP_FILE is None:
        _TEMP_FILE = _find_cpu_temp_file() or ""
    if not _TEMP_FILE:
        return None
    try:
        with open(_TEMP_FILE) as f:
            return int(f.read()) / 1000
    except OSError:
        return None


class _Gpu:
    def __init__(self):
        try:
            import pynvml

            pynvml.nvmlInit()
            self.nv, self.h = pynvml, pynvml.nvmlDeviceGetHandleByIndex(0)
            self.plimit = pynvml.nvmlDeviceGetEnforcedPowerLimit(self.h)
        except Exception as e:  # no NVIDIA GPU or driver
            log.warning("NVML unavailable: %s", e)
            self.nv = None

    def sample(self, state) -> None:
        if not self.nv:
            return
        nv, h = self.nv, self.h
        state.push("gpu", nv.nvmlDeviceGetUtilizationRates(h).gpu / 100)
        m = nv.nvmlDeviceGetMemoryInfo(h)
        state.push("vram", m.used / m.total, tau=3)
        state.push("gpu_power", nv.nvmlDeviceGetPowerUsage(h) / self.plimit)
        state.push("gpu_temp", (nv.nvmlDeviceGetTemperature(h, 0) - 35) / 50, tau=3)
