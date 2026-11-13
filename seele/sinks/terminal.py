"""Dry-run sink that draws each zone as truecolor blocks in the terminal."""
import sys


class TerminalSink:
    def __init__(self, cfg, sizes=None):
        self.labels = [f"{b.role:>7} {b.device}/{b.zone}"[:42].ljust(42) for b in cfg.zones]
        self.sizes = sizes or [8] * len(cfg.zones)
        self._drawn = False

    def show(self, frame, status: str = "") -> None:
        out = []
        if self._drawn:
            out.append(f"\x1b[{len(self.labels) + 1}F")
        for i, label in enumerate(self.labels):
            blocks = "".join(f"\x1b[38;2;{r};{g};{b}m██" for r, g, b in frame.get(i, []))
            out.append(f"{label} {blocks}\x1b[0m\x1b[K\n")
        out.append(status + "\x1b[K\n")
        sys.stdout.write("".join(out))
        sys.stdout.flush()
        self._drawn = True

    def close(self) -> None:
        pass
