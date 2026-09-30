//! Battery protection: stop charging at a limit so a laptop that lives on AC
//! runs from the adapter instead of topping the battery up forever, which is
//! what wears lithium cells fastest.
//!
//! Laptops expose this in different ways, found in this order:
//! - `charge_control_end_threshold` (ThinkPad, ASUS, Dell, Framework, many
//!   others through the kernel's generic battery interface): any limit.
//! - `charge_types` with `Long_Life` (Lenovo IdeaPad and Legion): the vendor's
//!   conservation mode, a fixed limit around 60–80 % set by the firmware.
//! - `conservation_mode` under the `ideapad_acpi` platform device, for older
//!   kernels that do not map it to `charge_types`.
//!
//! Firmware often forgets the setting on reboot or resume, so the daemon keeps
//! the wanted state on disk and applies it again at start and after sleep.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use ug_hw::SysRoot;

#[derive(Debug, Clone, PartialEq)]
pub enum Method {
    /// `charge_control_end_threshold` (and `_start_` when present).
    Threshold { end: PathBuf, start: Option<PathBuf> },
    /// `charge_types`: write `Long_Life` to protect, the previous type to stop.
    ChargeTypes { path: PathBuf },
    /// ideapad_acpi `conservation_mode`: 1 protects, 0 charges fully.
    Conservation { path: PathBuf },
}

impl Method {
    pub fn name(&self) -> &'static str {
        match self {
            Method::Threshold { .. } => "threshold",
            Method::ChargeTypes { .. } => "vendor-long-life",
            Method::Conservation { .. } => "vendor-conservation",
        }
    }

    /// Whether any limit can be chosen, or the firmware decides it.
    pub fn adjustable(&self) -> bool {
        matches!(self, Method::Threshold { .. })
    }
}

/// What the user wants.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case")]
pub enum Care {
    /// Charge to 100 % (the machine's own behaviour).
    Off,
    /// Stop at `limit` % (ignored by vendor modes, whose limit is fixed).
    Protect { limit: u8 },
    /// Charge to 100 % once, then go back to protecting at `limit`.
    FullOnce { limit: u8 },
}

pub struct Battery {
    root: SysRoot,
    state: PathBuf,
    pub method: Option<Method>,
    pub care: Care,
    /// Development run on the real `/sys`: record, never write.
    simulate: bool,
}

fn read(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

impl Battery {
    pub fn new(root: SysRoot, state: &Path, simulate: bool) -> Battery {
        let method = detect(&root);
        let care = read(state).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Care::Off);
        Battery { root, state: state.to_path_buf(), method, care, simulate }
    }

    /// Battery charge (%) and status (`Charging`, `Full`, …), if a battery exists.
    pub fn level(&self) -> Option<(u8, String)> {
        let bat = battery_dir(&self.root)?;
        let cap = read(&bat.join("capacity"))?.parse().ok()?;
        Some((cap, read(&bat.join("status")).unwrap_or_default()))
    }

    /// Changes what is wanted, saves it and applies it.
    pub fn set(&mut self, care: Care) -> Result<(), String> {
        if let Care::Protect { limit } | Care::FullOnce { limit } = care {
            if !(50..=100).contains(&limit) {
                return Err("the limit must be between 50 and 100 %".into());
            }
        }
        if self.method.is_none() && care != Care::Off {
            return Err(unsupported_reason().into());
        }
        self.care = care;
        let text = serde_json::to_string(&care).map_err(|e| e.to_string())?;
        if let Some(d) = self.state.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::write(&self.state, text).map_err(|e| format!("{}: {e}", self.state.display()))?;
        self.apply()
    }

    /// Writes the wanted state to the hardware. Called at start, after
    /// resume, and on changes; harmless to repeat.
    pub fn apply(&mut self) -> Result<(), String> {
        let Some(method) = self.method.clone() else { return Ok(()) };
        // A full charge done: back to protecting.
        if let Care::FullOnce { limit } = self.care {
            if self.level().is_some_and(|(c, s)| c >= 100 || s == "Full") {
                tracing::info!("battery reached full; protection is back on");
                return self.set(Care::Protect { limit });
            }
        }
        let protect = match self.care {
            Care::Off | Care::FullOnce { .. } => None,
            Care::Protect { limit } => Some(limit),
        };
        if self.simulate {
            tracing::warn!(?protect, "development run: battery protection simulated, nothing written");
            return Ok(());
        }
        let write = |p: &Path, v: &str| std::fs::write(p, v).map_err(|e| format!("{}: {e}", p.display()));
        match (&method, protect) {
            (Method::Threshold { end, start }, Some(limit)) => {
                // Start charging again a few percent below the end, so the
                // battery is not cycled by the last percent either.
                if let Some(s) = start {
                    let _ = write(s, &limit.saturating_sub(5).to_string());
                }
                write(end, &limit.to_string())
            }
            (Method::Threshold { end, start }, None) => {
                write(end, "100")?;
                if let Some(s) = start {
                    let _ = write(s, "95");
                }
                Ok(())
            }
            (Method::ChargeTypes { path }, Some(_)) => write(path, "Long_Life"),
            (Method::ChargeTypes { path }, None) => write(path, "Standard"),
            (Method::Conservation { path }, p) => write(path, if p.is_some() { "1" } else { "0" }),
        }
    }
}

pub fn unsupported_reason() -> &'static str {
    "This laptop does not expose a charge limit to Linux (no charge_control_end_threshold, \
     charge_types or conservation_mode). Some vendors add it with a newer kernel or a \
     platform driver."
}

fn battery_dir(root: &SysRoot) -> Option<PathBuf> {
    let base = root.join("sys/class/power_supply");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(base).ok()?.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    dirs.sort();
    dirs.into_iter().find(|p| read(&p.join("type")).as_deref() == Some("Battery"))
}

fn detect(root: &SysRoot) -> Option<Method> {
    if let Some(bat) = battery_dir(root) {
        let end = bat.join("charge_control_end_threshold");
        if end.exists() {
            let start = bat.join("charge_control_start_threshold");
            return Some(Method::Threshold { end, start: start.exists().then_some(start) });
        }
        let types = bat.join("charge_types");
        if read(&types).is_some_and(|t| t.split_whitespace().any(|w| w.trim_matches(['[', ']']) == "Long_Life")) {
            return Some(Method::ChargeTypes { path: types });
        }
    }
    let platform = root.join("sys/bus/platform/devices");
    let dirs = std::fs::read_dir(platform).ok()?;
    dirs.filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("VPC2004"))
        .map(|e| e.path().join("conservation_mode"))
        .find(|p| p.exists())
        .map(|path| Method::Conservation { path })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    const BAT: &str = "sys/class/power_supply/BAT1";

    fn battery(fs: &TestFs, simulate: bool) -> (tempfile::TempDir, Battery) {
        let d = tempfile::tempdir().unwrap();
        let b = Battery::new(SysRoot::at(fs.root()), &d.path().join("battery.json"), simulate);
        (d, b)
    }

    fn base(fs: &TestFs) {
        fs.write(&format!("{BAT}/type"), "Battery");
        fs.write(&format!("{BAT}/capacity"), "70");
        fs.write(&format!("{BAT}/status"), "Charging");
    }

    #[test]
    fn a_threshold_laptop_gets_the_chosen_limit_and_a_start_below_it() {
        let fs = TestFs::new();
        base(&fs);
        fs.write(&format!("{BAT}/charge_control_end_threshold"), "100");
        fs.write(&format!("{BAT}/charge_control_start_threshold"), "95");
        let (_d, mut b) = battery(&fs, false);
        assert!(b.method.as_ref().unwrap().adjustable());
        b.set(Care::Protect { limit: 80 }).unwrap();
        assert_eq!(read(&fs.root().join(format!("{BAT}/charge_control_end_threshold"))).unwrap(), "80");
        assert_eq!(read(&fs.root().join(format!("{BAT}/charge_control_start_threshold"))).unwrap(), "75");
        b.set(Care::Off).unwrap();
        assert_eq!(read(&fs.root().join(format!("{BAT}/charge_control_end_threshold"))).unwrap(), "100");
    }

    #[test]
    fn an_ideapad_uses_long_life_charging() {
        // The development laptop: Lenovo IdeaPad, charge_types "[Fast] Standard Long_Life".
        let fs = TestFs::new();
        base(&fs);
        fs.write(&format!("{BAT}/charge_types"), "[Fast] Standard Long_Life");
        let (_d, mut b) = battery(&fs, false);
        assert_eq!(b.method.as_ref().unwrap().name(), "vendor-long-life");
        assert!(!b.method.as_ref().unwrap().adjustable());
        b.set(Care::Protect { limit: 80 }).unwrap();
        assert_eq!(read(&fs.root().join(format!("{BAT}/charge_types"))).unwrap(), "Long_Life");
    }

    #[test]
    fn older_ideapads_use_conservation_mode() {
        let fs = TestFs::new();
        base(&fs);
        fs.write("sys/bus/platform/devices/VPC2004:00/conservation_mode", "0");
        let (_d, mut b) = battery(&fs, false);
        b.set(Care::Protect { limit: 80 }).unwrap();
        assert_eq!(read(&fs.root().join("sys/bus/platform/devices/VPC2004:00/conservation_mode")).unwrap(), "1");
    }

    #[test]
    fn the_choice_survives_a_restart_and_is_applied_again() {
        let fs = TestFs::new();
        base(&fs);
        fs.write(&format!("{BAT}/charge_control_end_threshold"), "100");
        let d = tempfile::tempdir().unwrap();
        let state = d.path().join("battery.json");
        Battery::new(SysRoot::at(fs.root()), &state, false).set(Care::Protect { limit: 70 }).unwrap();
        // Firmware forgot it across a reboot.
        fs.write(&format!("{BAT}/charge_control_end_threshold"), "100");
        let mut b = Battery::new(SysRoot::at(fs.root()), &state, false);
        assert_eq!(b.care, Care::Protect { limit: 70 });
        b.apply().unwrap();
        assert_eq!(read(&fs.root().join(format!("{BAT}/charge_control_end_threshold"))).unwrap(), "70");
    }

    #[test]
    fn full_once_charges_fully_then_protects_again() {
        let fs = TestFs::new();
        base(&fs);
        fs.write(&format!("{BAT}/charge_control_end_threshold"), "80");
        let (_d, mut b) = battery(&fs, false);
        b.set(Care::FullOnce { limit: 80 }).unwrap();
        assert_eq!(read(&fs.root().join(format!("{BAT}/charge_control_end_threshold"))).unwrap(), "100");
        fs.write(&format!("{BAT}/capacity"), "100");
        fs.write(&format!("{BAT}/status"), "Full");
        b.apply().unwrap();
        assert_eq!(b.care, Care::Protect { limit: 80 });
        assert_eq!(read(&fs.root().join(format!("{BAT}/charge_control_end_threshold"))).unwrap(), "80");
    }

    #[test]
    fn unsupported_laptops_and_bad_limits_are_refused_honestly() {
        let fs = TestFs::new();
        base(&fs);
        let (_d, mut b) = battery(&fs, false);
        assert!(b.method.is_none());
        assert!(b.set(Care::Protect { limit: 80 }).unwrap_err().contains("does not expose"));
        fs.write(&format!("{BAT}/charge_control_end_threshold"), "100");
        let (_d, mut b) = battery(&fs, false);
        assert!(b.set(Care::Protect { limit: 20 }).is_err());
    }

    #[test]
    fn a_development_run_never_writes() {
        let fs = TestFs::new();
        base(&fs);
        fs.write(&format!("{BAT}/charge_control_end_threshold"), "100");
        let (_d, mut b) = battery(&fs, true);
        b.set(Care::Protect { limit: 60 }).unwrap();
        assert_eq!(read(&fs.root().join(format!("{BAT}/charge_control_end_threshold"))).unwrap(), "100");
    }
}
