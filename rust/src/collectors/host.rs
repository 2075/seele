//! Host and GPU load, every value normalised to 0..1. Port of seele/collectors/host.py.
use std::time::Duration;

use sysinfo::System;
use tracing::warn;

use crate::state::{Sig, State};

/// Locate the CPU die sensor once (k10temp/zenpower/coretemp). A full hwmon scan is slow and contends with OpenRGB.
fn find_cpu_temp_file() -> Option<String> {
    let rd = std::fs::read_dir("/sys/class/hwmon").ok()?;
    let mut dirs: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    dirs.sort();
    for d in dirs {
        if let Ok(n) = std::fs::read_to_string(d.join("name"))
            && matches!(n.trim(), "k10temp" | "zenpower" | "coretemp") {
                return Some(d.join("temp1_input").to_string_lossy().into_owned());
            }
    }
    None
}

fn read_temp(path: &str) -> Option<f64> {
    std::fs::read_to_string(path).ok()?.trim().parse::<i64>().ok().map(|v| v as f64 / 1000.0)
}

/// 35 C -> 0, 90 C -> 1.
pub fn norm_cpu_temp(t: f64) -> f64 { (t - 35.0) / 55.0 }
/// 35 C -> 0, 85 C -> 1.
pub fn norm_gpu_temp(t: f64) -> f64 { (t - 35.0) / 50.0 }

struct Gpu {
    nvml: nvml_wrapper::Nvml,
    plimit: f64,
}

impl Gpu {
    fn new() -> Option<Gpu> {
        let nvml = match nvml_wrapper::Nvml::init() {
            Ok(n) => n,
            Err(e) => { warn!("NVML unavailable: {e}"); return None; }
        };
        let plimit = match nvml.device_by_index(0).and_then(|d| d.enforced_power_limit()) {
            Ok(p) if p > 0 => p as f64,
            Ok(_) => return None,
            Err(e) => { warn!("NVML unavailable: {e}"); return None; }
        };
        Some(Gpu { nvml, plimit })
    }

    fn sample(&self, state: &State) -> Result<(), nvml_wrapper::error::NvmlError> {
        use nvml_wrapper::enum_wrappers::device::TemperatureSensor;
        let h = self.nvml.device_by_index(0)?;
        state.push(Sig::Gpu, h.utilization_rates()?.gpu as f64 / 100.0, 1.5);
        let m = h.memory_info()?;
        state.push(Sig::Vram, m.used as f64 / m.total as f64, 3.0);
        state.push(Sig::GpuPower, h.power_usage()? as f64 / self.plimit, 1.5);
        state.push(Sig::GpuTemp, norm_gpu_temp(h.temperature(TemperatureSensor::Gpu)? as f64), 3.0);
        Ok(())
    }
}

pub async fn run(state: State, opts: toml::Table) {
    let interval = opts.get("interval").and_then(|v| v.as_float().or(v.as_integer().map(|i| i as f64))).unwrap_or(0.5);
    let mut sys = System::new();
    sys.refresh_cpu_usage();
    let temp_file = find_cpu_temp_file();
    let gpu = Gpu::new();
    let mut last_log: Option<std::time::Instant> = None;
    state.set_health("host", crate::state::Health::Ok);
    loop {
        tokio::time::sleep(Duration::from_secs_f64(interval)).await;
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        state.push(Sig::Cpu, sys.global_cpu_usage() as f64 / 100.0, 1.5);
        let total = sys.total_memory();
        if total > 0 {
            let used = total.saturating_sub(sys.available_memory());
            state.push(Sig::Mem, used as f64 / total as f64, 5.0);
        }
        if let Some(t) = temp_file.as_deref().and_then(read_temp) {
            state.push(Sig::CpuTemp, norm_cpu_temp(t), 3.0);
        }
        if let Some(g) = &gpu
            && let Err(e) = g.sample(&state)
                && last_log.is_none_or(|t| t.elapsed().as_secs() > 60) {
                    last_log = Some(std::time::Instant::now());
                    warn!("host gpu sample: {e}");
                }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_norm() {
        assert_eq!(norm_cpu_temp(35.0), 0.0);
        assert_eq!(norm_cpu_temp(90.0), 1.0);
        assert!((norm_gpu_temp(60.0) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn read_temp_parses_millidegrees() {
        let p = std::env::temp_dir().join("seele_host_temp_test");
        std::fs::write(&p, "45500\n").unwrap();
        assert_eq!(read_temp(p.to_str().unwrap()), Some(45.5));
        assert_eq!(read_temp("/nonexistent/x"), None);
    }

    #[tokio::test]
    #[ignore]
    async fn live() {
        let s = State::new();
        let _ = tokio::time::timeout(Duration::from_secs(8), run(s.clone(), toml::Table::new())).await;
        dump_state(&s, "host");
    }
}

#[cfg(test)]
pub fn dump_state(s: &State, name: &str) {
    for sig in Sig::ALL {
        if let Some(x) = s.0.lock().signals.get(&sig) { println!("{} = {:.3}", sig.as_str(), x.value); }
    }
    println!("health[{name}] = {:?}", s.health(name));
}
