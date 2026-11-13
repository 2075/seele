//! Tokens/s and daily spend from LiteLLM spend logs (read-only SELECTs via docker exec psql).
//! The container trusts the local socket, so no password is used. Only startTime, total_tokens and spend are
//! selected (never prompt or key columns). Spend is USD, treated as ~EUR.
use std::collections::VecDeque;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::{Local, TimeZone, Timelike};
use tokio::process::Command;

use crate::state::{Health, Sig, State};

const CONTAINER: &str = "research-stack-litellm-db-1";
const TIERS: [(&str, &str, Sig); 2] = [("local", "gw_local", Sig::TokLocal), ("remote", "gw_open", Sig::TokRemote)];
const WINDOW: f64 = 30.0;
/// startTime is stored in UTC.
const EPOCH: &str = "extract(epoch from \"startTime\")";

/// Tokens/s log-normalised: 1 -> 0, 500 -> 1.
pub fn tok_norm(tps: f64) -> f64 {
    if tps > 1.0 { (tps.log10() / 500f64.log10()).clamp(0.0, 1.0) } else { 0.0 }
}

/// Lines 'epoch|tokens' -> [(epoch, tokens)]; bad lines are skipped.
pub fn parse_rows(out: &str) -> Vec<(f64, f64)> {
    out.lines().filter_map(|line| {
        let mut it = line.split('|');
        let t = it.next()?.trim().parse::<f64>().ok()?;
        let n = it.next()?.trim();
        Some((t, if n.is_empty() { 0.0 } else { n.parse::<f64>().ok()? }))
    }).collect()
}

pub fn window_tps(win: &mut VecDeque<(f64, f64)>, now: f64, span: f64) -> f64 {
    while win.front().is_some_and(|f| f.0 < now - span) { win.pop_front(); }
    win.iter().map(|(_, n)| n).sum::<f64>() / span
}

pub fn spend_ratio(spend: f64, budget: f64) -> f64 {
    if budget > 0.0 { (spend / budget).clamp(0.0, 1.0) } else { 0.0 }
}

pub fn local_midnight_epoch(now: Option<f64>) -> f64 {
    let n = now.unwrap_or_else(now_epoch);
    let d = Local.timestamp_opt(n as i64, 0).single().unwrap_or_else(Local::now);
    let m = d.with_hour(0).and_then(|d| d.with_minute(0)).and_then(|d| d.with_second(0)).and_then(|d| d.with_nanosecond(0)).unwrap_or(d);
    m.timestamp() as f64
}

fn now_epoch() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

async fn psql(db: &str, sql: &str) -> anyhow::Result<String> {
    let fut = Command::new("docker")
        .args(["exec", "-e", "PGOPTIONS=-c default_transaction_read_only=on", CONTAINER, "psql", "-U", db, "-d", db, "-At", "-c", sql])
        .kill_on_drop(true).output();
    let o = tokio::time::timeout(Duration::from_secs(8), fut).await??;
    if !o.status.success() {
        let e = String::from_utf8_lossy(&o.stderr);
        anyhow::bail!("{}", e.trim().chars().take(200).collect::<String>());
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

async fn cycle(state: &State, wins: &mut [VecDeque<(f64, f64)>; 2], last: &mut [f64; 2], budget: f64) -> anyhow::Result<()> {
    for (i, (_, db, sig)) in TIERS.iter().enumerate() {
        let sql = format!(
            "SELECT {EPOCH}, total_tokens FROM \"LiteLLM_SpendLogs\" WHERE {EPOCH} > {:.3} ORDER BY \"startTime\"", last[i]);
        for (t, n) in parse_rows(&psql(db, &sql).await?) {
            wins[i].push_back((t, n));
            last[i] = last[i].max(t);
        }
        state.push(*sig, tok_norm(window_tps(&mut wins[i], now_epoch(), WINDOW)), 2.0);
    }
    let m = local_midnight_epoch(None);
    let out = psql(TIERS[1].1, &format!("SELECT COALESCE(SUM(spend),0) FROM \"LiteLLM_SpendLogs\" WHERE {EPOCH} >= {m:.0}")).await?;
    let spend: f64 = out.trim().parse().unwrap_or(0.0);
    state.push(Sig::SpendToday, spend_ratio(spend, budget), 5.0);
    Ok(())
}

pub async fn run(state: State, opts: toml::Table) {
    let budget = opts.get("budget_eur_day").and_then(|v| v.as_float().or(v.as_integer().map(|i| i as f64))).unwrap_or(5.0);
    let mut wins: [VecDeque<(f64, f64)>; 2] = [VecDeque::new(), VecDeque::new()];
    let mut last = [now_epoch() - WINDOW; 2];
    let mut last_log: Option<Instant> = None;
    let mut fails = 0u64;
    loop {
        match cycle(&state, &mut wins, &mut last, budget).await {
            Ok(()) => {
                state.set_health("litellm", Health::Ok);
                fails = 0;
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            Err(e) => {
                fails += 1;
                state.set_health("litellm", Health::Down);
                for s in [Sig::TokLocal, Sig::TokRemote, Sig::SpendToday] { state.push(s, 0.0, 2.0); }
                if last_log.is_none_or(|t| t.elapsed().as_secs() > 60) {
                    last_log = Some(Instant::now());
                    tracing::warn!("litellm collector: {e}");
                }
                tokio::time::sleep(Duration::from_secs((5 * fails).min(60))).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tok_norm_cases() {
        assert!(tok_norm(0.5) == 0.0 && tok_norm(1.0) == 0.0);
        assert!((tok_norm(500.0) - 1.0).abs() < 1e-9 && tok_norm(5000.0) == 1.0);
        assert!(tok_norm(30.0) > 0.0 && tok_norm(30.0) < 1.0);
    }

    #[test]
    fn parse_rows_cases() {
        assert_eq!(parse_rows("100.5|20\nbad\n101|\n"), vec![(100.5, 20.0), (101.0, 0.0)]);
    }

    #[test]
    fn window() {
        let mut w: VecDeque<(f64, f64)> = [(0.0, 100.0), (80.0, 300.0), (95.0, 300.0)].into();
        assert_eq!(window_tps(&mut w, 100.0, 30.0), 20.0);
        assert_eq!(w.len(), 2);
    }

    #[test]
    fn spend() {
        assert!(spend_ratio(2.5, 5.0) == 0.5 && spend_ratio(9.0, 5.0) == 1.0 && spend_ratio(1.0, 0.0) == 0.0);
    }

    #[test]
    fn midnight() {
        assert!(local_midnight_epoch(Some(1_800_000_000.0)) <= 1_800_000_000.0);
    }

    #[tokio::test]
    #[ignore]
    async fn live() {
        let s = State::new();
        let _ = tokio::time::timeout(Duration::from_secs(8), run(s.clone(), toml::Table::new())).await;
        crate::collectors::host::dump_state(&s, "litellm");
    }
}
