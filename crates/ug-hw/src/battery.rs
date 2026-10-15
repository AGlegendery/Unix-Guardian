//! What the battery says about itself, and whether the screen is on.
//!
//! Capacity comes in two dialects: `energy_*` (µWh) from most laptops, and
//! `charge_*` (µAh) from others, which is turned into watt-hours with the
//! design minimum voltage (the convention UPower uses too). Both are the
//! battery's own fuel gauge; it relearns `*_full` on full cycles, so the
//! figure can lag the truth by a few percent.

use crate::{read_trimmed, read_u64, SysRoot};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BatteryInfo {
    /// Capacity when new, Wh.
    pub design_wh: Option<f64>,
    /// Capacity at the last full charge, Wh.
    pub full_wh: Option<f64>,
    pub now_wh: Option<f64>,
    pub cycles: Option<u32>,
    pub manufacturer: String,
    pub model: String,
    pub technology: String,
}

/// The first present battery's capacity figures; `None` without a battery.
pub fn battery_info(root: &SysRoot) -> Option<BatteryInfo> {
    let base = root.join("sys/class/power_supply");
    let mut dirs: Vec<_> = std::fs::read_dir(&base)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();
    dirs.sort();
    let p = dirs.into_iter().find(|p| {
        read_trimmed(p.join("type")).as_deref() == Some("Battery")
            && read_u64(p.join("present")) != Some(0)
            // Mice and headsets report scope Device; the laptop's is System.
            && read_trimmed(p.join("scope")).is_none_or(|s| s == "System")
    })?;
    let wh = |name: &str| read_u64(p.join(name)).map(|v| v as f64 / 1e6);
    let volts = read_u64(p.join("voltage_min_design"))
        .or_else(|| read_u64(p.join("voltage_now")))
        .map(|v| v as f64 / 1e6);
    let from_charge = |name: &str| Some(read_u64(p.join(name))? as f64 / 1e6 * volts?);
    let pick = |e: &str, c: &str| wh(e).or_else(|| from_charge(c)).filter(|v| *v > 0.0);
    Some(BatteryInfo {
        design_wh: pick("energy_full_design", "charge_full_design"),
        full_wh: pick("energy_full", "charge_full"),
        now_wh: wh("energy_now").or_else(|| from_charge("charge_now")),
        // Many firmwares report 0 for "not counted".
        cycles: read_u64(p.join("cycle_count"))
            .filter(|c| *c > 0)
            .map(|c| c as u32),
        manufacturer: read_trimmed(p.join("manufacturer")).unwrap_or_default(),
        model: read_trimmed(p.join("model_name")).unwrap_or_default(),
        technology: read_trimmed(p.join("technology")).unwrap_or_default(),
    })
}

/// Whether any display is lit: a connector driving a CRTC whose DPMS state
/// is On. Atomic drivers keep `dpms` in step with the CRTC's active state,
/// so a compositor blanking the screen reads as off. Reads attributes the
/// kernel caches; never probes a connector or wakes a GPU. `None` when
/// there is no DRM display at all (a server, a VM without KMS).
pub fn screen_on(root: &SysRoot) -> Option<bool> {
    let mut seen = false;
    for e in std::fs::read_dir(root.join("sys/class/drm")).ok()?.flatten() {
        let p = e.path();
        let Some(enabled) = read_trimmed(p.join("enabled")) else {
            continue;
        };
        seen = true;
        if enabled == "enabled" && read_trimmed(p.join("dpms")).as_deref() == Some("On") {
            return Some(true);
        }
    }
    seen.then_some(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    #[test]
    fn energy_figures_are_read_in_watt_hours() {
        let fs = TestFs::new();
        let b = "sys/class/power_supply/BAT1";
        fs.write(format!("{b}/type"), "Battery")
            .write(format!("{b}/present"), "1")
            .write(format!("{b}/energy_full_design"), "60000000")
            .write(format!("{b}/energy_full"), "53310000")
            .write(format!("{b}/energy_now"), "40000000")
            .write(format!("{b}/cycle_count"), "646")
            .write(format!("{b}/manufacturer"), "Sunwoda");
        let i = battery_info(&SysRoot::at(fs.root())).unwrap();
        assert_eq!(i.design_wh, Some(60.0));
        assert_eq!(i.full_wh, Some(53.31));
        assert_eq!(i.cycles, Some(646));
        assert_eq!(i.manufacturer, "Sunwoda");
    }

    #[test]
    fn charge_figures_use_the_design_voltage_and_zero_cycles_mean_unknown() {
        let fs = TestFs::new();
        let b = "sys/class/power_supply/BAT0";
        fs.write(format!("{b}/type"), "Battery")
            .write(format!("{b}/charge_full_design"), "4000000")
            .write(format!("{b}/charge_full"), "3000000")
            .write(format!("{b}/voltage_min_design"), "11400000")
            .write(format!("{b}/cycle_count"), "0");
        let i = battery_info(&SysRoot::at(fs.root())).unwrap();
        assert!((i.design_wh.unwrap() - 45.6).abs() < 1e-9);
        assert!((i.full_wh.unwrap() - 34.2).abs() < 1e-9);
        assert_eq!(i.cycles, None);
    }

    #[test]
    fn a_mouse_battery_is_not_the_laptop_battery() {
        let fs = TestFs::new();
        let b = "sys/class/power_supply/hidpp_battery_0";
        fs.write(format!("{b}/type"), "Battery")
            .write(format!("{b}/scope"), "Device");
        assert!(battery_info(&SysRoot::at(fs.root())).is_none());
    }

    #[test]
    fn the_screen_is_on_when_an_enabled_connector_is_dpms_on() {
        let fs = TestFs::new();
        let c = "sys/class/drm/card2-eDP-2";
        let other = "sys/class/drm/card1-HDMI-A-1";
        fs.write(format!("{other}/enabled"), "disabled")
            .write(format!("{other}/dpms"), "On");
        fs.write(format!("{c}/enabled"), "enabled")
            .write(format!("{c}/dpms"), "Off");
        let root = SysRoot::at(fs.root());
        assert_eq!(screen_on(&root), Some(false), "blanked");
        fs.write(format!("{c}/dpms"), "On");
        assert_eq!(screen_on(&root), Some(true));
        assert_eq!(screen_on(&SysRoot::at(TestFs::new().root())), None);
    }
}
