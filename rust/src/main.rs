//! seele-rgb: machine state through OpenRGB lighting.
//!   seele-rgb run [--dry-run]   start the monitor (dry run = terminal preview only)
//!   seele-rgb identify          light each configured zone in turn
//!   seele-rgb replay <fixture>  deterministic scene replay (parity check)
mod banner;
mod collectors;
mod colors;
mod config;
mod debug_http;
mod install;
mod replay;
mod scene;
mod sinks;
mod state;

use std::time::{Duration, Instant};

use anyhow::Result;
use clap::{Parser, Subcommand};

use config::Config;
use scene::Scene;
use sinks::Sink;
use sinks::terminal::TerminalSink;
use state::{Sig, State};

#[derive(Parser)]
#[command(name = "seele-rgb", disable_version_flag = true, about = "Show machine load through OpenRGB lighting")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
    /// config file (default: ~/.config/seele/rgb.toml)
    #[arg(short, long, global = true)]
    config: Option<String>,
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Start the monitor
    Run {
        /// render to the terminal instead of OpenRGB
        #[arg(long)]
        dry_run: bool,
        /// port for the /state JSON endpoint (127.0.0.1)
        #[arg(long, default_value_t = 6743)]
        state_port: u16,
    },
    /// Light each configured zone in turn
    Identify {
        #[arg(long)]
        dry_run: bool,
        /// seconds per zone
        #[arg(long, default_value_t = 4.0)]
        hold: f64,
    },
    /// Set up modules, udev rules and systemd units, then verify (uses sudo for root steps)
    Install {
        /// print every action without executing
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove the systemd units and udev rules
    Uninstall {
        #[arg(long)]
        dry_run: bool,
    },
    /// Check the installation (no changes)
    Doctor,
    /// Replay a fixture through the scene and print JSON
    Replay { fixture: String },
}

/// Real LED counts (docs/findings/2026-11-12-openrgb-devices.md), used by the dry run.
fn dry_sizes(cfg: &Config) -> Vec<usize> {
    cfg.zones
        .iter()
        .map(|b| {
            if b.device.contains("Smart Device") {
                if b.zone.ends_with('1') { 16 } else { 10 }
            } else if b.device.contains("Kraken") && b.zone.ends_with('2') {
                8
            } else {
                match b.zone.as_str() {
                    "Unknown" => 4,
                    "Aura Mainboard" => 8,
                    "Hue 2 Channel 3" => 1,
                    _ => 8,
                }
            }
        })
        .collect()
}

/// Calls `sinks::openrgb::connect(&Config) -> anyhow::Result<OpenRgbSink>` (OpenRgbSink: Sink).
fn connect_openrgb(cfg: &Config) -> Result<Box<dyn Sink>> {
    Ok(Box::new(sinks::openrgb::connect(cfg)?))
}

fn make_sink(cfg: &Config, dry: bool) -> Result<Box<dyn Sink>> {
    if dry { Ok(Box::new(TerminalSink::new(cfg, dry_sizes(cfg)))) } else { connect_openrgb(cfg) }
}

fn build(cfg: &Config, dry: bool) -> Result<(Box<dyn Sink>, Scene)> {
    let sink = make_sink(cfg, dry)?;
    let scene = Scene::new(cfg.zones.clone(), sink.sizes().to_vec(), cfg.brightness, cfg.quiet.as_deref(), cfg.preview.clone(), cfg.look.clone());
    Ok((sink, scene))
}

fn mtime(path: &str) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn status_line(state: &State) -> String {
    let g = |s| state.get(s);
    let mut health: Vec<String> = state.0.lock().health.iter().map(|(k, v)| format!("{k}:{v:?}").to_lowercase()).collect();
    health.sort();
    format!(
        "cpu {:.2}  gpu {:.2}  vram {:.2}  loc {:.2}/{:.2}  rem {:.2}/{:.2}  busy {:.0}  spend {:.2}  bad {:.2}  health {}",
        g(Sig::Cpu), g(Sig::Gpu), g(Sig::Vram), g(Sig::ReqLocal), g(Sig::TokLocal), g(Sig::ReqRemote), g(Sig::TokRemote),
        g(Sig::LocalBusy), g(Sig::SpendToday), g(Sig::ContainersBad), health.join(",")
    )
}

async fn run(mut cfg: Config, dry: bool, path: &str, state_port: u16) -> Result<()> {
    let state = State::new();
    tokio::spawn(debug_http::serve(state.clone(), "127.0.0.1", state_port));
    tokio::spawn(collectors::host::run(state.clone(), toml::Table::new()));
    for name in cfg.collectors.clone() {
        let opts = cfg.collector_opts.get(&name).and_then(|v| v.as_table()).cloned().unwrap_or_default();
        let s = state.clone();
        match name.as_str() {
            "docker" => tokio::spawn(collectors::docker::run(s, opts)),
            "traefik" => tokio::spawn(collectors::traefik::run(s, opts)),
            "ollama" => tokio::spawn(collectors::ollama::run(s, opts)),
            "litellm" => tokio::spawn(collectors::litellm::run(s, opts)),
            "gateway" => tokio::spawn(collectors::gateway::run(s, opts)),
            "host" => continue,
            other => {
                tracing::warn!("collector {other} unavailable");
                continue;
            }
        };
    }

    let (mut sink, mut scene) = build(&cfg, dry)?;
    let mut last_mtime = mtime(path);
    let t0 = Instant::now();
    let mut last_check = 0.0;
    let mut dt = Duration::from_secs_f64(1.0 / cfg.fps as f64);
    let result = async {
        loop {
            let t = t0.elapsed().as_secs_f64();
            if t - last_check > 2.0 {
                last_check = t;
                let m = mtime(path);
                if m != last_mtime {
                    last_mtime = m;
                    match config::load(path).and_then(|new| {
                        sink.close();
                        build(&new, dry).map(|b| (new, b))
                    }) {
                        Ok((new, (s, sc))) => {
                            dt = Duration::from_secs_f64(1.0 / new.fps as f64);
                            cfg = new;
                            sink = s;
                            scene = sc;
                            tracing::info!("config reloaded from {path}");
                        }
                        Err(e) => tracing::warn!("config reload failed, keeping old config: {e:#}"),
                    }
                }
            }
            let frame = scene.render(&state, t, Some(state::clock()));
            let status = if dry { status_line(&state) } else { String::new() };
            if let Err(e) = sink.show(&frame, &status) {
                tracing::warn!("sink error: {e:#}");
            }
            tokio::time::sleep(dt).await;
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    };
    tokio::select! {
        r = result => { sink.close(); r }
        _ = tokio::signal::ctrl_c() => { sink.close(); Ok(()) }
    }
}

fn identify(cfg: &Config, dry: bool, hold: f64) -> Result<()> {
    let mut sink = make_sink(cfg, dry)?;
    let sizes = sink.sizes().to_vec();
    for (i, b) in cfg.zones.iter().enumerate() {
        let mut frame: sinks::Frame = sizes.iter().enumerate().map(|(j, &n)| (j, vec![(0, 0, 0); n])).collect();
        frame.insert(i, vec![(255, 255, 255); sizes[i]]);
        if !dry {
            println!("[{i}] {} / {}  (role={}, {} LEDs)", b.device, b.zone, b.role, sizes[i]);
        }
        sink.show(&frame, "")?;
        std::thread::sleep(Duration::from_secs_f64(hold));
    }
    sink.close();
    Ok(())
}

fn main() -> Result<()> {
    if std::env::args().skip(1).any(|a| a == "--version" || a == "-V") {
        banner::print(banner::color_ok());
        println!("  seele-rgb {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let cli = Cli::parse();
    let config = cli.config.clone().unwrap_or_else(config::default_path);
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(if cli.verbose { tracing::Level::DEBUG } else { tracing::Level::INFO })
        .init();
    match cli.cmd {
        Cmd::Replay { fixture } => replay::main(&fixture, &config),
        Cmd::Install { dry_run } => install::install(dry_run, &config),
        Cmd::Uninstall { dry_run } => install::uninstall(dry_run),
        Cmd::Doctor => install::doctor(&config),
        Cmd::Identify { dry_run, hold } => {
            config::ensure_default(&config)?;
            identify(&config::load(&config)?, dry_run, hold)
        }
        Cmd::Run { dry_run, state_port } => {
            if banner::color_ok() {
                banner::print(true);
            }
            config::ensure_default(&config)?;
            let cfg = config::load(&config)?;
            tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(run(cfg, dry_run, &config, state_port))
        }
    }
}
