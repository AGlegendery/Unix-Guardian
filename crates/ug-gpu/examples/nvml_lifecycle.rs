//! Measures what holding NVML open costs, and what releasing it saves.
//!
//! `cargo run --release -p ug-gpu --example nvml_lifecycle`
//!
//! Answers three questions the daemon's NVML lifecycle depends on: how much
//! memory NVML keeps while loaded, whether shutting it down gives that memory
//! back, and whether initialising it wakes a runtime-suspended GPU.

use std::time::Instant;

fn rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS"))?
                .split_whitespace()
                .nth(1)?
                .parse::<f64>()
                .ok()
        })
        .unwrap_or(0.0)
        / 1024.0
}

fn gpu_state() -> String {
    // The discrete GPU on the development laptop; pass another address as argv[1].
    let pci = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "0000:01:00.0".into());
    std::fs::read_to_string(format!("/sys/bus/pci/devices/{pci}/power/runtime_status"))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "n/a".into())
}

fn main() {
    println!(
        "baseline                 RSS {:5.1} MB   dGPU {}",
        rss_mb(),
        gpu_state()
    );

    for round in 1..=2 {
        let before = gpu_state();
        let t = Instant::now();
        let nvml = nvml_wrapper::Nvml::init().expect("NVML init");
        let init_ms = t.elapsed().as_millis();
        println!(
            "round {round}: init           RSS {:5.1} MB   dGPU {before} -> {}   ({init_ms} ms)",
            rss_mb(),
            gpu_state()
        );

        let t = Instant::now();
        let dev = nvml.device_by_index(0).expect("device 0");
        let _ = dev.temperature(nvml_wrapper::enum_wrappers::device::TemperatureSensor::Gpu);
        println!(
            "round {round}: first read     RSS {:5.1} MB   dGPU {}   ({} ms)",
            rss_mb(),
            gpu_state(),
            t.elapsed().as_millis()
        );

        drop(nvml);
        println!(
            "round {round}: shut down      RSS {:5.1} MB   dGPU {}",
            rss_mb(),
            gpu_state()
        );
    }
}
