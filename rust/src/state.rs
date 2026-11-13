//! Shared machine state. Collectors write, the scene reads. Mirrors seele/state.py.
//! OWNED BY THE ORCHESTRATOR: agents use this API and must not change it.
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;
use serde::Serialize;

/// Every continuous signal. Names (`as_str`) match docs/agents/collector-contract.md exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Sig {
    Cpu, Mem, CpuTemp, Gpu, Vram, GpuPower, GpuTemp,
    ReqLocal, ReqRemote, ErrRate, EgressLocal, EgressRemote, ContainersBad,
    LocalBusy, VramModels, TokLocal, TokRemote, SpendToday, GatewayDown,
}

impl Sig {
    pub const ALL: [Sig; 19] = [
        Sig::Cpu, Sig::Mem, Sig::CpuTemp, Sig::Gpu, Sig::Vram, Sig::GpuPower, Sig::GpuTemp,
        Sig::ReqLocal, Sig::ReqRemote, Sig::ErrRate, Sig::EgressLocal, Sig::EgressRemote, Sig::ContainersBad,
        Sig::LocalBusy, Sig::VramModels, Sig::TokLocal, Sig::TokRemote, Sig::SpendToday, Sig::GatewayDown,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Sig::Cpu => "cpu", Sig::Mem => "mem", Sig::CpuTemp => "cpu_temp", Sig::Gpu => "gpu",
            Sig::Vram => "vram", Sig::GpuPower => "gpu_power", Sig::GpuTemp => "gpu_temp",
            Sig::ReqLocal => "req_local", Sig::ReqRemote => "req_remote", Sig::ErrRate => "err_rate",
            Sig::EgressLocal => "egress_local", Sig::EgressRemote => "egress_remote",
            Sig::ContainersBad => "containers_bad", Sig::LocalBusy => "local_busy",
            Sig::VramModels => "vram_models", Sig::TokLocal => "tok_local", Sig::TokRemote => "tok_remote",
            Sig::SpendToday => "spend_today", Sig::GatewayDown => "gateway_down",
        }
    }
    pub fn from_str(s: &str) -> Option<Sig> {
        Sig::ALL.into_iter().find(|x| x.as_str() == s)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Health { Ok, Degraded, Down }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier { Local, Remote }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level { Warn, Crit }

#[derive(Clone, Debug, PartialEq)]
pub enum EventKind {
    Request { tier: Tier, status: u16, ms: f64 },
    Alert { source: String, level: Level, msg: String },
    Container { name: String, state: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// Seconds on the scene clock (monotonic seconds since process start; replay sets it explicitly).
    pub t: f64,
    pub kind: EventKind,
}

/// EMA with a time constant, independent of update rate (seele/state.py Signal).
#[derive(Clone, Debug)]
pub struct Signal {
    pub tau: f64,
    pub value: f64,
    last: Option<Instant>,
}

impl Signal {
    pub fn new(tau: f64) -> Self { Signal { tau, value: 0.0, last: None } }
    pub fn push(&mut self, x: f64) {
        let now = Instant::now();
        match self.last {
            None => self.value = x,
            Some(prev) => {
                let a = 1.0 - (-(now - prev).as_secs_f64() / self.tau).exp();
                self.value += a * (x - self.value);
            }
        }
        self.last = Some(now);
    }
}

#[derive(Default, Debug)]
pub struct Inner {
    pub signals: HashMap<Sig, Signal>,
    pub events: VecDeque<Event>,
    pub health: HashMap<String, Health>,
    /// Written by the scene each frame: "ok" | "degraded" | "outage".
    pub severity: Option<&'static str>,
}

/// Cheap to clone; share between tasks.
#[derive(Clone, Default)]
pub struct State(pub Arc<Mutex<Inner>>);

pub const DEFAULT_TAU: f64 = 1.5;
const MAX_EVENTS: usize = 256;

static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// Scene clock: seconds since first call (process start, effectively).
pub fn clock() -> f64 {
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

impl State {
    pub fn new() -> Self { Self::default() }

    /// Push a 0..1 value (clamped) with smoothing time constant `tau`.
    pub fn push(&self, s: Sig, x: f64, tau: f64) {
        let mut g = self.0.lock();
        g.signals.entry(s).or_insert_with(|| Signal::new(tau)).push(x.clamp(0.0, 1.0));
    }
    /// Set the smoothed value directly (replay/tests only).
    pub fn set(&self, s: Sig, x: f64) {
        self.0.lock().signals.entry(s).or_insert_with(|| Signal::new(DEFAULT_TAU)).value = x.clamp(0.0, 1.0);
    }
    pub fn get(&self, s: Sig) -> f64 {
        self.0.lock().signals.get(&s).map(|x| x.value).unwrap_or(0.0)
    }
    pub fn emit(&self, kind: EventKind) {
        self.emit_at(clock(), kind)
    }
    pub fn emit_at(&self, t: f64, kind: EventKind) {
        let mut g = self.0.lock();
        if g.events.len() >= MAX_EVENTS { g.events.pop_front(); }
        g.events.push_back(Event { t, kind });
    }
    pub fn drain(&self) -> Vec<Event> {
        self.0.lock().events.drain(..).collect()
    }
    pub fn set_health(&self, source: &str, h: Health) {
        self.0.lock().health.insert(source.to_string(), h);
    }
    pub fn health(&self, source: &str) -> Option<Health> {
        self.0.lock().health.get(source).copied()
    }
}
