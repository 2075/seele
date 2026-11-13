"""Send frames to an `openrgb --server` instance through the SDK."""
import logging
import time

from openrgb import OpenRGBClient
from openrgb.utils import RGBColor

log = logging.getLogger(__name__)


class OpenRGBSink:
    def __init__(self, cfg):
        self.client = OpenRGBClient(cfg.host, cfg.port, name="seele")
        self._wait_for_devices(cfg)
        self.zones, self.sizes, self._last = [], [], {}
        devices = set()
        for b in cfg.zones:
            dev = next((d for d in self.client.devices if b.device.lower() in d.name.lower()), None)
            zone = dev and next((z for z in dev.zones if z.name == b.zone), None)
            if zone is None:
                log.warning("not found: %s / %s; skipping", b.device, b.zone)
                self.zones.append(None)
                self.sizes.append(0)
                continue
            if dev.id not in devices:
                dev.set_mode("direct")
                devices.add(dev.id)
            self.zones.append(zone)
            self.sizes.append(len(zone.leds))

    def _wait_for_devices(self, cfg, timeout: float = 30.0) -> None:
        """The server detects devices asynchronously at startup (USB comes after SMBus), so poll."""
        wanted = {b.device.lower() for b in cfg.zones}
        deadline = time.monotonic() + timeout
        while True:
            names = [d.name.lower() for d in self.client.devices]
            missing = [w for w in wanted if not any(w in n for n in names)]
            if not missing or time.monotonic() > deadline:
                if missing:
                    log.warning("devices still missing after %.0fs: %s", timeout, missing)
                return
            time.sleep(1)
            self.client.update()

    def show(self, frame) -> None:
        for i, colors in frame.items():
            z = self.zones[i]
            if z is None or self._last.get(i) == colors:
                continue  # unchanged, so don't spend SDK packets on it
            z.set_colors([RGBColor(*c) for c in colors], fast=True)
            self._last[i] = colors

    def close(self) -> None:
        self.client.disconnect()
