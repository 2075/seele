RGB = tuple[int, int, int]

# Load ramp: calm blue → teal → amber → red
LOAD_RAMP: list[tuple[float, RGB]] = [
    (0.00, (0, 40, 255)),
    (0.35, (0, 200, 160)),
    (0.70, (255, 150, 0)),
    (1.00, (255, 0, 0)),
]


def lerp(a: RGB, b: RGB, t: float) -> RGB:
    return tuple(round(x + (y - x) * t) for x, y in zip(a, b))  # type: ignore[return-value]


def ramp(x: float, stops=LOAD_RAMP) -> RGB:
    x = max(0.0, min(1.0, x))
    for (p0, c0), (p1, c1) in zip(stops, stops[1:]):
        if x <= p1:
            return lerp(c0, c1, (x - p0) / (p1 - p0))
    return stops[-1][1]


def scale(c: RGB, k: float) -> RGB:
    return tuple(max(0, min(255, round(v * k))) for v in c)  # type: ignore[return-value]
