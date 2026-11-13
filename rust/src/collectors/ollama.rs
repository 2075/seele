//! Local model activity from Ollama: loaded-model VRAM share and llama-server slot busyness.
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::Value;
use tokio::process::Command;

use crate::state::{Health, Sig, State};

const PS_URL: &str = "http://127.0.0.1:11434/api/ps";
const DEFAULT_VRAM: u64 = 8 * 1024 * 1024 * 1024;

/// Ports of real runners only: the executable itself must be llama-server (not a shell mentioning it).
pub fn parse_ports(out: &str) -> Vec<u16> {
    let mut ports = std::collections::BTreeSet::new();
    for line in out.lines() {
        let parts: Vec<&str> = line.trim_start().splitn(3, char::is_whitespace).collect();
        if parts.len() == 3 && parts[1].rsplit('/').next() == Some("llama-server") {
            let args: Vec<&str> = parts[2].split_whitespace().collect();
            for (i, a) in args.iter().enumerate() {
                let v = if *a == "--port" { args.get(i + 1).copied() } else { a.strip_prefix("--port=") };
                if let Some(p) = v.and_then(|v| v.parse::<u16>().ok()) { ports.insert(p); }
            }
        }
    }
    ports.into_iter().collect()
}

pub fn vram_fraction(ps: &Value, total: u64) -> f64 {
    let used: u64 = ps.get("models").and_then(|m| m.as_array()).map_or(0, |a| {
        a.iter().map(|m| m.get("size_vram").and_then(|v| v.as_u64()).unwrap_or(0)).sum()
    });
    if total > 0 { (used as f64 / total as f64).clamp(0.0, 1.0) } else { 0.0 }
}

pub fn any_busy(slots: &Value) -> bool {
    slots.as_array().is_some_and(|a| a.iter().any(|s| s.get("is_processing").and_then(|b| b.as_bool()) == Some(true)))
}

fn gpu_total() -> u64 {
    nvml_wrapper::Nvml::init()
        .and_then(|n| n.device_by_index(0).and_then(|d| d.memory_info()).map(|m| m.total))
        .unwrap_or(DEFAULT_VRAM)
}

async fn pgrep() -> Vec<u16> {
    match Command::new("pgrep").args(["-af", "llama-server"]).stderr(std::process::Stdio::null()).output().await {
        Ok(o) => parse_ports(&String::from_utf8_lossy(&o.stdout)),
        Err(_) => vec![],
    }
}

struct Shared {
    ps_ok: AtomicBool,
    slots_ok: AtomicBool,
    last_log: Mutex<Option<Instant>>,
}

impl Shared {
    fn warn(&self, msg: String) {
        let mut l = self.last_log.lock();
        if l.is_none_or(|t| t.elapsed().as_secs() > 60) {
            *l = Some(Instant::now());
            tracing::warn!("{msg}");
        }
    }
    fn set_health(&self, state: &State) {
        let ps = self.ps_ok.load(Ordering::Relaxed);
        let sl = self.slots_ok.load(Ordering::Relaxed);
        state.set_health("ollama", if ps && sl { Health::Ok } else if ps { Health::Degraded } else { Health::Down });
    }
}

async fn ps_loop(state: &State, http: &reqwest::Client, sh: &Shared, total: u64) {
    loop {
        let r: Result<Value, reqwest::Error> = async { http.get(PS_URL).send().await?.error_for_status()?.json().await }.await;
        match r {
            Ok(v) => { state.push(Sig::VramModels, vram_fraction(&v, total), 2.0); sh.ps_ok.store(true, Ordering::Relaxed); }
            Err(e) => {
                sh.ps_ok.store(false, Ordering::Relaxed);
                state.push(Sig::VramModels, 0.0, 2.0);
                sh.warn(format!("ollama /api/ps: {e}"));
            }
        }
        sh.set_health(state);
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn slots_loop(state: &State, http: &reqwest::Client, sh: &Shared) {
    let mut ports: Vec<u16> = vec![];
    let mut found: Option<Instant> = None;
    let mut fails = 0u32;
    loop {
        if ports.is_empty() || found.is_none_or(|t| t.elapsed().as_secs() > 10) {
            ports = pgrep().await;
            found = Some(Instant::now());
        }
        let (mut busy, mut err) = (false, false);
        for port in &ports {
            let r: Result<Value, reqwest::Error> =
                async { http.get(format!("http://127.0.0.1:{port}/slots")).send().await?.error_for_status()?.json().await }.await;
            match r {
                Ok(v) => busy = busy || any_busy(&v),
                Err(e) if e.is_timeout() => busy = true, // too busy to answer /slots: generating
                Err(_) => err = true,
            }
        }
        if err { found = None; }
        // A freshly spawned runner refuses connections briefly: degrade only after 3 failed rounds.
        fails = if err { fails + 1 } else { 0 };
        sh.slots_ok.store(fails < 3, Ordering::Relaxed);
        state.push(Sig::LocalBusy, if busy { 1.0 } else { 0.0 }, 0.3);
        sh.set_health(state);
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

pub async fn run(state: State, _opts: toml::Table) {
    let total = tokio::task::spawn_blocking(gpu_total).await.unwrap_or(DEFAULT_VRAM);
    let sh = Shared { ps_ok: AtomicBool::new(true), slots_ok: AtomicBool::new(true), last_log: Mutex::new(None) };
    loop {
        match reqwest::Client::builder().timeout(Duration::from_millis(1500)).build() {
            Ok(http) => { tokio::join!(ps_loop(&state, &http, &sh, total), slots_loop(&state, &http, &sh)); }
            Err(e) => {
                state.set_health("ollama", Health::Down);
                sh.warn(format!("ollama client: {e}"));
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ports() {
        let out = "123 /x/llama-server --model m --port 42773 -c 4096\n124 llama-server --port=36641\n125 pgrep -af llama-server";
        assert_eq!(parse_ports(out), vec![36641, 42773]);
        assert_eq!(parse_ports(""), Vec::<u16>::new());
    }

    #[test]
    fn vram() {
        let g = 1024u64.pow(3);
        let ps = json!({"models": [{"size_vram": 2 * g}, {"size_vram": g}]});
        assert_eq!(vram_fraction(&ps, 8 * g), 3.0 / 8.0);
        assert_eq!(vram_fraction(&json!({"models": []}), 8), 0.0);
        assert_eq!(vram_fraction(&json!({"models": [{"size_vram": 99}]}), 8), 1.0);
    }

    #[test]
    fn busy() {
        assert!(any_busy(&json!([{"is_processing": false}, {"is_processing": true}])));
        assert!(!any_busy(&json!([{"is_processing": false}])));
        assert!(!any_busy(&json!({"error": 1})));
    }

    #[test]
    fn parse_ports_ignores_shells_mentioning_llama_server() {
        let out = "1 /usr/bin/zsh -c pgrep -af llama-server --port 9\n2 /usr/local/lib/ollama/llama-server --model x --port 42773";
        assert_eq!(parse_ports(out), vec![42773]);
    }

    #[tokio::test]
    #[ignore]
    async fn live() {
        let s = State::new();
        let _ = tokio::time::timeout(Duration::from_secs(8), run(s.clone(), toml::Table::new())).await;
        crate::collectors::host::dump_state(&s, "ollama");
    }
}
