//! Energy used per day, from the CPU package's RAPL counter.
//!
//! RAPL counts microjoules cumulatively, so energy between two reads needs no
//! sampling in between: one read every ten minutes is exact, as long as the
//! counter cannot wrap twice in that time (at the typical 262 kJ range that
//! would take a package drawing over 400 W). That keeps the idle daemon's
//! periodic work to one small file read every ten minutes.
//!
//! Only the CPU package is measured. The GPU is read only while awake and the
//! rest of the machine (screen, disks, board) has no sensor at all, so the
//! totals are labelled for what they are, never passed off as the wall draw.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use ug_hw::SysRoot;

/// Days kept on disk.
const KEEP_DAYS: usize = 90;

#[derive(Default, Serialize, Deserialize)]
struct File {
    /// Local date `YYYY-MM-DD` → watt-hours.
    days: BTreeMap<String, f64>,
}

pub struct EnergyMeter {
    root: SysRoot,
    path: PathBuf,
    days: BTreeMap<String, f64>,
    /// (counter sum, wrap point) at the last read.
    prev: Option<(u64, u64)>,
}

impl EnergyMeter {
    pub fn load(root: SysRoot, path: &Path) -> EnergyMeter {
        let f: File = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        EnergyMeter {
            root,
            path: path.to_path_buf(),
            days: f.days,
            prev: None,
        }
    }

    /// Whether any RAPL package counter is readable here.
    pub fn available(&self) -> bool {
        self.counter().is_some()
    }

    /// Sum of package counters and their summed wrap point.
    fn counter(&self) -> Option<(u64, u64)> {
        let base = self.root.join("sys/class/powercap");
        let mut total = 0u64;
        let mut wrap = 0u64;
        let mut found = false;
        for e in std::fs::read_dir(base).ok()?.filter_map(|e| e.ok()) {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.starts_with("intel-rapl:") || name.matches(':').count() != 1 {
                continue;
            }
            let read = |f: &str| {
                std::fs::read_to_string(e.path().join(f))
                    .ok()?
                    .trim()
                    .parse::<u64>()
                    .ok()
            };
            if let Some(uj) = read("energy_uj") {
                total += uj;
                wrap += read("max_energy_range_uj").unwrap_or(u64::MAX / 4);
                found = true;
            }
        }
        found.then_some((total, wrap))
    }

    /// Reads the counter and books the energy since the last read to `date`.
    /// The first read only sets the baseline. Returns the watt-hours added.
    pub fn tick(&mut self, date: &str) -> f64 {
        let Some((now, wrap)) = self.counter() else {
            return 0.0;
        };
        let added_uj = match self.prev {
            None => 0,
            Some((prev, _)) if now >= prev => now - prev,
            // Wrapped once since the last read.
            Some((prev, _)) if wrap > prev => wrap - prev + now,
            Some(_) => 0,
        };
        self.prev = Some((now, wrap));
        let wh = added_uj as f64 / 3.6e9;
        if wh > 0.0 {
            *self.days.entry(date.to_string()).or_insert(0.0) += wh;
            while self.days.len() > KEEP_DAYS {
                let oldest = self.days.keys().next().cloned();
                if let Some(k) = oldest {
                    self.days.remove(&k);
                }
            }
            if let Err(e) = self.save() {
                tracing::warn!(error = %e, "could not save energy totals");
            }
        }
        wh
    }

    /// Watt-hours per local date, oldest first.
    pub fn days(&self) -> Vec<(String, f64)> {
        self.days.iter().map(|(d, wh)| (d.clone(), *wh)).collect()
    }

    fn save(&self) -> Result<(), String> {
        let text = serde_json::to_string_pretty(&File {
            days: self.days.clone(),
        })
        .map_err(|e| e.to_string())?;
        if let Some(d) = self.path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, text)
            .and_then(|()| std::fs::rename(&tmp, &self.path))
            .map_err(|e| e.to_string())
    }
}

/// Today's local date as `YYYY-MM-DD`.
pub fn local_date() -> String {
    // SAFETY: localtime_r writes only into the struct we pass.
    let tm = unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        tm
    };
    format!(
        "{:04}-{:02}-{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday
    )
}

/// The whole machine's draw while running on battery, in watts: the one
/// honest whole-system figure a laptop offers. `None` on AC or without one.
pub fn battery_discharge_w(root: &SysRoot) -> Option<f64> {
    let base = root.join("sys/class/power_supply");
    for e in std::fs::read_dir(base).ok()?.filter_map(|e| e.ok()) {
        let p = e.path();
        let read = |f: &str| {
            std::fs::read_to_string(p.join(f))
                .ok()
                .map(|s| s.trim().to_string())
        };
        if read("type").as_deref() != Some("Battery")
            || read("status").as_deref() != Some("Discharging")
        {
            continue;
        }
        if let Some(uw) = read("power_now").and_then(|s| s.parse::<f64>().ok()) {
            return Some(uw / 1e6);
        }
        // Some batteries report current and voltage instead.
        let ua = read("current_now").and_then(|s| s.parse::<f64>().ok())?;
        let uv = read("voltage_now").and_then(|s| s.parse::<f64>().ok())?;
        return Some(ua * uv / 1e12);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    fn rapl(fs: &TestFs, uj: u64) {
        fs.write("sys/class/powercap/intel-rapl:0/energy_uj", &uj.to_string());
        fs.write(
            "sys/class/powercap/intel-rapl:0/max_energy_range_uj",
            "1000000000",
        );
        fs.write("sys/class/powercap/intel-rapl:0:0/energy_uj", "999"); // a subdomain: ignored
    }

    #[test]
    fn energy_is_booked_to_the_day_and_survives_a_restart() {
        let fs = TestFs::new();
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("energy.json");
        let mut m = EnergyMeter::load(SysRoot::at(fs.root()), &path);
        rapl(&fs, 0);
        assert_eq!(
            m.tick("2026-09-30"),
            0.0,
            "the first read only sets the baseline"
        );
        rapl(&fs, 360_000_000); // 360 J = 0.1 Wh
        assert!((m.tick("2026-09-30") - 0.1).abs() < 1e-9);
        rapl(&fs, 720_000_000);
        m.tick("2026-10-01");
        let m = EnergyMeter::load(SysRoot::at(fs.root()), &path);
        let days = m.days();
        assert_eq!(days.len(), 2);
        assert!((days[0].1 - 0.1).abs() < 1e-9 && (days[1].1 - 0.1).abs() < 1e-9);
    }

    #[test]
    fn a_wrapped_counter_counts_the_real_energy() {
        let fs = TestFs::new();
        let d = tempfile::tempdir().unwrap();
        let mut m = EnergyMeter::load(SysRoot::at(fs.root()), &d.path().join("e.json"));
        rapl(&fs, 900_000_000);
        m.tick("d");
        rapl(&fs, 260_000_000); // wrapped at 1e9: 100 J + 260 J
        assert!((m.tick("d") - 0.1).abs() < 1e-9);
    }

    #[test]
    fn no_rapl_means_no_numbers_not_zeroes_made_up() {
        let fs = TestFs::new();
        let d = tempfile::tempdir().unwrap();
        let mut m = EnergyMeter::load(SysRoot::at(fs.root()), &d.path().join("e.json"));
        assert!(!m.available());
        assert_eq!(m.tick("d"), 0.0);
        assert!(m.days().is_empty());
    }

    #[test]
    fn battery_draw_is_reported_only_while_discharging() {
        let fs = TestFs::new();
        fs.write("sys/class/power_supply/BAT1/type", "Battery");
        fs.write("sys/class/power_supply/BAT1/status", "Charging");
        fs.write("sys/class/power_supply/BAT1/power_now", "17000000");
        assert_eq!(battery_discharge_w(&SysRoot::at(fs.root())), None);
        fs.write("sys/class/power_supply/BAT1/status", "Discharging");
        assert_eq!(battery_discharge_w(&SysRoot::at(fs.root())), Some(17.0));
        fs.write("sys/class/power_supply/AC/type", "Mains");
    }
}
