//! End-to-end gateway reachability for both tiers. Port of seele/collectors/gateway.py.
use std::time::Duration;

use crate::state::{EventKind, Health, Level, Sig, State};

const PROBES: [(&str, &str); 2] = [("local", "/health/liveliness"), ("remote", "/open/health/liveliness")];

/// results: one bool per tier (up?). Returns (health, gateway_down level).
pub fn classify(results: &[bool]) -> (Health, f64) {
    let up = results.iter().filter(|b| **b).count();
    if up == results.len() { (Health::Ok, 0.0) } else if up > 0 { (Health::Degraded, 0.5) } else { (Health::Down, 1.0) }
}

pub async fn run(state: State, opts: toml::Table) {
    let base = opts.get("base").and_then(|v| v.as_str()).unwrap_or("http://127.0.0.1:4000").to_string();
    let interval = opts.get("interval").and_then(|v| v.as_float().or(v.as_integer().map(|i| i as f64))).unwrap_or(2.0);
    let client = loop {
        match reqwest::Client::builder().timeout(Duration::from_secs(2)).build() {
            Ok(c) => break c,
            Err(e) => { tracing::warn!("gateway client: {e}"); tokio::time::sleep(Duration::from_secs(60)).await; }
        }
    };
    let mut fails = [0u32; 2];
    loop {
        for (i, (_, path)) in PROBES.iter().enumerate() {
            match client.get(format!("{base}{path}")).send().await {
                Ok(r) => {
                    let s = r.status().as_u16();
                    fails[i] = if s < 500 && s != 404 { 0 } else { fails[i] + 1 };
                }
                Err(_) => fails[i] += 1,
            }
        }
        let ups: Vec<bool> = fails.iter().map(|f| *f < 2).collect();
        let (health, level) = classify(&ups);
        if state.health("gateway") != Some(health) && health != Health::Ok {
            state.emit(EventKind::Alert {
                source: "gateway".into(),
                level: if health == Health::Down { Level::Crit } else { Level::Warn },
                msg: if health == Health::Down { "down" } else { "degraded" }.into(),
            });
        }
        state.set_health("gateway", health);
        state.push(Sig::GatewayDown, level, 0.3);
        tokio::time::sleep(Duration::from_secs_f64(interval)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_cases() {
        assert_eq!(classify(&[true, true]), (Health::Ok, 0.0));
        assert_eq!(classify(&[true, false]), (Health::Degraded, 0.5));
        assert_eq!(classify(&[false, false]), (Health::Down, 1.0));
    }

    #[tokio::test]
    #[ignore]
    async fn live() {
        let s = State::new();
        let _ = tokio::time::timeout(Duration::from_secs(8), run(s.clone(), toml::Table::new())).await;
        crate::collectors::host::dump_state(&s, "gateway");
    }
}
