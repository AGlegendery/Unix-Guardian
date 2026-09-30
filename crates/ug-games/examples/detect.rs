//! Watches the real process table and prints game start/stop events.
//!
//! `cargo run --release -p ug-games --example detect [seconds]`
//!
//! Also reports what each scan costs, since this runs every two seconds for as
//! long as the user is logged in.
use std::time::{Duration, Instant};
use ug_games::{db::GameDb, detect::Detector};
use ug_hw::SysRoot;

fn main() {
    let secs: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(12);
    let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME"));
    let uid = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find_map(|l| l.strip_prefix("Uid:"))?.split_whitespace().next()?.parse().ok())
        .expect("uid");
    let installed = ug_games::scan_installed(&home);
    let mut d = Detector::new(GameDb::bundled(), installed, uid);
    let root = SysRoot::real();
    let procs = std::fs::read_dir("/proc").unwrap().filter(|e| e.as_ref().map(|e| e.file_name().to_string_lossy().parse::<i32>().is_ok()).unwrap_or(false)).count();
    println!("watching {procs} processes for {secs} s");

    let start = Instant::now();
    let mut costs = Vec::new();
    while start.elapsed() < Duration::from_secs(secs) {
        let t = Instant::now();
        let events = d.scan(&root, Instant::now());
        costs.push(t.elapsed());
        for e in events {
            println!("{e:?}");
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    let first = costs[0];
    let rest: Duration = costs[1..].iter().sum::<Duration>() / (costs.len() - 1).max(1) as u32;
    println!("scans: {}  first (classifies everything): {:.2} ms  steady state: {:.3} ms", costs.len(), first.as_secs_f64() * 1e3, rest.as_secs_f64() * 1e3);
    println!("running games at end: {:?}", d.running().iter().map(|g| &g.name).collect::<Vec<_>>());
}
