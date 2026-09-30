//! Disk throughput, battery power, per-core clocks and load average: the
//! readings the live monitor adds to the snapshot.

use crate::{read_trimmed, read_u64, SysRoot};
use std::collections::HashMap;
use std::time::Instant;
use ug_proto::ExtraSample;

/// Turns successive `/proc/diskstats` counters into rates.
#[derive(Debug, Default)]
pub struct ExtraReader {
    prev: Option<(HashMap<String, [u64; 3]>, Instant)>,
}

/// Whole disks only: `nvme0n1`, `sda`, `vda`, `mmcblk0` — not partitions,
/// loop devices, zram or device-mapper volumes, which would double-count.
fn is_whole_disk(root: &SysRoot, name: &str) -> bool {
    if name.starts_with("loop") || name.starts_with("zram") || name.starts_with("ram") || name.starts_with("dm-") {
        return false;
    }
    root.join(format!("sys/block/{name}")).exists()
}

impl ExtraReader {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn read(&mut self, root: &SysRoot, ts_unix_ms: u64) -> ExtraSample {
        let now = Instant::now();
        // sectors read, sectors written, ms doing I/O
        let mut disks: HashMap<String, [u64; 3]> = HashMap::new();
        for line in read_trimmed(root.join("proc/diskstats")).unwrap_or_default().lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 13 || !is_whole_disk(root, f[2]) {
                continue;
            }
            let n = |i: usize| f[i].parse::<u64>().unwrap_or(0);
            disks.insert(f[2].to_string(), [n(5), n(9), n(12)]);
        }
        let (mut rd, mut wr, mut busy) = (0u64, 0u64, 0f64);
        if let Some((prev, at)) = &self.prev {
            let secs = now.duration_since(*at).as_secs_f64();
            if secs > 0.0 {
                for (name, c) in &disks {
                    if let Some(p) = prev.get(name) {
                        rd += ((c[0].saturating_sub(p[0]) * 512) as f64 / secs) as u64;
                        wr += ((c[1].saturating_sub(p[1]) * 512) as f64 / secs) as u64;
                        let b = c[2].saturating_sub(p[2]) as f64 / (secs * 10.0);
                        busy = busy.max(b.min(100.0));
                    }
                }
            }
        }
        self.prev = Some((disks, now));

        let (battery_pct, battery_w, battery_status, on_ac) = battery(root);
        let load_avg_1 = read_trimmed(root.join("proc/loadavg"))
            .and_then(|s| s.split_whitespace().next()?.parse().ok())
            .unwrap_or(f64::NAN);
        let processes = read_trimmed(root.join("proc/loadavg"))
            .and_then(|s| s.split_whitespace().nth(3)?.split('/').nth(1)?.parse().ok())
            .unwrap_or(0);
        ExtraSample {
            ts_unix_ms,
            per_core_mhz: per_core_mhz(root),
            disk_read_bps: rd,
            disk_write_bps: wr,
            disk_busy_pct: busy,
            battery_pct,
            battery_w,
            battery_status,
            on_ac,
            load_avg_1,
            processes,
        }
    }
}

fn per_core_mhz(root: &SysRoot) -> Vec<u32> {
    let mut cpus: Vec<(u32, u32)> = std::fs::read_dir(root.join("sys/devices/system/cpu"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let n: u32 = name.strip_prefix("cpu")?.parse().ok()?;
            let khz = read_u64(e.path().join("cpufreq/scaling_cur_freq")).unwrap_or(0);
            Some((n, (khz / 1000) as u32))
        })
        .collect();
    cpus.sort_unstable();
    cpus.into_iter().map(|(_, m)| m).collect()
}

/// (percent, watts draining, status, on AC) for the first battery.
fn battery(root: &SysRoot) -> (f64, f64, String, bool) {
    let base = root.join("sys/class/power_supply");
    let mut on_ac = false;
    let mut bat = None;
    for e in std::fs::read_dir(&base).into_iter().flatten().flatten() {
        let p = e.path();
        match read_trimmed(p.join("type")).as_deref() {
            Some("Mains") => on_ac |= read_u64(p.join("online")) == Some(1),
            Some("Battery") if bat.is_none() && read_u64(p.join("present")) != Some(0) => bat = Some(p),
            _ => {}
        }
    }
    let Some(p) = bat else { return (f64::NAN, f64::NAN, String::new(), on_ac) };
    let pct = read_u64(p.join("capacity")).map(|v| v as f64).unwrap_or(f64::NAN);
    let status = read_trimmed(p.join("status")).unwrap_or_default();
    // power_now is µW; some batteries report current_now (µA) × voltage_now (µV).
    let uw = read_u64(p.join("power_now")).map(|v| v as f64).or_else(|| {
        let a = read_u64(p.join("current_now"))? as f64;
        let v = read_u64(p.join("voltage_now"))? as f64;
        Some(a * v / 1e6)
    });
    let w = match uw {
        Some(v) => {
            let w = v / 1e6;
            if status == "Charging" {
                -w
            } else {
                w
            }
        }
        None => f64::NAN,
    };
    (pct, w, status, on_ac)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    #[test]
    fn disk_rates_count_whole_disks_once() {
        let fs = TestFs::new();
        fs.dir("sys/block/nvme0n1").dir("sys/block/loop0");
        let stats = |r: u64, w: u64, t: u64| {
            format!(
                " 259 0 nvme0n1 1 0 {r} 0 1 0 {w} 0 0 {t} 0\n 259 2 nvme0n1p2 1 0 {r} 0 1 0 {w} 0 0 {t} 0\n 7 0 loop0 1 0 999999 0 1 0 999999 0 0 0 0"
            )
        };
        fs.write("proc/diskstats", &stats(1000, 2000, 100));
        let root = SysRoot::at(fs.root());
        let mut r = ExtraReader::new();
        assert_eq!(r.read(&root, 0).disk_read_bps, 0);
        std::thread::sleep(std::time::Duration::from_millis(200));
        fs.write("proc/diskstats", &stats(1000 + 2048, 2000 + 4096, 150));
        let s = r.read(&root, 0);
        // 2048 sectors = 1 MiB in ~0.2 s, partitions and loop ignored.
        assert!(s.disk_read_bps > 3_000_000 && s.disk_read_bps < 5_300_000, "{}", s.disk_read_bps);
        assert!((s.disk_write_bps as f64 / s.disk_read_bps as f64 - 2.0).abs() < 0.01);
        assert!(s.disk_busy_pct > 15.0 && s.disk_busy_pct <= 25.0, "{}", s.disk_busy_pct);
    }

    #[test]
    fn battery_power_is_negative_while_charging() {
        let fs = TestFs::new();
        fs.write("sys/class/power_supply/ACAD/type", "Mains").write("sys/class/power_supply/ACAD/online", "1");
        let b = "sys/class/power_supply/BAT1";
        fs.write(format!("{b}/type"), "Battery").write(format!("{b}/capacity"), "80");
        fs.write(format!("{b}/status"), "Charging").write(format!("{b}/power_now"), "12500000");
        let (pct, w, status, ac) = battery(&SysRoot::at(fs.root()));
        assert_eq!((pct, w, status.as_str(), ac), (80.0, -12.5, "Charging", true));
        fs.write(format!("{b}/status"), "Discharging");
        assert_eq!(battery(&SysRoot::at(fs.root())).1, 12.5);
    }

    #[test]
    fn no_battery_reads_as_unknown() {
        let fs = TestFs::new();
        let (pct, w, _, ac) = battery(&SysRoot::at(fs.root()));
        assert!(pct.is_nan() && w.is_nan() && !ac);
    }
}
