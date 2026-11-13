//! Traefik collector: tails the edge access log (JSON on stdout) for request events and rates.
use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::state::{EventKind, Health, Sig, State, Tier};

const CONTAINER: &str = "research-stack-edge-1";
const WINDOW: f64 = 10.0;

/// 5xx, rate limits, and auth failures (user decision, 2026-11-12).
pub fn is_error(status: u16) -> bool {
    status >= 500 || matches!(status, 401 | 403 | 429)
}

#[derive(Debug, PartialEq)]
pub struct Req { pub tier: Tier, pub status: u16, pub ms: f64 }

/// Parse one access-log line; None if not JSON or not a gateway request.
pub fn parse_line(line: &str) -> Option<Req> {
    let d: Value = serde_json::from_str(line.trim()).ok()?;
    let path = d.get("RequestPath")?.as_str()?;
    let status = num(d.get("DownstreamStatus")?)? as u16;
    let ms = num(d.get("Duration")?)? / 1e6;
    let tier = if path.starts_with("/open/") { Tier::Remote } else if path.starts_with("/v1/") { Tier::Local } else { return None };
    Some(Req { tier, status, ms })
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

pub struct Window { span: f64, q: VecDeque<(f64, Tier, u16)> }

impl Window {
    pub fn new(span: f64) -> Self { Window { span, q: VecDeque::new() } }
    pub fn add(&mut self, t: f64, tier: Tier, status: u16) { self.q.push_back((t, tier, status)); }
    /// (req_local, req_remote, err_rate), each 0..1.
    pub fn stats(&mut self, now: f64) -> (f64, f64, f64) {
        while self.q.front().is_some_and(|f| now - f.0 > self.span) { self.q.pop_front(); }
        let (mut l, mut r, mut err) = (0u32, 0u32, 0u32);
        for (_, tier, st) in &self.q {
            if *tier == Tier::Local { l += 1 } else { r += 1 }
            if is_error(*st) { err += 1 }
        }
        let total = self.q.len();
        (
            (l as f64 / self.span / 2.0).min(1.0),
            (r as f64 / self.span / 2.0).min(1.0),
            if total > 0 { err as f64 / total as f64 } else { 0.0 },
        )
    }
}

async fn tail(state: &State, win: &Mutex<Window>, t0: Instant) -> anyhow::Result<()> {
    let mut child = Command::new("docker")
        .args(["logs", "-f", "--since", "0s", CONTAINER])
        .stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true).spawn()?;
    let mut lines = BufReader::new(child.stdout.take().ok_or_else(|| anyhow::anyhow!("no stdout"))?).lines();
    state.set_health("traefik", Health::Ok);
    while let Some(line) = lines.next_line().await? {
        if let Some(r) = parse_line(&line) {
            win.lock().add(t0.elapsed().as_secs_f64(), r.tier, r.status);
            state.emit(EventKind::Request { tier: r.tier, status: r.status, ms: r.ms });
        }
    }
    anyhow::bail!("docker logs exited")
}

async fn push_loop(state: State, win: Arc<Mutex<Window>>, t0: Instant) {
    loop {
        let (a, b, e) = win.lock().stats(t0.elapsed().as_secs_f64());
        state.push(Sig::ReqLocal, a, 1.0);
        state.push(Sig::ReqRemote, b, 1.0);
        state.push(Sig::ErrRate, e, 1.0);
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

pub async fn run(state: State, _opts: toml::Table) {
    let t0 = Instant::now();
    let win = Arc::new(Mutex::new(Window::new(WINDOW)));
    let pusher = tokio::spawn(push_loop(state.clone(), win.clone(), t0));
    let mut last_log: Option<Instant> = None;
    loop {
        if let Err(e) = tail(&state, &win, t0).await {
            state.set_health("traefik", Health::Down);
            if last_log.is_none_or(|t| t.elapsed().as_secs() > 60) {
                last_log = Some(Instant::now());
                tracing::warn!("traefik collector: {e}");
            }
        }
        if pusher.is_finished() { return std::future::pending().await; }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_local() {
        let r = parse_line(r#"{"RequestPath":"/v1/models","DownstreamStatus":401,"Duration":2500000}"#).unwrap();
        assert_eq!(r, Req { tier: Tier::Local, status: 401, ms: 2.5 });
    }

    #[test]
    fn parse_remote_and_skip() {
        let r = parse_line(r#"{"RequestPath":"/open/v1/x","DownstreamStatus":200,"Duration":1000000}"#).unwrap();
        assert_eq!(r.tier, Tier::Remote);
        assert!(parse_line(r#"{"RequestPath":"/ping","DownstreamStatus":200,"Duration":1}"#).is_none());
    }

    #[test]
    fn parse_garbage() {
        assert!(parse_line("time=... level=info msg=hi").is_none());
        assert!(parse_line("{}").is_none());
        assert!(parse_line("").is_none());
    }

    #[test]
    fn window() {
        let mut w = Window::new(10.0);
        for _ in 0..10 { w.add(0.0, Tier::Local, 200); }
        w.add(0.0, Tier::Remote, 429);
        w.add(0.0, Tier::Remote, 500);
        let (a, b, e) = w.stats(1.0);
        assert!(a == 0.5 && b == 0.1 && (e - 2.0 / 12.0).abs() < 1e-9);
        assert_eq!(w.stats(20.0), (0.0, 0.0, 0.0));
    }

    #[test]
    fn is_error_includes_auth_failures() {
        assert!([401, 403, 429, 500, 502].into_iter().all(is_error));
        assert!(![200, 201, 404].into_iter().any(is_error));
    }

    #[tokio::test]
    #[ignore]
    async fn live() {
        let s = State::new();
        let _ = tokio::time::timeout(Duration::from_secs(8), run(s.clone(), toml::Table::new())).await;
        crate::collectors::host::dump_state(&s, "traefik");
    }
}
