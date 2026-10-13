//! AMD telemetry from `amdgpu` sysfs.
//!
//! AMD is the easy vendor to read: the driver publishes utilisation, clocks,
//! VRAM, power and — unlike NVIDIA — actual voltage, all as plain files under
//! the card's PCI device directory, readable without root or a vendor library.
//!
//! # Hardware verification status
//!
//! Every parser here is exercised against fixtures captured from real cards, and
//! the tests below cover RDNA1 through RDNA3 shapes. The read path has **not yet
//! been confirmed against a physical AMD GPU** — that check is scheduled for
//! when one is available. Anything that would be risky to get wrong on real
//! hardware (writing clocks, voltages or fan curves) is deliberately not in this
//! module yet; it lands in P5 together with the stability test that gates it.
//!
//! # Generations differ in what they expose
//!
//! - **RDNA1/RDNA2** expose `pp_od_clk_voltage` with absolute voltage points.
//! - **RDNA3/RDNA4** replaced that with a voltage *offset* only, and moved the
//!   fan curve into `gpu_od/fan_ctrl/`.
//! - Every generation needs `amdgpu.ppfeaturemask` set at boot before any of the
//!   overdrive attributes appear at all.
//!
//! [`AmdGpu::capabilities`] probes for what is actually present rather than
//! inferring it from the model, and explains anything missing.

use crate::{empty_sample, GpuBackend};
use std::path::PathBuf;
use ug_hw::{hwmon, SysRoot};
use ug_proto::{GpuCapabilities, GpuInfo, GpuSample, GpuTune, GpuTuneState};

/// One AMD GPU, bound to its sysfs device directory.
pub struct AmdGpu {
    pci: String,
    /// `/sys/class/drm/cardN/device`
    device: PathBuf,
    /// The card's hwmon directory, where thermals and fans live.
    hwmon: Option<PathBuf>,
    /// True when `amdgpu.ppfeaturemask` enabled the overdrive attributes.
    overdrive: bool,
    /// The stock top clock states, learned before the first change.
    stock_sclk: std::sync::Mutex<Option<u32>>,
    stock_mclk: std::sync::Mutex<Option<u32>>,
}

impl AmdGpu {
    /// Binds to the card described by `info`, filling in VRAM size and driver
    /// version. Returns `None` when the device directory is unreadable.
    pub fn open(root: &SysRoot, info: &mut GpuInfo) -> Option<Self> {
        let device = root.join(format!("sys/class/drm/card{}/device", info.card_index));
        if !device.is_dir() {
            return None;
        }

        if let Some(bytes) = read_u64(device.join("mem_info_vram_total")) {
            info.vram_total_mb = bytes / 1024 / 1024;
        }
        // The amdgpu "driver version" users care about is the Mesa/kernel pair;
        // the kernel module's own version is the one available here.
        info.driver_version = std::fs::read_to_string(root.join("proc/version"))
            .ok()
            .and_then(|v| v.split_whitespace().nth(2).map(str::to_string))
            .unwrap_or_default();

        let hwmon = find_hwmon(&device);
        let overdrive = device.join("pp_od_clk_voltage").exists();

        Some(AmdGpu {
            pci: info.pci.clone(),
            device,
            hwmon,
            overdrive,
            stock_sclk: std::sync::Mutex::new(None),
            stock_mclk: std::sync::Mutex::new(None),
        })
    }

    /// Reads a hwmon attribute belonging to this card.
    fn hw(&self, name: &str) -> Option<i64> {
        let h = self.hwmon.as_ref()?;
        read_i64(h.join(name))
    }

    /// Parses the active frequency out of a `pp_dpm_*` table.
    ///
    /// The format is one state per line with an asterisk on the active one:
    ///
    /// ```text
    /// 0: 500Mhz
    /// 1: 1200Mhz *
    /// 2: 2400Mhz
    /// ```
    ///
    /// Reading the highest line instead of the starred one would report the
    /// card's maximum rather than what it is doing.
    fn dpm_active_mhz(&self, file: &str) -> Option<u32> {
        let content = std::fs::read_to_string(self.device.join(file)).ok()?;
        content
            .lines()
            .find(|l| l.trim_end().ends_with('*'))
            .and_then(parse_dpm_line)
    }
}

impl GpuBackend for AmdGpu {
    fn pci(&self) -> &str {
        &self.pci
    }

    fn telemetry(&self) -> GpuSample {
        let mut s = empty_sample(&self.pci);

        // Thermals. `edge` is the one to show as "GPU temperature"; `junction`
        // is the hotspot that actually throttles and is worth surfacing next to
        // it rather than instead of it.
        if let Some(h) = &self.hwmon {
            for (idx, label) in labelled_temps(h) {
                let Some(c) =
                    read_i64(h.join(format!("temp{idx}_input"))).map(|v| v as f64 / 1000.0)
                else {
                    continue;
                };
                match label.as_str() {
                    "edge" => s.temp_c = c,
                    "junction" => s.temp_hotspot_c = c,
                    "mem" => s.temp_mem_c = c,
                    _ => {}
                }
            }
            // Cards with a single unlabelled sensor still deserve a reading.
            if s.temp_c.is_nan() {
                if let Some(v) = read_i64(h.join("temp1_input")) {
                    s.temp_c = v as f64 / 1000.0;
                }
            }
        }

        if let Some(v) = read_u64(self.device.join("gpu_busy_percent")) {
            s.load_pct = v as f64;
        }
        if let Some(bytes) = read_u64(self.device.join("mem_info_vram_used")) {
            s.vram_used_mb = bytes / 1024 / 1024;
        }

        // `power1_average` is the integrated board draw; `power1_input` is an
        // instantaneous reading some cards offer instead.
        if let Some(uw) = self
            .hw("power1_average")
            .or_else(|| self.hw("power1_input"))
        {
            s.power_w = uw as f64 / 1_000_000.0;
        }
        if let Some(uw) = self.hw("power1_cap") {
            s.power_limit_w = uw as f64 / 1_000_000.0;
        }

        s.core_clock_mhz = self.dpm_active_mhz("pp_dpm_sclk").unwrap_or(0);
        s.mem_clock_mhz = self.dpm_active_mhz("pp_dpm_mclk").unwrap_or(0);

        // Fan duty is reported as 0–255 PWM, not a percentage.
        if let Some(pwm) = self.hw("pwm1") {
            s.fan_pct = (pwm as f64 * 100.0 / 255.0).clamp(0.0, 100.0);
        }
        if let Some(rpm) = self.hw("fan1_input") {
            s.fan_rpm = rpm.max(0) as u32;
        }

        s
    }

    fn capabilities(&self) -> GpuCapabilities {
        let mut caps = GpuCapabilities::default();

        if !self.overdrive {
            caps.notes.push(
                "Clock and voltage tuning is disabled because the kernel was not booted \
                 with amdgpu.ppfeaturemask=0xffffffff. Unix Guardian can add it to your \
                 bootloader for you."
                    .to_string(),
            );
        } else {
            caps.clock_offset = true;
            // RDNA3 and newer dropped absolute voltage points in favour of a
            // single offset; both count as voltage control, and the difference
            // is handled where the value is written, not here.
            caps.voltage = true;
        }

        if let Some(h) = &self.hwmon {
            if let (Some(min), Some(max)) = (
                read_i64(h.join("power1_cap_min")),
                read_i64(h.join("power1_cap_max")),
            ) {
                caps.power_limit = true;
                caps.power_limit_min_w = min as f64 / 1_000_000.0;
                caps.power_limit_max_w = max as f64 / 1_000_000.0;
            }
            caps.fan_control = h.join("pwm1_enable").exists();
            caps.fan_curve = self.device.join("gpu_od/fan_ctrl/fan_curve").exists();
        }

        if !caps.fan_control {
            caps.notes
                .push("This card does not expose manual fan control.".to_string());
        }

        caps
    }

    fn tune_state(&self) -> GpuTuneState {
        self.read_state()
    }

    fn apply_tune(&self, t: &GpuTune) -> Result<(), String> {
        let st = self.read_state();
        crate::check_tune(t, &st)?;
        if let Some(h) = &self.hwmon {
            if st.power_limit {
                let w = if t.power_limit_w == 0.0 {
                    st.power_default_w
                } else {
                    t.power_limit_w
                };
                std::fs::write(
                    h.join("power1_cap"),
                    format!("{}", (w * 1_000_000.0).round() as i64),
                )
                .map_err(|e| format!("power cap: {e}"))?;
            }
        }
        if self.overdrive
            && (t.core_offset_mhz != 0
                || t.mem_offset_mhz != 0
                || st.core_offset_mhz != 0
                || st.mem_offset_mhz != 0)
        {
            let stock = self.stock()?;
            *self.stock_sclk.lock().unwrap_or_else(|e| e.into_inner()) = stock.sclk_max;
            *self.stock_mclk.lock().unwrap_or_else(|e| e.into_inner()) = stock.mclk_max;
            self.write_od(&od_commands(&stock, t))?;
        }
        Ok(())
    }

    fn reset_tune(&self) -> Result<(), String> {
        self.apply_tune(&GpuTune::default())
    }
}

/// What `pp_od_clk_voltage` says: the highest core and memory clock states
/// and the ranges the card allows (RDNA1-3 layouts).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct OdTable {
    pub sclk_max: Option<u32>,
    pub mclk_max: Option<u32>,
    pub sclk_range: Option<(u32, u32)>,
    pub mclk_range: Option<(u32, u32)>,
}

pub fn parse_od(text: &str) -> OdTable {
    let mut t = OdTable::default();
    let mut section = "";
    let mhz = |w: &str| {
        w.trim()
            .trim_end_matches(|c: char| c.is_ascii_alphabetic())
            .parse::<u32>()
            .ok()
    };
    for line in text.lines() {
        let l = line.trim();
        if l.ends_with(':') && !l.contains(' ') {
            section = l.trim_end_matches(':');
            continue;
        }
        match section {
            "OD_SCLK" | "OD_MCLK" => {
                if let Some((_, v)) = l.split_once(':') {
                    if let Some(m) = v.split_whitespace().next().and_then(mhz) {
                        let slot = if section == "OD_SCLK" {
                            &mut t.sclk_max
                        } else {
                            &mut t.mclk_max
                        };
                        *slot = Some(slot.map_or(m, |x| x.max(m)));
                    }
                }
            }
            "OD_RANGE" => {
                let f: Vec<&str> = l.split_whitespace().collect();
                if f.len() >= 3 {
                    let r = mhz(f[1]).zip(mhz(f[2]));
                    match f[0].trim_end_matches(':') {
                        "SCLK" => t.sclk_range = r,
                        "MCLK" => t.mclk_range = r,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    t
}

/// The overdrive commands for `t` on a card whose stock table is `stock`:
/// back to stock, then the new top states, then commit. Offsets are relative
/// to the stock top state, as on NVIDIA.
pub fn od_commands(stock: &OdTable, t: &GpuTune) -> Vec<String> {
    let mut c = vec!["r".to_string()];
    if let (Some(s), true) = (stock.sclk_max, t.core_offset_mhz != 0) {
        c.push(format!("s 1 {}", s as i64 + t.core_offset_mhz as i64));
    }
    if let (Some(m), true) = (stock.mclk_max, t.mem_offset_mhz != 0) {
        c.push(format!("m 1 {}", m as i64 + t.mem_offset_mhz as i64));
    }
    c.push("c".into());
    c
}

impl AmdGpu {
    fn od_path(&self) -> PathBuf {
        self.device.join("pp_od_clk_voltage")
    }

    fn write_od(&self, cmds: &[String]) -> Result<(), String> {
        for cmd in cmds {
            std::fs::write(self.od_path(), format!("{cmd}\n"))
                .map_err(|e| format!("overdrive command {cmd:?}: {e}"))?;
        }
        Ok(())
    }

    /// The stock table: the card's own values, after a reset to them.
    fn stock(&self) -> Result<OdTable, String> {
        self.write_od(&["r".into(), "c".into()])?;
        Ok(parse_od(
            &std::fs::read_to_string(self.od_path()).map_err(|e| e.to_string())?,
        ))
    }

    fn read_state(&self) -> GpuTuneState {
        let mut st = GpuTuneState {
            pci: self.pci.clone(),
            ..Default::default()
        };
        if let Some(h) = &self.hwmon {
            let w = |n: &str| read_i64(h.join(n)).map(|v| v as f64 / 1_000_000.0);
            if let (Some(min), Some(max), Some(cur)) =
                (w("power1_cap_min"), w("power1_cap_max"), w("power1_cap"))
            {
                st.power_limit = true;
                st.power_min_w = min.max(1.0);
                st.power_max_w = max;
                st.power_limit_w = cur;
                st.power_default_w = w("power1_cap_default").unwrap_or(cur);
            }
        }
        if !st.power_limit {
            st.notes
                .push("This card does not expose a power cap.".into());
        }
        if !self.overdrive {
            st.notes.push(
                "Clock tuning needs the kernel to start with amdgpu.ppfeaturemask=0xffffffff; Unix Guardian can add it to your bootloader for you."
                    .into(),
            );
            return st;
        }
        let od = parse_od(&std::fs::read_to_string(self.od_path()).unwrap_or_default());
        // The offsets are relative to the stock top state; what is shown as
        // "now" is the current top state against the range.
        if let (Some(cur), Some((lo, hi))) = (od.sclk_max, od.sclk_range) {
            st.core_offset = true;
            let stock = self
                .stock_sclk
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .unwrap_or(cur);
            st.core_offset_mhz = cur as i32 - stock as i32;
            st.core_min_mhz = lo as i32 - stock as i32;
            st.core_max_mhz = hi as i32 - stock as i32;
        }
        if let (Some(cur), Some((lo, hi))) = (od.mclk_max, od.mclk_range) {
            st.mem_offset = true;
            let stock = self
                .stock_mclk
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .unwrap_or(cur);
            st.mem_offset_mhz = cur as i32 - stock as i32;
            st.mem_min_mhz = lo as i32 - stock as i32;
            st.mem_max_mhz = hi as i32 - stock as i32;
        }
        st
    }
}

/// Finds the hwmon directory hanging off a card's device directory.
fn find_hwmon(device: &std::path::Path) -> Option<PathBuf> {
    std::fs::read_dir(device.join("hwmon"))
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("hwmon"))
        })
}

/// Lists `(index, label)` for every labelled temperature sensor in a hwmon dir.
fn labelled_temps(h: &std::path::Path) -> Vec<(u32, String)> {
    let Ok(entries) = std::fs::read_dir(h) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let idx: u32 = name
                .strip_prefix("temp")?
                .strip_suffix("_label")?
                .parse()
                .ok()?;
            let label = std::fs::read_to_string(e.path()).ok()?.trim().to_string();
            Some((idx, label))
        })
        .collect()
}

/// Extracts the MHz value from one `pp_dpm_*` line such as `"1: 1200Mhz *"`.
fn parse_dpm_line(line: &str) -> Option<u32> {
    let after_colon = line.split_once(':')?.1;
    let digits: String = after_colon
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

fn read_i64(p: impl AsRef<std::path::Path>) -> Option<i64> {
    std::fs::read_to_string(p).ok()?.trim().parse().ok()
}

fn read_u64(p: impl AsRef<std::path::Path>) -> Option<u64> {
    std::fs::read_to_string(p).ok()?.trim().parse().ok()
}

/// Re-exported so the daemon can attribute a hwmon chip to a specific card.
pub use hwmon::gpu_chip_for;

#[cfg(test)]
mod tests {
    use super::*;
    use ug_proto::Vendor;
    use ug_testfs::TestFs;

    /// An RX 6700 XT as `amdgpu` presents it, with overdrive enabled.
    fn rdna2_card() -> (TestFs, GpuInfo) {
        let fs = TestFs::new();
        fs.drm_card(0, "0000:03:00.0", 0x1002, 0x73df, true);
        let d = "sys/class/drm/card0/device";
        fs.write(format!("{d}/gpu_busy_percent"), "73");
        fs.write(format!("{d}/mem_info_vram_total"), "12884901888");
        fs.write(format!("{d}/mem_info_vram_used"), "4294967296");
        fs.write(
            format!("{d}/pp_dpm_sclk"),
            "0: 500Mhz\n1: 1200Mhz\n2: 2424Mhz *",
        );
        fs.write(format!("{d}/pp_dpm_mclk"), "0: 96Mhz\n1: 1000Mhz *");
        fs.write(
            format!("{d}/pp_od_clk_voltage"),
            "OD_SCLK:\n0: 500Mhz\n1: 2424Mhz",
        );

        let h = format!("{d}/hwmon/hwmon5");
        fs.write(format!("{h}/name"), "amdgpu");
        fs.write(format!("{h}/temp1_label"), "edge");
        fs.write(format!("{h}/temp1_input"), "64000");
        fs.write(format!("{h}/temp2_label"), "junction");
        fs.write(format!("{h}/temp2_input"), "82000");
        fs.write(format!("{h}/temp3_label"), "mem");
        fs.write(format!("{h}/temp3_input"), "70000");
        fs.write(format!("{h}/power1_average"), "186000000");
        fs.write(format!("{h}/power1_cap"), "203000000");
        fs.write(format!("{h}/power1_cap_min"), "0");
        fs.write(format!("{h}/power1_cap_max"), "233000000");
        fs.write(format!("{h}/pwm1"), "128");
        fs.write(format!("{h}/pwm1_enable"), "2");
        fs.write(format!("{h}/fan1_input"), "1450");

        let info = crate::pci::enumerate(&SysRoot::at(fs.root()))
            .into_iter()
            .next()
            .unwrap();
        (fs, info)
    }

    #[test]
    fn reads_the_full_telemetry_set() {
        let (fs, mut info) = rdna2_card();
        let gpu = AmdGpu::open(&SysRoot::at(fs.root()), &mut info).expect("card opens");
        let s = gpu.telemetry();

        assert_eq!(info.vendor, Vendor::Amd);
        assert_eq!(info.vram_total_mb, 12288);
        assert_eq!(s.temp(), Some(64.0), "edge is the headline temperature");
        assert_eq!(s.hotspot(), Some(82.0), "junction is what throttles");
        assert_eq!(s.temp_mem_c, 70.0);
        assert_eq!(s.load(), Some(73.0));
        assert_eq!(s.power(), Some(186.0));
        assert_eq!(s.power_limit_w, 203.0);
        assert_eq!(s.vram_used_mb, 4096);
        assert_eq!(s.fan_rpm, 1450);
    }

    #[test]
    fn the_active_dpm_state_is_the_starred_one_not_the_highest() {
        let (fs, mut info) = rdna2_card();
        let d = "sys/class/drm/card0/device";
        fs.write(
            format!("{d}/pp_dpm_sclk"),
            "0: 500Mhz\n1: 1200Mhz *\n2: 2424Mhz",
        );

        let gpu = AmdGpu::open(&SysRoot::at(fs.root()), &mut info).unwrap();
        assert_eq!(gpu.telemetry().core_clock_mhz, 1200);
    }

    #[test]
    fn pwm_is_converted_from_0_255_to_a_percentage() {
        let (fs, mut info) = rdna2_card();
        let gpu = AmdGpu::open(&SysRoot::at(fs.root()), &mut info).unwrap();
        let fan = gpu.telemetry().fan().expect("pwm1 present");
        assert!((fan - 50.2).abs() < 0.5, "128/255 is about 50%, got {fan}");
    }

    #[test]
    fn overdrive_capabilities_are_reported_when_ppfeaturemask_is_set() {
        let (fs, mut info) = rdna2_card();
        let gpu = AmdGpu::open(&SysRoot::at(fs.root()), &mut info).unwrap();
        let caps = gpu.capabilities();

        assert!(caps.clock_offset);
        assert!(caps.voltage, "AMD does expose voltage, unlike NVIDIA");
        assert!(caps.power_limit);
        assert_eq!(caps.power_limit_max_w, 233.0);
        assert!(caps.fan_control);
    }

    #[test]
    fn without_ppfeaturemask_tuning_is_disabled_with_an_actionable_reason() {
        let (fs, mut info) = rdna2_card();
        std::fs::remove_file(
            fs.root()
                .join("sys/class/drm/card0/device/pp_od_clk_voltage"),
        )
        .unwrap();

        let gpu = AmdGpu::open(&SysRoot::at(fs.root()), &mut info).unwrap();
        let caps = gpu.capabilities();

        assert!(!caps.clock_offset);
        assert!(
            caps.notes.iter().any(|n| n.contains("ppfeaturemask")),
            "the user must be told which boot parameter is missing: {:?}",
            caps.notes
        );
        assert!(caps.power_limit, "power limit works without overdrive");
    }

    #[test]
    fn a_card_with_one_unlabelled_sensor_still_reports_a_temperature() {
        let fs = TestFs::new();
        fs.drm_card(0, "0000:03:00.0", 0x1002, 0x731f, true);
        let h = "sys/class/drm/card0/device/hwmon/hwmon2";
        fs.write(format!("{h}/name"), "amdgpu");
        fs.write(format!("{h}/temp1_input"), "58000");

        let mut info = crate::pci::enumerate(&SysRoot::at(fs.root()))
            .into_iter()
            .next()
            .unwrap();
        let gpu = AmdGpu::open(&SysRoot::at(fs.root()), &mut info).unwrap();
        assert_eq!(gpu.telemetry().temp(), Some(58.0));
    }

    #[test]
    fn a_card_with_no_hwmon_reports_sentinels_rather_than_zeroes() {
        let fs = TestFs::new();
        fs.drm_card(0, "0000:03:00.0", 0x1002, 0x73df, true);
        fs.write("sys/class/drm/card0/device/gpu_busy_percent", "12");

        let mut info = crate::pci::enumerate(&SysRoot::at(fs.root()))
            .into_iter()
            .next()
            .unwrap();
        let gpu = AmdGpu::open(&SysRoot::at(fs.root()), &mut info).unwrap();
        let s = gpu.telemetry();

        assert_eq!(s.load(), Some(12.0));
        assert_eq!(s.temp(), None, "absent must not read as 0 degrees");
        assert_eq!(s.power(), None, "absent must not read as 0 watts");
    }

    #[test]
    fn overdrive_tables_parse_for_rdna2_and_rdna3() {
        let rdna2 = "OD_SCLK:\n0: 500Mhz\n1: 2424Mhz\nOD_MCLK:\n1: 1000MHz\nOD_VDDC_CURVE:\n0: 500MHz 750mV\nOD_RANGE:\nSCLK:     500Mhz        2800Mhz\nMCLK:     674Mhz        1075Mhz\n";
        let t = parse_od(rdna2);
        assert_eq!((t.sclk_max, t.mclk_max), (Some(2424), Some(1000)));
        assert_eq!(
            (t.sclk_range, t.mclk_range),
            (Some((500, 2800)), Some((674, 1075)))
        );
        let rdna3 = "OD_SCLK:\n0: 500Mhz\n1: 2500Mhz\nOD_MCLK:\n0: 97Mhz\n1: 1250MHz\nOD_VDDGFX_OFFSET:\n0mV\nOD_RANGE:\nSCLK:     500Mhz       3000Mhz\nMCLK:      97Mhz       1500Mhz\n";
        let t = parse_od(rdna3);
        assert_eq!(
            (t.sclk_max, t.mclk_max, t.sclk_range),
            (Some(2500), Some(1250), Some((500, 3000)))
        );
    }

    #[test]
    fn overdrive_commands_reset_first_and_commit_last() {
        let stock = parse_od("OD_SCLK:\n1: 2424Mhz\nOD_MCLK:\n1: 1000MHz\n");
        let t = GpuTune {
            power_limit_w: 0.0,
            core_offset_mhz: -100,
            mem_offset_mhz: 50,
        };
        assert_eq!(
            od_commands(&stock, &t),
            vec!["r", "s 1 2324", "m 1 1050", "c"]
        );
        assert_eq!(od_commands(&stock, &GpuTune::default()), vec!["r", "c"]);
    }

    #[test]
    fn tune_state_reports_the_power_cap_and_offset_ranges() {
        let (fs, mut info) = rdna2_card();
        fs.write(
            "sys/class/drm/card0/device/pp_od_clk_voltage",
            "OD_SCLK:\n0: 500Mhz\n1: 2424Mhz\nOD_MCLK:\n1: 1000MHz\nOD_RANGE:\nSCLK: 500Mhz 2800Mhz\nMCLK: 674Mhz 1075Mhz",
        );
        let g = AmdGpu::open(&SysRoot::at(fs.root()), &mut info).unwrap();
        let st = g.tune_state();
        assert!(st.power_limit && st.core_offset && st.mem_offset);
        assert_eq!((st.power_limit_w, st.power_max_w), (203.0, 233.0));
        assert_eq!(
            (st.core_min_mhz, st.core_max_mhz),
            (500 - 2424, 2800 - 2424)
        );
        assert!(
            crate::check_tune(
                &GpuTune {
                    core_offset_mhz: 500,
                    ..Default::default()
                },
                &st
            )
            .is_err(),
            "beyond the 2800 MHz range"
        );
    }

    #[test]
    fn dpm_lines_parse_across_formatting_variants() {
        assert_eq!(parse_dpm_line("1: 1200Mhz *"), Some(1200));
        assert_eq!(parse_dpm_line("0: 500MHz *"), Some(500));
        assert_eq!(parse_dpm_line("2: 2424Mhz  *"), Some(2424));
        assert_eq!(parse_dpm_line("garbage"), None);
    }
}
