//! Deterministic scene replay: the parity spec against the Python port.
use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::config;
use crate::scene::Scene;
use crate::state::{EventKind, Health, Level, Sig, State, Tier};

pub fn replay(fixture: &Value, config_path: &str) -> Result<Value> {
    let cfg = config::load(config_path)?;
    let sizes: Vec<usize> = fixture["sizes"].as_array().context("sizes")?.iter().map(|x| x.as_u64().unwrap_or(0) as usize).collect();
    let mut scene = Scene::new(cfg.zones.clone(), sizes, 0.8, None, None, cfg.look.clone());
    let state = State::new();
    let mut out = vec![];
    for step in fixture["steps"].as_array().context("steps")? {
        let t = step["t"].as_f64().context("t")?;
        if let Some(sig) = step["signals"].as_object() {
            for (k, v) in sig {
                if let Some(s) = Sig::from_str(k) {
                    state.set(s, v.as_f64().unwrap_or(0.0));
                }
            }
        }
        if let Some(h) = step["health"].as_object() {
            for (k, v) in h {
                let h = match v.as_str() {
                    Some("ok") => Health::Ok,
                    Some("down") => Health::Down,
                    _ => Health::Degraded,
                };
                state.set_health(k, h);
            }
        }
        for e in step["events"].as_array().into_iter().flatten() {
            let kind = match e["kind"].as_str() {
                Some("request") => EventKind::Request {
                    tier: if e["tier"] == "remote" { Tier::Remote } else { Tier::Local },
                    status: e["status"].as_u64().unwrap_or(200) as u16,
                    ms: e["ms"].as_f64().unwrap_or(0.0),
                },
                Some("alert") => EventKind::Alert {
                    source: e["source"].as_str().unwrap_or("").into(),
                    level: if e["level"] == "crit" { Level::Crit } else { Level::Warn },
                    msg: e["msg"].as_str().unwrap_or("").into(),
                },
                _ => EventKind::Container { name: e["name"].as_str().unwrap_or("").into(), state: e["state"].as_str().unwrap_or("").into() },
            };
            state.emit_at(t, kind);
        }
        let frame = scene.render(&state, t, Some(t));
        let frame: serde_json::Map<String, Value> =
            frame.iter().map(|(i, v)| (i.to_string(), Value::Array(v.iter().map(|c| json!([c.0, c.1, c.2])).collect()))).collect();
        out.push(json!({"t": step["t"], "severity": scene.severity, "frame": frame}));
    }
    Ok(Value::Array(out))
}

pub fn main(fixture_path: &str, config_path: &str) -> Result<()> {
    let fx: Value = serde_json::from_str(&std::fs::read_to_string(fixture_path)?)?;
    println!("{}", serde_json::to_string(&replay(&fx, config_path)?)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_python_golden() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/");
        let fx: Value = serde_json::from_str(&std::fs::read_to_string(format!("{dir}tests/parity/fixture.json")).unwrap()).unwrap();
        let golden: Value = serde_json::from_str(&std::fs::read_to_string(format!("{dir}tests/parity/golden.json")).unwrap()).unwrap();
        let got = replay(&fx, &format!("{dir}tests/parity/config.toml")).unwrap();
        assert_eq!(got, golden);
    }
}
