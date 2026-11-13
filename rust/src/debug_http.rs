//! Tiny JSON endpoint: GET /state -> signals, health, severity. Mirrors seele/debug_http.py.
use std::collections::BTreeMap;

use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::state::State;

pub fn snapshot(state: &State) -> serde_json::Value {
    let g = state.0.lock();
    let signals: BTreeMap<&str, f64> = g.signals.iter().map(|(k, s)| (k.as_str(), (s.value * 1000.0).round_ties_even() / 1000.0)).collect();
    let health: BTreeMap<&String, _> = g.health.iter().collect();
    json!({"signals": signals, "health": health, "severity": g.severity})
}

pub async fn serve(state: State, host: &str, port: u16) {
    let listener = match TcpListener::bind((host, port)).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("debug http: cannot bind {host}:{port}: {e}");
            return;
        }
    };
    loop {
        let Ok((mut sock, _)) = listener.accept().await else { continue };
        let state = state.clone();
        tokio::spawn(async move {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 16384 {
                match sock.read(&mut chunk).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                }
            }
            let body = snapshot(&state).to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = sock.write_all(resp.as_bytes()).await;
            let _ = sock.shutdown().await;
        });
    }
}
