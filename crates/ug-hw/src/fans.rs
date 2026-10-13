//! Fans that can be driven by hand (hwmon `pwmN` with `pwmN_enable`), and
//! the arithmetic of a fan curve.
//!
//! Desktop boards (nct6775, it87, …) and AMD graphics cards expose these;
//! most laptops do not, because the embedded controller owns the fans — the
//! honest answer there is the firmware's own fan modes.

use crate::SysRoot;
use std::path::PathBuf;

/// One fan output that accepts a duty cycle.
#[derive(Debug, Clone, PartialEq)]
pub struct Fan {
    /// Stable id: `<hwmon name>/pwm<N>`, e.g. `amdgpu/pwm1`, `nct6798/pwm2`.
    pub id: String,
    /// The hwmon directory and the output number.
    pub dir: PathBuf,
    pub n: u32,
    /// What it cools, for the label and the default temperature source.
    pub chip: String,
    /// `fan<N>_input`, when the fan reports its speed.
    pub rpm: Option<u32>,
    /// Current duty, 0–255.
    pub pwm: Option<u32>,
    /// `pwmN_enable`: 0 full speed, 1 manual, 2+ automatic (driver/firmware).
    pub enable: Option<u32>,
}

fn read(p: PathBuf) -> Option<u32> {
    std::fs::read_to_string(p).ok()?.trim().parse().ok()
}

/// Every hwmon `pwmN` that has an `_enable` switch.
pub fn discover(root: &SysRoot) -> Vec<Fan> {
    let mut out = Vec::new();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root.join("sys/class/hwmon"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    dirs.sort();
    for d in dirs {
        let chip = std::fs::read_to_string(d.join("name"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        for n in 1..=8u32 {
            if !d.join(format!("pwm{n}")).exists() || !d.join(format!("pwm{n}_enable")).exists() {
                continue;
            }
            out.push(Fan {
                id: format!("{chip}/pwm{n}"),
                dir: d.clone(),
                n,
                chip: chip.clone(),
                rpm: read(d.join(format!("fan{n}_input"))),
                pwm: read(d.join(format!("pwm{n}"))),
                enable: read(d.join(format!("pwm{n}_enable"))),
            });
        }
    }
    out
}

/// The temperature a fan should follow, °C: the hottest labelled sensor of
/// its own chip for a GPU (`junction` first), else the CPU package.
pub fn source_temp(root: &SysRoot, fan: &Fan) -> Option<f64> {
    let milli = |p: PathBuf| read(p).map(|v| v as f64 / 1000.0);
    if fan.chip == "amdgpu" {
        for label in ["junction", "edge"] {
            for i in 1..=8 {
                if std::fs::read_to_string(fan.dir.join(format!("temp{i}_label")))
                    .is_ok_and(|l| l.trim() == label)
                {
                    if let Some(c) = milli(fan.dir.join(format!("temp{i}_input"))) {
                        return Some(c);
                    }
                }
            }
        }
        return milli(fan.dir.join("temp1_input"));
    }
    for name in ["coretemp", "k10temp", "zenpower"] {
        for e in std::fs::read_dir(root.join("sys/class/hwmon"))
            .into_iter()
            .flatten()
            .flatten()
        {
            if std::fs::read_to_string(e.path().join("name")).is_ok_and(|n| n.trim() == name) {
                if let Some(c) = milli(e.path().join("temp1_input")) {
                    return Some(c);
                }
            }
        }
    }
    None
}

/// Duty percent for `temp` on a curve of (°C, %) points, linear between
/// points and flat beyond the ends. Points need not be sorted.
pub fn curve_pct(points: &[(f64, f64)], temp: f64) -> f64 {
    let mut p: Vec<(f64, f64)> = points.to_vec();
    p.sort_by(|a, b| a.0.total_cmp(&b.0));
    let Some(&first) = p.first() else {
        return 100.0;
    };
    let last = *p.last().expect("non-empty");
    if temp <= first.0 {
        return first.1;
    }
    if temp >= last.0 {
        return last.1;
    }
    for w in p.windows(2) {
        let ((t0, d0), (t1, d1)) = (w[0], w[1]);
        if temp <= t1 {
            return if t1 > t0 {
                d0 + (d1 - d0) * (temp - t0) / (t1 - t0)
            } else {
                d1
            };
        }
    }
    last.1
}

/// The next duty for a fan: a rise follows at once, a fall waits until the
/// target is `hysteresis` percentage points lower, so the fan does not hunt
/// up and down around a threshold (the noise users notice most).
pub fn next_pct(current: f64, target: f64, hysteresis: f64) -> f64 {
    if target >= current || current - target >= hysteresis {
        target
    } else {
        current
    }
}

/// Refuses curves that would cook something: at least two points, duty
/// 0–100, and never below 30 % at 80 °C or more.
pub fn validate(points: &[(f64, f64)]) -> Result<(), String> {
    if points.len() < 2 {
        return Err("a fan curve needs at least two points".into());
    }
    if points
        .iter()
        .any(|p| !(0.0..=100.0).contains(&p.1) || !(0.0..=110.0).contains(&p.0))
    {
        return Err("points are 0–110 °C and 0–100 %".into());
    }
    for t in [80.0, 90.0, 100.0] {
        if curve_pct(points, t) < 30.0 {
            return Err(format!(
                "the fan would run below 30 % at {t:.0} °C; that is not allowed"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    #[test]
    fn curves_interpolate_and_stay_flat_beyond_the_ends() {
        let c = [(40.0, 20.0), (60.0, 40.0), (80.0, 100.0)];
        assert_eq!(curve_pct(&c, 30.0), 20.0);
        assert_eq!(curve_pct(&c, 50.0), 30.0);
        assert_eq!(curve_pct(&c, 70.0), 70.0);
        assert_eq!(curve_pct(&c, 95.0), 100.0);
        assert_eq!(
            curve_pct(&[(80.0, 100.0), (40.0, 20.0)], 60.0),
            60.0,
            "order does not matter"
        );
    }

    #[test]
    fn falls_wait_for_the_hysteresis_rises_do_not() {
        assert_eq!(next_pct(50.0, 60.0, 5.0), 60.0);
        assert_eq!(next_pct(50.0, 47.0, 5.0), 50.0);
        assert_eq!(next_pct(50.0, 44.0, 5.0), 44.0);
    }

    #[test]
    fn dangerous_curves_are_refused() {
        assert!(validate(&[(40.0, 20.0), (90.0, 100.0)]).is_ok());
        assert!(
            validate(&[(40.0, 0.0), (100.0, 20.0)]).is_err(),
            "20 % at 100 °C"
        );
        assert!(validate(&[(40.0, 20.0)]).is_err());
        assert!(validate(&[(40.0, 20.0), (90.0, 150.0)]).is_err());
    }

    #[test]
    fn discovers_driveable_fans_only() {
        let fs = TestFs::new();
        let g = "sys/class/hwmon/hwmon3";
        fs.write(format!("{g}/name"), "amdgpu")
            .write(format!("{g}/pwm1"), "102")
            .write(format!("{g}/pwm1_enable"), "2");
        fs.write(format!("{g}/fan1_input"), "1450")
            .write(format!("{g}/temp2_label"), "junction")
            .write(format!("{g}/temp2_input"), "71000");
        fs.write("sys/class/hwmon/hwmon4/name", "acpitz")
            .write("sys/class/hwmon/hwmon4/temp1_input", "50000");
        fs.write("sys/class/hwmon/hwmon5/name", "nct6798")
            .write("sys/class/hwmon/hwmon5/pwm2", "80");
        let root = SysRoot::at(fs.root());
        let fans = discover(&root);
        assert_eq!(
            fans.len(),
            1,
            "pwm2 without _enable cannot be switched to manual"
        );
        assert_eq!(fans[0].id, "amdgpu/pwm1");
        assert_eq!(
            (fans[0].rpm, fans[0].pwm, fans[0].enable),
            (Some(1450), Some(102), Some(2))
        );
        assert_eq!(source_temp(&root, &fans[0]), Some(71.0));
    }
}
