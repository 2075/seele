//! Config loading. Mirrors seele/config.py.
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::colors::Rgb;

#[derive(Clone, Debug)]
pub struct ZoneBinding {
    pub device: String,
    pub zone: String,
    pub role: String,
}

#[derive(Clone, Debug)]
pub struct Look {
    pub ramp: Vec<(f64, Rgb)>,
    pub local: Rgb,
    pub remote: Rgb,
    pub error: Rgb,
    pub amber: Rgb,
    pub pulse_seconds: f64,
    pub pulse_width: f64,
    pub pulse_width_max: f64,
    pub pulse_slow_ms: f64,
    pub pulse_max: usize,
    pub strip_base: f64,
    pub breathe_depth: f64,
    pub breathe_speed: f64,
    pub demo_rps: f64,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            ramp: vec![
                (0.0, (0, 40, 255)),
                (0.35, (0, 200, 160)),
                (0.7, (255, 150, 0)),
                (1.0, (255, 0, 0)),
            ],
            local: (0, 255, 150),
            remote: (170, 0, 255),
            error: (255, 0, 0),
            amber: (255, 120, 0),
            pulse_seconds: 0.9,
            pulse_width: 1.0,
            pulse_width_max: 4.0,
            pulse_slow_ms: 6000.0,
            pulse_max: 24,
            strip_base: 0.55,
            breathe_depth: 0.25,
            breathe_speed: 0.6,
            demo_rps: 2.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub fps: u32,
    pub brightness: f64,
    pub zones: Vec<ZoneBinding>,
    pub quiet: Option<String>,
    pub preview: Option<String>,
    pub look: Look,
    pub collectors: Vec<String>,
    pub collector_opts: toml::Table,
}

fn num(v: &toml::Value) -> Option<f64> {
    v.as_float().or_else(|| v.as_integer().map(|i| i as f64))
}

/// Accept "#rrggbb" or [r, g, b].
fn rgb(v: &toml::Value) -> Result<Rgb> {
    if let Some(s) = v.as_str() {
        let s = s.trim_start_matches('#');
        if s.len() != 6 {
            bail!("bad colour {s:?}");
        }
        let p = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).context("bad colour");
        return Ok((p(0)?, p(2)?, p(4)?));
    }
    let a = v.as_array().context("colour must be string or array")?;
    let c: Vec<u8> = a.iter().map(|x| num(x).unwrap_or(0.0) as u8).collect();
    if c.len() != 3 {
        bail!("colour needs 3 components");
    }
    Ok((c[0], c[1], c[2]))
}

fn look(raw: &toml::Table) -> Result<Look> {
    let mut lk = Look::default();
    if let Some(pal) = raw.get("palette").and_then(|v| v.as_table()) {
        if let Some(r) = pal.get("ramp").and_then(|v| v.as_array()) {
            lk.ramp = r
                .iter()
                .map(|e| {
                    let e = e.as_array().context("ramp entry")?;
                    Ok((num(e.first().context("ramp pos")?).context("ramp pos")?, rgb(e.get(1).context("ramp colour")?)?))
                })
                .collect::<Result<_>>()?;
        }
        for (k, slot) in [("local", &mut lk.local), ("remote", &mut lk.remote), ("error", &mut lk.error), ("amber", &mut lk.amber)] {
            if let Some(v) = pal.get(k) {
                *slot = rgb(v)?;
            }
        }
    }
    if let Some(p) = raw.get("pulses").and_then(|v| v.as_table()) {
        for (k, v) in p {
            let Some(x) = num(v) else { continue };
            match k.as_str() {
                "pulse_seconds" => lk.pulse_seconds = x,
                "pulse_width" => lk.pulse_width = x,
                "pulse_width_max" => lk.pulse_width_max = x,
                "pulse_slow_ms" => lk.pulse_slow_ms = x,
                "pulse_max" => lk.pulse_max = x as usize,
                "strip_base" => lk.strip_base = x,
                "breathe_depth" => lk.breathe_depth = x,
                "breathe_speed" => lk.breathe_speed = x,
                "demo_rps" => lk.demo_rps = x,
                _ => {}
            }
        }
    }
    Ok(lk)
}

const DEFAULT_CONFIG: &str = include_str!("../assets/default-config.toml");

/// Default config location: `$XDG_CONFIG_HOME/seele/rgb.toml`, else `~/.config/seele/rgb.toml`.
pub fn default_path() -> String {
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var("HOME").ok().map(|h| std::path::Path::new(&h).join(".config")))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join("seele").join("rgb.toml").to_string_lossy().into_owned()
}

/// Write the compiled-in default config if `path` does not exist.
pub fn ensure_default(path: &str) -> Result<()> {
    let p = Path::new(path);
    if p.exists() {
        return Ok(());
    }
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(p, DEFAULT_CONFIG).with_context(|| format!("writing {path}"))?;
    tracing::info!("no config found; wrote default to {path}");
    Ok(())
}

pub fn load(path: impl AsRef<Path>) -> Result<Config> {
    let text = std::fs::read_to_string(path.as_ref()).with_context(|| format!("reading {}", path.as_ref().display()))?;
    let raw: toml::Table = text.parse().context("parsing config")?;
    let empty = toml::Table::new();
    let o = raw.get("openrgb").and_then(|v| v.as_table()).unwrap_or(&empty);
    let r = raw.get("render").and_then(|v| v.as_table()).unwrap_or(&empty);
    let s = |t: &toml::Table, k: &str| t.get(k).and_then(|v| v.as_str()).map(String::from);
    let mut zones = vec![];
    for z in raw.get("zone").and_then(|v| v.as_array()).into_iter().flatten() {
        let z = z.as_table().context("zone must be a table")?;
        let f = |k: &str| s(z, k).with_context(|| format!("zone missing {k}"));
        zones.push(ZoneBinding { device: f("device")?, zone: f("zone")?, role: f("role")? });
    }
    let col = raw.get("collectors").and_then(|v| v.as_table()).unwrap_or(&empty);
    let mut opts = toml::Table::new();
    for (k, v) in col {
        if v.is_table() {
            opts.insert(k.clone(), v.clone());
        }
    }
    Ok(Config {
        host: s(o, "host").unwrap_or_else(|| "127.0.0.1".into()),
        port: o.get("port").and_then(num).map_or(6742, |x| x as u16),
        fps: r.get("fps").and_then(num).map_or(20, |x| (x as u32).max(1)),
        brightness: r.get("brightness").and_then(num).unwrap_or(0.8),
        zones,
        quiet: s(r, "quiet"),
        preview: s(r, "preview").filter(|x| !x.is_empty()),
        look: look(&raw)?,
        collectors: col
            .get("enabled")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        collector_opts: opts,
    })
}
