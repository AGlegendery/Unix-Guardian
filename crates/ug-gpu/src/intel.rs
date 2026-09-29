//! Intel telemetry: monitoring only.
//!
//! `i915` and `xe` expose a frequency and, on discrete Arc cards, a hwmon chip
//! with temperature and power. There is no supported tuning interface, so this
//! backend reports no capabilities rather than offering controls that would
//! fail. Integrated graphics are covered by the CPU package power the RAPL
//! reader already provides.

use crate::{empty_sample, GpuBackend};
use std::path::PathBuf;
use ug_hw::SysRoot;
use ug_proto::{GpuCapabilities, GpuInfo, GpuSample};

/// One Intel GPU, integrated or discrete.
pub struct IntelGpu {
    pci: String,
    device: PathBuf,
    hwmon: Option<PathBuf>,
}

impl IntelGpu {
    /// Binds to the card described by `info`.
    pub fn open(root: &SysRoot, info: &GpuInfo) -> Option<Self> {
        let device = root.join(format!("sys/class/drm/card{}/device", info.card_index));
        if !device.is_dir() {
            return None;
        }
        let hwmon = std::fs::read_dir(device.join("hwmon"))
            .ok()
            .and_then(|mut d| d.next()?.ok().map(|e| e.path()));
        Some(IntelGpu {
            pci: info.pci.clone(),
            device,
            hwmon,
        })
    }
}

impl GpuBackend for IntelGpu {
    fn pci(&self) -> &str {
        &self.pci
    }

    fn telemetry(&self) -> GpuSample {
        let mut s = empty_sample(&self.pci);

        // `xe` and `i915` name the frequency attribute differently and nest it
        // under different directories; try the known shapes in turn.
        for rel in [
            "tile0/gt0/freq0/cur_freq",
            "gt/gt0/rps_cur_freq_mhz",
            "drm/card0/gt_cur_freq_mhz",
        ] {
            if let Some(mhz) = read_u64(self.device.join(rel)) {
                s.core_clock_mhz = mhz as u32;
                break;
            }
        }

        if let Some(h) = &self.hwmon {
            if let Some(v) = read_i64(h.join("temp1_input")) {
                s.temp_c = v as f64 / 1000.0;
            }
            if let Some(uw) = read_i64(h.join("power1_average")) {
                s.power_w = uw as f64 / 1_000_000.0;
            }
        }

        s
    }

    fn capabilities(&self) -> GpuCapabilities {
        GpuCapabilities {
            notes: vec![
                "Intel graphics can be monitored but not tuned: the driver exposes no \
                 supported interface for clocks, power limits or fans."
                    .to_string(),
            ],
            ..Default::default()
        }
    }
}

fn read_i64(p: impl AsRef<std::path::Path>) -> Option<i64> {
    std::fs::read_to_string(p).ok()?.trim().parse().ok()
}

fn read_u64(p: impl AsRef<std::path::Path>) -> Option<u64> {
    std::fs::read_to_string(p).ok()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    #[test]
    fn reports_frequency_and_no_tuning_capability() {
        let fs = TestFs::new();
        fs.drm_card(0, "0000:00:02.0", 0x8086, 0x468b, true);
        fs.write("sys/class/drm/card0/device/gt/gt0/rps_cur_freq_mhz", "1100");

        let info = crate::pci::enumerate(&SysRoot::at(fs.root()))
            .into_iter()
            .next()
            .unwrap();
        let gpu = IntelGpu::open(&SysRoot::at(fs.root()), &info).unwrap();

        assert_eq!(gpu.telemetry().core_clock_mhz, 1100);
        let caps = gpu.capabilities();
        assert!(!caps.power_limit && !caps.clock_offset && !caps.fan_control);
        assert!(!caps.notes.is_empty(), "the UI needs a reason to show");
    }

    #[test]
    fn an_arc_card_reports_temperature_and_power() {
        let fs = TestFs::new();
        fs.drm_card(0, "0000:03:00.0", 0x8086, 0x56a0, false);
        let h = "sys/class/drm/card0/device/hwmon/hwmon3";
        fs.write(format!("{h}/name"), "i915");
        fs.write(format!("{h}/temp1_input"), "52000");
        fs.write(format!("{h}/power1_average"), "95000000");

        let info = crate::pci::enumerate(&SysRoot::at(fs.root()))
            .into_iter()
            .next()
            .unwrap();
        let gpu = IntelGpu::open(&SysRoot::at(fs.root()), &info).unwrap();
        let s = gpu.telemetry();

        assert_eq!(s.temp(), Some(52.0));
        assert_eq!(s.power(), Some(95.0));
    }
}
