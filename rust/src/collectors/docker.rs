//! Docker collector: container health, container events and egress throughput via the Engine API (bollard).
use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bollard::Docker;
use bollard::query_parameters::{EventsOptionsBuilder, ListContainersOptionsBuilder, StatsOptionsBuilder};
use futures_util::StreamExt;

use crate::state::{EventKind, Health, Level, Sig, State};

const PROJECT: &str = "research-stack";
const EGRESS: [(&str, Sig); 2] = [("research-stack-egress-local-1", Sig::EgressLocal), ("research-stack-egress-open-1", Sig::EgressRemote)];
/// Core research-stack services. Optional profiles (qdrant, open-webui) are not expected.
pub const EXPECTED: [&str; 7] = ["edge", "guard", "gw-local", "gw-open", "litellm-db", "egress-local", "egress-open"];

/// 1 KB/s -> 0, 10 MB/s -> 1, log scale.
pub fn norm_bytes_rate(bps: f64) -> f64 {
    if bps <= 1000.0 { 0.0 } else { ((bps / 1000.0).log10() / 4.0).min(1.0) }
}

/// Sum of rx+tx bytes over all networks of one stats sample.
pub fn net_bytes(stats: &bollard::models::ContainerStatsResponse) -> u64 {
    stats.networks.as_ref().map_or(0, |n| n.values().map(|v| v.rx_bytes.unwrap_or(0) + v.tx_bytes.unwrap_or(0)).sum())
}

/// Minimal container view for health accounting.
pub struct Ct { pub service: Option<String>, pub state: String, pub status: String }

/// Share of expected services that are missing, not running, or unhealthy.
/// Missing counts as bad: `docker compose down` removes containers, so an empty list means an outage.
pub fn bad_share(containers: &[Ct], expected: &[&str]) -> f64 {
    let mut seen: HashMap<&str, bool> = HashMap::new();
    for c in containers {
        let ok = c.state == "running" && !c.status.contains("unhealthy");
        if let Some(svc) = c.service.as_deref().filter(|s| !s.is_empty()) {
            let e = seen.entry(svc).or_insert(false);
            *e = *e || ok;
        }
    }
    let names: BTreeSet<&str> = expected.iter().copied().chain(seen.keys().copied()).collect();
    if names.is_empty() { return 0.0; }
    names.iter().filter(|n| !seen.get(*n).copied().unwrap_or(false)).count() as f64 / names.len() as f64
}

/// (container name, state) for an event worth reporting, else None.
pub fn parse_event(action: &str, name: Option<&str>) -> Option<(String, String)> {
    let (base, detail) = action.split_once(':').unwrap_or((action, ""));
    let base = base.trim();
    if !matches!(base, "health_status" | "die" | "start" | "oom") { return None; }
    let state = if base == "health_status" { detail.trim() } else { base };
    if state.is_empty() { return None; }
    Some((name.unwrap_or("?").to_string(), state.to_string()))
}

fn label_filter() -> HashMap<String, Vec<String>> {
    HashMap::from([("label".to_string(), vec![format!("com.docker.compose.project={PROJECT}")])])
}

async fn poll_containers(state: &State, d: &Docker) -> anyhow::Result<()> {
    loop {
        let r = tokio::time::timeout(Duration::from_secs(5), d.list_containers(Some(
            ListContainersOptionsBuilder::new().all(true).filters(&label_filter()).build()))).await;
        match r {
            Ok(Ok(list)) => {
                let cts: Vec<Ct> = list.iter().map(|c| Ct {
                    service: c.labels.as_ref().and_then(|l| l.get("com.docker.compose.service").cloned()),
                    state: c.state.as_ref().map(|s| s.to_string()).unwrap_or_default(),
                    status: c.status.clone().unwrap_or_default(),
                }).collect();
                state.push(Sig::ContainersBad, bad_share(&cts, &EXPECTED), 1.0);
                state.set_health("docker", Health::Ok);
            }
            Ok(Err(e)) => { state.set_health("docker", Health::Down); return Err(e.into()); }
            Err(e) => { state.set_health("docker", Health::Down); return Err(e.into()); }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

async fn events(state: &State, d: &Docker) -> anyhow::Result<()> {
    let mut flt = label_filter();
    flt.insert("type".into(), vec!["container".into()]);
    let since = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs().to_string();
    let mut s = d.events(Some(EventsOptionsBuilder::new().since(&since).filters(&flt).build()));
    while let Some(ev) = s.next().await {
        let ev = ev?;
        let name = ev.actor.as_ref().and_then(|a| a.attributes.as_ref()).and_then(|a| a.get("name")).map(|s| s.as_str());
        let Some((name, st)) = parse_event(ev.action.as_deref().unwrap_or(""), name) else { continue };
        state.emit(EventKind::Container { name: name.clone(), state: st.clone() });
        if matches!(st.as_str(), "unhealthy" | "die" | "oom") {
            state.emit(EventKind::Alert { source: "docker".into(), level: Level::Crit, msg: format!("{name}: {st}") });
        }
    }
    anyhow::bail!("events stream ended")
}

async fn stats(state: &State, d: &Docker, name: &str, sig: Sig) -> anyhow::Result<()> {
    let mut prev: Option<(Instant, u64)> = None;
    let mut s = d.stats(name, Some(StatsOptionsBuilder::new().stream(true).build()));
    while let Some(r) = s.next().await {
        let (now, total) = (Instant::now(), net_bytes(&r?));
        if let Some((pt, pb)) = prev {
            let dt = now.duration_since(pt).as_secs_f64();
            if dt > 0.0 {
                state.push(sig, norm_bytes_rate(total.saturating_sub(pb) as f64 / dt), 1.0);
            }
        }
        prev = Some((now, total));
    }
    anyhow::bail!("stats stream ended")
}

async fn forever<F, Fut>(state: State, label: &str, sigs: &[Sig], f: F)
where F: Fn(State) -> Fut, Fut: Future<Output = anyhow::Result<()>> {
    let mut last_log: Option<Instant> = None;
    loop {
        if let Err(e) = f(state.clone()).await {
            state.set_health("docker", Health::Down);
            if last_log.is_none_or(|t| t.elapsed().as_secs() > 60) {
                last_log = Some(Instant::now());
                tracing::warn!("docker collector {label}: {e}");
            }
            for s in sigs { state.push(*s, 0.0, 1.5); }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

pub async fn run(state: State, _opts: toml::Table) {
    // Connect lazily and retry: the socket may not exist yet.
    let connect = || Docker::connect_with_unix("/var/run/docker.sock", 120, bollard::API_DEFAULT_VERSION);
    let fresh = || async move { connect().map_err(anyhow::Error::from) };
    let poll = forever(state.clone(), "poll_containers", &[], |s| async move {
        let d = fresh().await?;
        poll_containers(&s, &d).await
    });
    let ev = forever(state.clone(), "events", &[], |s| async move {
        let d = fresh().await?;
        events(&s, &d).await
    });
    let st = EGRESS.map(|(n, sig)| {
        forever(state.clone(), "stats", if sig == Sig::EgressLocal { &[Sig::EgressLocal] } else { &[Sig::EgressRemote] }, move |s| async move {
            let d = fresh().await?;
            stats(&s, &d, n, sig).await
        })
    });
    let [a, b] = st;
    tokio::join!(poll, ev, a, b);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(svc: &str, state: &str, status: &str) -> Ct {
        Ct { service: Some(svc.into()), state: state.into(), status: status.into() }
    }

    #[test]
    fn norm() {
        assert_eq!(norm_bytes_rate(500.0), 0.0);
        assert_eq!(norm_bytes_rate(1000.0), 0.0);
        assert!((norm_bytes_rate(1e7) - 1.0).abs() < 1e-9);
        assert_eq!(norm_bytes_rate(1e12), 1.0);
        let x = norm_bytes_rate(1e4);
        assert!(x > 0.2 && x < 0.3);
    }

    #[test]
    fn net_bytes_sums() {
        use bollard::models::{ContainerNetworkStats, ContainerStatsResponse};
        let n = |rx, tx| ContainerNetworkStats { rx_bytes: Some(rx), tx_bytes: Some(tx), ..Default::default() };
        let s = ContainerStatsResponse {
            networks: Some(HashMap::from([("a".to_string(), n(5, 7)), ("b".to_string(), n(1, 1))])),
            ..Default::default()
        };
        assert_eq!(net_bytes(&s), 14);
        assert_eq!(net_bytes(&ContainerStatsResponse::default()), 0);
    }

    #[test]
    fn bad_share_basic() {
        let cs = [c("a", "running", "Up 1h (healthy)"), c("b", "running", "Up (unhealthy)"),
                  c("c", "exited", "Exited"), c("d", "running", "Up")];
        assert_eq!(bad_share(&cs, &[]), 0.5);
        assert_eq!(bad_share(&[], &[]), 0.0);
    }

    #[test]
    fn parse_event_cases() {
        assert_eq!(parse_event("health_status: unhealthy", Some("x")), Some(("x".into(), "unhealthy".into())));
        assert_eq!(parse_event("die", Some("y")), Some(("y".into(), "die".into())));
        assert_eq!(parse_event("exec_start: sh", None), None);
    }

    #[test]
    fn bad_share_counts_missing_expected_services() {
        let e = ["edge", "guard"];
        assert_eq!(bad_share(&[], &e), 1.0);
        assert_eq!(bad_share(&[c("edge", "running", "Up")], &e), 0.5);
        assert_eq!(bad_share(&[c("edge", "running", "Up"), c("guard", "running", "Up (unhealthy)")], &e), 0.5);
        assert_eq!(bad_share(&[c("edge", "running", "Up"), c("guard", "running", "Up")], &e), 0.0);
    }

    #[tokio::test]
    #[ignore]
    async fn live() {
        let s = State::new();
        let _ = tokio::time::timeout(Duration::from_secs(8), run(s.clone(), toml::Table::new())).await;
        crate::collectors::host::dump_state(&s, "docker");
    }
}
