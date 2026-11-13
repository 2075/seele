//! Turn machine state into one frame. Mirrors seele/scene.py (see the layer notes there).
use chrono::Timelike;
use rand::RngExt;

use crate::colors::{Rgb, lerp, ramp, scale};
use crate::config::{Look, ZoneBinding};
use crate::sinks::Frame;
use crate::state::{EventKind, Health, Level, Sig, State, Tier};

const ALERT_SECONDS: f64 = 1.5;

struct Pulse {
    born: f64,
    color: Rgb,
    width: f64,
}

/// 5xx, rate limits and auth failures (same as collectors.traefik.is_error in Python).
pub fn is_error(status: u16) -> bool {
    status >= 500 || matches!(status, 401 | 403 | 429)
}

pub fn severity(state: &State) -> &'static str {
    let (gw, bad) = (state.get(Sig::GatewayDown), state.get(Sig::ContainersBad));
    if gw > 0.75 || bad >= 0.5 {
        return "outage";
    }
    let collectors_bad = state.0.lock().health.values().any(|h| *h != Health::Ok);
    if gw > 0.25 || bad > 0.05 || collectors_bad {
        return "degraded";
    }
    "ok"
}

fn add(a: Rgb, b: Rgb) -> Rgb {
    (a.0.saturating_add(b.0), a.1.saturating_add(b.1), a.2.saturating_add(b.2))
}

/// "HH:MM-HH:MM" -> minutes since midnight.
fn parse_quiet(spec: Option<&str>) -> Option<(u32, u32)> {
    let (a, b) = spec?.split_once('-')?;
    let m = |s: &str| -> Option<u32> {
        let (h, mi) = s.trim().split_once(':')?;
        Some(h.parse::<u32>().ok()? * 60 + mi.parse::<u32>().ok()?)
    };
    Some((m(a)?, m(b)?))
}

pub struct Scene {
    bindings: Vec<ZoneBinding>,
    sizes: Vec<usize>,
    brightness: f64,
    quiet: Option<(u32, u32)>,
    quiet_level: f64,
    preview: Option<String>,
    look: Look,
    pulses: Vec<Pulse>,
    alert_until: f64,
    next_demo: f64,
    pub severity: &'static str,
}

impl Scene {
    pub fn new(bindings: Vec<ZoneBinding>, sizes: Vec<usize>, brightness: f64, quiet: Option<&str>, preview: Option<String>, look: Look) -> Self {
        Scene {
            bindings,
            sizes,
            brightness,
            quiet: parse_quiet(quiet),
            quiet_level: 0.15,
            preview,
            look,
            pulses: vec![],
            alert_until: 0.0,
            next_demo: 0.0,
            severity: "ok",
        }
    }

    fn ingest(&mut self, state: &State, now: f64) {
        for e in state.drain() {
            match e.kind {
                EventKind::Request { tier, status, ms } => {
                    let l = &self.look;
                    let color = if is_error(status) {
                        l.error
                    } else if tier == Tier::Remote {
                        l.remote
                    } else {
                        l.local
                    };
                    let slow = (ms / l.pulse_slow_ms).min(1.0);
                    let width = l.pulse_width + (l.pulse_width_max - l.pulse_width) * slow;
                    self.pulses.push(Pulse { born: now, color, width });
                }
                EventKind::Alert { level: Level::Crit, .. } => self.alert_until = now + ALERT_SECONDS,
                _ => {}
            }
        }
        let secs = self.look.pulse_seconds;
        self.pulses.retain(|p| now - p.born < secs);
        let max = self.look.pulse_max;
        if max > 0 && self.pulses.len() > max {
            self.pulses.drain(..self.pulses.len() - max);
        }
    }

    fn gain(&self) -> f64 {
        let Some((a, b)) = self.quiet else { return self.brightness };
        if self.preview.is_some() {
            return self.brightness;
        }
        let now = chrono::Local::now();
        let m = now.hour() * 60 + now.minute();
        let inside = if a < b { a <= m && m < b } else { m >= a || m < b };
        self.brightness * if inside { self.quiet_level } else { 1.0 }
    }

    /// `t`: animation phase seconds. `now`: clock for pulses/alerts (defaults to the state clock).
    pub fn render(&mut self, state: &State, t: f64, now: Option<f64>) -> Frame {
        let now = now.unwrap_or_else(crate::state::clock);
        self.ingest(state, now);
        let g = |s: Sig| state.get(s);
        let (cpu, gpu) = (g(Sig::Cpu), g(Sig::Gpu));
        let ambient = cpu.max(gpu).max(0.6 * g(Sig::Mem));
        let loc = g(Sig::ReqLocal) + g(Sig::TokLocal);
        let rem = g(Sig::ReqRemote) + g(Sig::TokRemote);
        let mix = if loc + rem > 0.02 { Some(rem / (loc + rem)) } else { None };
        let gain = self.gain();
        let alert = now < self.alert_until;
        let flash = alert && (now * 6.0) as i64 % 2 == 0;
        if self.preview.as_deref() == Some("traffic") && now >= self.next_demo {
            let mut rng = rand::rng();
            let u: f64 = rng.random();
            self.next_demo = now - (1.0 - u).ln() / self.look.demo_rps;
            let r: f64 = rng.random();
            let tier = if r < 0.33 { Tier::Remote } else { Tier::Local };
            let status = if r > 0.9 { 500 } else { 200 };
            state.emit_at(now, EventKind::Request { tier, status, ms: rng.random_range(100.0..8000.0) });
        }
        let sev = match self.preview.as_deref() {
            Some("ok") => "ok",
            Some("degraded") => "degraded",
            Some("outage") => "outage",
            _ => severity(state),
        };
        self.severity = sev;
        state.0.lock().severity = Some(sev);
        let outage_k = 0.2 + 0.8 * (0.5 + 0.5 * (t * 2.0 * std::f64::consts::PI / 3.0).sin());

        let l = &self.look;
        let mut frame = Frame::new();
        for (i, (b, &n)) in self.bindings.iter().zip(&self.sizes).enumerate() {
            if n == 0 {
                continue;
            }
            let role = b.role.as_str();
            let level = match role {
                "cpu" => cpu.max(0.8 * g(Sig::CpuTemp)),
                "gpu" => gpu.max(0.8 * g(Sig::GpuTemp)),
                _ => ambient,
            };
            let mut base = ramp(level, &l.ramp);
            let mut k = 1.0 - l.breathe_depth + l.breathe_depth * (t * (l.breathe_speed + 3.0 * level) + i as f64).sin();
            if role == "fans"
                && let Some(mix) = mix {
                    base = lerp(base, lerp(l.local, l.remote, mix), (0.4 + loc + rem).min(1.0));
                }
            if role == "gpu" && g(Sig::LocalBusy) > 0.5 {
                base = lerp(base, l.local, 0.7);
                k = 0.6 + 0.4 * (t * 5.0).sin();
            }
            if role == "ambient" && g(Sig::SpendToday) > 0.0 {
                base = lerp(base, l.remote, g(Sig::SpendToday).min(1.0));
            }
            if sev == "degraded" {
                base = lerp(base, l.amber, 0.85);
            }
            let strip = role == "strip";
            let c0 = scale(base, k * if strip { l.strip_base } else { 1.0 });
            let mut leds = vec![c0; n];
            if strip {
                for p in &self.pulses {
                    let pos = (now - p.born) / l.pulse_seconds * (n as f64 + 2.0 * p.width) - p.width;
                    for (j, c) in leds.iter_mut().enumerate() {
                        *c = add(*c, scale(p.color, (1.0 - (j as f64 - pos).abs() / p.width).max(0.0)));
                    }
                }
            }
            if sev == "outage" {
                leds = vec![scale(l.error, outage_k); n];
            } else if flash {
                leds = vec![l.error; n];
            }
            frame.insert(i, leds.into_iter().map(|c| scale(c, gain)).collect());
        }
        frame
    }
}
