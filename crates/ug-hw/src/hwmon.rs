//! hwmon: the kernel's uniform view of temperature, fan, voltage and power chips.
//!
//! Every chip under `/sys/class/hwmon` names itself, and the name is how we tell
//! a CPU package sensor from an NVMe drive from a motherboard Super I/O chip.
//! Those names are not standardised across kernels, so the mapping here is a
//! best-effort classification with an explicit `Other` rather than a guess.

use crate::{read_i64, read_trimmed, SysRoot};
use std::path::PathBuf;

/// What a hwmon chip is attached to, inferred from its `name`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChipKind {
    /// CPU package/core temperatures: `coretemp`, `k10temp`, `zenpower`.
    Cpu,
    /// A discrete or integrated GPU: `amdgpu`, `i915`, `xe`.
    /// NVIDIA is absent by design — its driver registers no hwmon chip, which is
    /// why GPU telemetry on NVIDIA has to come from NVML instead.
    Gpu,
    /// NVMe or SATA drive.
    Storage,
    /// Motherboard Super I/O chip — case and CPU fans live here.
    Motherboard,
    /// Battery, AC adapter, USB-C power supply.
    Power,
    /// Recognised chip we have no use for, or an unknown name.
    Other,
}

impl ChipKind {
    fn classify(name: &str) -> Self {
        match name {
            "coretemp" | "k10temp" | "zenpower" | "k8temp" => ChipKind::Cpu,
            "amdgpu" | "i915" | "xe" | "radeon" | "nouveau" => ChipKind::Gpu,
            "nvme" | "drivetemp" => ChipKind::Storage,
            "BAT0" | "BAT1" | "ACAD" | "AC" => ChipKind::Power,
            n if n.starts_with("nct")
                || n.starts_with("it87")
                || n.starts_with("f71")
                || n.starts_with("w836")
                || n.starts_with("asus") =>
            {
                ChipKind::Motherboard
            }
            n if n.starts_with("ucsi") || n.starts_with("BAT") => ChipKind::Power,
            _ => ChipKind::Other,
        }
    }
}

/// One hwmon chip and the sensors it exposes.
#[derive(Debug, Clone)]
pub struct Chip {
    /// Contents of the chip's `name` file, e.g. `"coretemp"`.
    pub name: String,
    pub kind: ChipKind,
    /// Absolute path to the chip directory.
    pub path: PathBuf,
    pub temps: Vec<Sensor>,
    pub fans: Vec<Sensor>,
    pub powers: Vec<Sensor>,
}

/// A single numbered sensor within a chip.
#[derive(Debug, Clone)]
pub struct Sensor {
    /// The `N` in `tempN_input`.
    pub index: u32,
    /// Contents of `tempN_label` when present, e.g. `"Package id 0"`.
    pub label: Option<String>,
    /// Absolute path to the `_input` file, re-read on every sample.
    pub input_path: PathBuf,
}

impl Sensor {
    /// Reads the raw sysfs value with no unit conversion.
    pub fn raw(&self) -> Option<i64> {
        read_i64(&self.input_path)
    }

    /// Reads a temperature sensor as degrees Celsius.
    pub fn celsius(&self) -> Option<f64> {
        self.raw().map(|v| v as f64 / 1000.0)
    }

    /// Reads a fan sensor as RPM.
    pub fn rpm(&self) -> Option<u32> {
        self.raw().and_then(|v| u32::try_from(v).ok())
    }

    /// Reads a power sensor as watts.
    pub fn watts(&self) -> Option<f64> {
        self.raw().map(|v| v as f64 / 1_000_000.0)
    }
}

impl Chip {
    /// Finds a temperature sensor by exact label, e.g. `"Package id 0"`.
    pub fn temp_by_label(&self, label: &str) -> Option<&Sensor> {
        self.temps
            .iter()
            .find(|s| s.label.as_deref() == Some(label))
    }

    /// The package temperature for a CPU chip.
    ///
    /// Intel labels it `Package id 0`; AMD's `k10temp` labels it `Tctl` (the
    /// control temperature the firmware actually regulates against) and may also
    /// offer `Tdie`. Falls back to the lowest-numbered sensor, which is the
    /// package on every chip we have seen.
    pub fn package_temp(&self) -> Option<f64> {
        for label in ["Package id 0", "Tctl", "Tdie", "Composite"] {
            if let Some(s) = self.temp_by_label(label) {
                return s.celsius();
            }
        }
        self.temps.iter().min_by_key(|s| s.index)?.celsius()
    }
}

/// Discovers every hwmon chip under `root`.
///
/// Chips whose `name` file cannot be read are skipped rather than reported: a
/// chip that will not name itself has nothing we can map its sensors onto.
pub fn enumerate(root: &SysRoot) -> Vec<Chip> {
    let base = root.join("sys/class/hwmon");
    let Ok(entries) = std::fs::read_dir(&base) else {
        tracing::debug!(path = %base.display(), "no hwmon directory");
        return Vec::new();
    };

    let mut chips: Vec<Chip> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            let name = read_trimmed(path.join("name"))?;
            let kind = ChipKind::classify(&name);
            Some(Chip {
                temps: collect(&path, "temp"),
                fans: collect(&path, "fan"),
                powers: collect(&path, "power"),
                name,
                kind,
                path,
            })
        })
        .collect();

    // Stable order so repeated snapshots line sensors up the same way.
    chips.sort_by(|a, b| a.path.cmp(&b.path));
    chips
}

/// Collects the numbered sensors of one kind (`temp`, `fan`, `power`) in a chip.
fn collect(chip: &std::path::Path, prefix: &str) -> Vec<Sensor> {
    let Ok(entries) = std::fs::read_dir(chip) else {
        return Vec::new();
    };

    // `power` sensors report as `powerN_average` on GPUs and `powerN_input`
    // elsewhere; prefer average, which is what the hardware actually integrates.
    let suffixes: &[&str] = if prefix == "power" {
        &["_average", "_input"]
    } else {
        &["_input"]
    };

    let mut out: Vec<Sensor> = Vec::new();
    for entry in entries.filter_map(|e| e.ok()) {
        let fname = entry.file_name();
        let Some(fname) = fname.to_str() else { continue };
        let Some(rest) = fname.strip_prefix(prefix) else {
            continue;
        };
        let Some(suffix) = suffixes.iter().find(|s| rest.ends_with(**s)) else {
            continue;
        };
        let Ok(index) = rest.trim_end_matches(suffix).parse::<u32>() else {
            continue;
        };
        if out.iter().any(|s| s.index == index) {
            continue; // already added via a preferred suffix
        }
        out.push(Sensor {
            index,
            label: read_trimmed(chip.join(format!("{prefix}{index}_label"))),
            input_path: entry.path(),
        });
    }
    out.sort_by_key(|s| s.index);
    out
}

/// Finds the chip most likely to hold CPU package temperature.
pub fn cpu_chip(chips: &[Chip]) -> Option<&Chip> {
    chips.iter().find(|c| c.kind == ChipKind::Cpu)
}

/// Finds the hwmon chip belonging to a specific GPU by matching the PCI device
/// directory the chip hangs off.
///
/// A machine can have two GPUs with hwmon chips, so matching on `kind` alone
/// would attribute one card's temperature to the other.
pub fn gpu_chip_for<'a>(chips: &'a [Chip], pci: &str) -> Option<&'a Chip> {
    chips.iter().find(|c| {
        c.kind == ChipKind::Gpu
            && std::fs::canonicalize(c.path.join("device"))
                .map(|p| p.to_string_lossy().contains(pci))
                .unwrap_or(false)
    })
}

/// Total power reported by a chip's first power sensor, in watts.
pub fn chip_watts(chip: &Chip) -> Option<f64> {
    chip.powers.first()?.watts()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    #[test]
    fn classifies_chips_and_reads_package_temp() {
        let fs = TestFs::new();
        fs.hwmon(0, "coretemp")
            .temp(1, "Package id 0", 47_000)
            .temp(2, "Core 0", 45_000);
        fs.hwmon(1, "nvme").temp(1, "Composite", 38_000);

        let chips = enumerate(&SysRoot::at(fs.root()));
        assert_eq!(chips.len(), 2);

        let cpu = cpu_chip(&chips).expect("coretemp classified as CPU");
        assert_eq!(cpu.kind, ChipKind::Cpu);
        assert_eq!(cpu.package_temp(), Some(47.0));
        assert_eq!(cpu.temps.len(), 2);

        let nvme = chips.iter().find(|c| c.name == "nvme").unwrap();
        assert_eq!(nvme.kind, ChipKind::Storage);
    }

    #[test]
    fn package_temp_falls_back_to_lowest_index_when_unlabelled() {
        let fs = TestFs::new();
        fs.hwmon(0, "k10temp").temp_unlabelled(3, 60_000).temp_unlabelled(1, 55_000);

        let chips = enumerate(&SysRoot::at(fs.root()));
        assert_eq!(chips[0].package_temp(), Some(55.0));
    }

    #[test]
    fn amd_package_temp_prefers_tctl() {
        let fs = TestFs::new();
        fs.hwmon(0, "k10temp").temp(1, "Tctl", 61_500).temp(2, "Tccd1", 58_000);

        let chips = enumerate(&SysRoot::at(fs.root()));
        assert_eq!(chips[0].package_temp(), Some(61.5));
    }

    #[test]
    fn missing_hwmon_directory_is_empty_not_an_error() {
        let fs = TestFs::new();
        assert!(enumerate(&SysRoot::at(fs.root())).is_empty());
    }

    #[test]
    fn chip_without_a_name_is_skipped() {
        let fs = TestFs::new();
        fs.dir("sys/class/hwmon/hwmon0");
        fs.hwmon(1, "coretemp").temp(1, "Package id 0", 40_000);

        let chips = enumerate(&SysRoot::at(fs.root()));
        assert_eq!(chips.len(), 1);
        assert_eq!(chips[0].name, "coretemp");
    }

    #[test]
    fn fan_and_power_sensors_convert_units() {
        let fs = TestFs::new();
        fs.hwmon(0, "amdgpu").fan(1, 1450).power(1, 63_000_000);

        let chips = enumerate(&SysRoot::at(fs.root()));
        assert_eq!(chips[0].fans[0].rpm(), Some(1450));
        assert_eq!(chip_watts(&chips[0]), Some(63.0));
    }
}
