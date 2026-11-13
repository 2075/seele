//! Colour helpers. Mirrors seele/colors.py (round-half-to-even, like Python's round()).
pub type Rgb = (u8, u8, u8);

fn r(x: f64) -> f64 {
    x.round_ties_even()
}

pub fn lerp(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let f = |x: u8, y: u8| {
        let (x, y) = (x as f64, y as f64);
        r(x + (y - x) * t) as i64 as u8
    };
    (f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

pub fn ramp(x: f64, stops: &[(f64, Rgb)]) -> Rgb {
    let x = x.clamp(0.0, 1.0);
    for w in stops.windows(2) {
        let ((p0, c0), (p1, c1)) = (w[0], w[1]);
        if x <= p1 {
            return lerp(c0, c1, (x - p0) / (p1 - p0));
        }
    }
    stops.last().map(|s| s.1).unwrap_or((0, 0, 0))
}

pub fn scale(c: Rgb, k: f64) -> Rgb {
    let f = |v: u8| r(v as f64 * k).clamp(0.0, 255.0) as u8;
    (f(c.0), f(c.1), f(c.2))
}
