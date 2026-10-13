//! NVIDIA telemetry via NVML.
//!
//! The NVIDIA kernel modules — proprietary and open alike — register no hwmon
//! chip and publish no useful sysfs attributes, so every reading comes from
//! `libnvidia-ml.so.1`. That library is expensive to keep around, and how it is
//! loaded decides whether this daemon is a good citizen on a laptop:
//!
//! | Measured on an RTX 3050 Laptop GPU  |                                   |
//! |-------------------------------------|-----------------------------------|
//! | NVML loaded                         | +15 MB resident                   |
//! | NVML shut down                      | all of it returned                |
//! | `nvmlInit` while the GPU is asleep  | **wakes it**, 1.6 s               |
//! | `nvmlInit` while the GPU is awake   | 15 ms                             |
//!
//! Reproduce with `cargo run --release -p ug-gpu --example nvml_lifecycle`.
//!
//! So NVML lives in an [`NvmlSlot`] that loads on first use and is released
//! again after a quiet period, and telemetry is only requested for a GPU that is
//! already awake (the caller, [`crate::GpuSet`], checks runtime PM first).
//! Discovery never loads it: the name comes from `pci.ids` and the driver
//! version from `/sys/module/nvidia/version`, neither of which wakes the card.
//! The one path that may wake a sleeping GPU is [`GpuBackend::capabilities`],
//! which only runs when the user asks.
//!
//! # What tuning is actually possible here
//!
//! NVIDIA's Linux driver exposes no voltage/frequency curve. The undervolting
//! that Windows tools do by dragging a V/F curve has no equivalent, and claiming
//! otherwise would be a lie in the UI. What does exist is a power limit and
//! clock offsets, and lowering the former while raising the latter produces the
//! same practical outcome — cooler and quieter at the same performance. The
//! capability flags below say exactly that, and P5 builds the tuning on it.

use crate::{empty_sample, GpuBackend};
use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};
use nvml_wrapper::{Device, Nvml as NvmlLib};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ug_hw::SysRoot;
use ug_proto::{GpuCapabilities, GpuInfo, GpuSample, GpuTune, GpuTuneState};

/// After a failed load, how long before trying again. A broken driver does not
/// fix itself in a second, and a retry costs a library load each time.
const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(60);

/// NVML, loaded on demand and released when idle. Shared by every NVIDIA GPU in
/// the process, since NVML is a process-wide handle.
#[derive(Default)]
pub struct NvmlSlot {
    state: Mutex<SlotState>,
}

#[derive(Default)]
struct SlotState {
    lib: Option<Arc<NvmlLib>>,
    last_used: Option<Instant>,
    failed_at: Option<Instant>,
}

impl NvmlSlot {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Returns NVML, loading it if needed.
    ///
    /// Loading wakes a runtime-suspended GPU; callers on the sampling path must
    /// have checked that the GPU is awake first. `None` means the library is
    /// missing or refused to initialise — the normal state of a machine whose
    /// driver is broken, which the troubleshooter reports on.
    pub fn acquire(&self) -> Option<Arc<NvmlLib>> {
        let mut st = self.lock();
        st.last_used = Some(Instant::now());
        if let Some(lib) = &st.lib {
            return Some(Arc::clone(lib));
        }
        if st
            .failed_at
            .is_some_and(|t| t.elapsed() < RETRY_AFTER_FAILURE)
        {
            return None;
        }
        match NvmlLib::init() {
            Ok(lib) => {
                tracing::debug!("NVML loaded");
                let lib = Arc::new(lib);
                st.lib = Some(Arc::clone(&lib));
                st.failed_at = None;
                Some(lib)
            }
            Err(e) => {
                tracing::info!(error = %e, "NVML unavailable; NVIDIA readings disabled");
                st.failed_at = Some(Instant::now());
                None
            }
        }
    }

    /// Shuts NVML down if nothing has used it for `grace`, returning its memory.
    ///
    /// Returns `None` when NVML is no longer loaded, or `Some(remaining)` — how
    /// long until it will have been idle for `grace` — when it was used too
    /// recently. The caller sleeps exactly that long, so release lands at
    /// `grace` after the last use rather than up to twice that.
    ///
    /// A call already in progress holds its own reference, so NVML is actually
    /// shut down when that call finishes, never underneath it.
    pub fn release_if_idle(&self, grace: Duration) -> Option<Duration> {
        let mut st = self.lock();
        st.lib.as_ref()?;
        let idle = st.last_used.map(|t| t.elapsed()).unwrap_or(grace);
        if idle >= grace {
            st.lib = None;
            tracing::debug!(idle_s = idle.as_secs(), "NVML released after idle period");
            None
        } else {
            Some(grace - idle)
        }
    }

    pub fn loaded(&self) -> bool {
        self.lock().lib.is_some()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SlotState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The NVIDIA kernel module version, read from sysfs without touching the GPU.
///
/// Present for both the proprietary and the open module. Absent means the
/// module is not loaded, which is itself a finding for the troubleshooter.
pub fn module_version(root: &SysRoot) -> Option<String> {
    std::fs::read_to_string(root.join("sys/module/nvidia/version"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Details only NVML can supply, cached from the first time it was loaded so
/// that later inventory calls do not need it again.
#[derive(Clone)]
struct Learned {
    name: String,
    vram_total_mb: u64,
}

/// One NVIDIA GPU.
pub struct NvidiaGpu {
    slot: Arc<NvmlSlot>,
    pci: String,
    learned: Mutex<Option<Learned>>,
}

impl NvidiaGpu {
    /// Binds to the GPU at `info.pci`. Does not load NVML.
    pub fn new(slot: Arc<NvmlSlot>, info: &GpuInfo) -> Self {
        NvidiaGpu {
            slot,
            pci: info.pci.clone(),
            learned: Mutex::new(None),
        }
    }

    /// Runs `f` against this GPU's NVML device, loading NVML if needed.
    fn with_device<T>(&self, f: impl FnOnce(&Device<'_>) -> T) -> Option<T> {
        let lib = self.slot.acquire()?;
        let dev = device_for_pci(&lib, &self.pci)?;
        self.learn(&dev);
        Some(f(&dev))
    }

    /// Records NVML-only details the first time the device is opened.
    fn learn(&self, dev: &Device<'_>) {
        let mut learned = self.learned.lock().unwrap_or_else(|e| e.into_inner());
        if learned.is_some() {
            return;
        }
        *learned = Some(Learned {
            name: dev.name().unwrap_or_default(),
            vram_total_mb: dev
                .memory_info()
                .map(|m| m.total / 1024 / 1024)
                .unwrap_or(0),
        });
    }
}

/// Opens the NVML device for a sysfs PCI address.
///
/// NVML writes the domain as eight hex digits (`00000000:01:00.0`) where sysfs
/// writes four; it accepts both on input, but matching by enumeration is kept
/// as a fallback for drivers that are stricter than documented.
fn device_for_pci<'n>(lib: &'n NvmlLib, pci: &str) -> Option<Device<'n>> {
    if let Ok(d) = lib.device_by_pci_bus_id(pci) {
        return Some(d);
    }
    let want = pci_tail(pci);
    let count = lib.device_count().ok()?;
    (0..count)
        .filter_map(|i| lib.device_by_index(i).ok())
        .find(|d| {
            d.pci_info()
                .map(|p| pci_tail(&p.bus_id) == want)
                .unwrap_or(false)
        })
}

/// The `bus:device.function` part of a PCI address, lowercased.
fn pci_tail(addr: &str) -> String {
    let a = addr.trim().to_ascii_lowercase();
    match a.find(':') {
        Some(i) => a[i + 1..].to_string(),
        None => a,
    }
}

impl GpuBackend for NvidiaGpu {
    fn pci(&self) -> &str {
        &self.pci
    }

    fn enrich(&self, info: &mut GpuInfo) {
        if let Some(l) = self
            .learned
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            if !l.name.is_empty() {
                info.name = l.name.clone();
            }
            if l.vram_total_mb > 0 {
                info.vram_total_mb = l.vram_total_mb;
            }
        }
    }

    fn telemetry(&self) -> GpuSample {
        let mut s = empty_sample(&self.pci);
        self.with_device(|dev| {
            if let Ok(t) = dev.temperature(TemperatureSensor::Gpu) {
                s.temp_c = t as f64;
            }
            if let Ok(u) = dev.utilization_rates() {
                s.load_pct = u.gpu as f64;
            }
            if let Ok(mw) = dev.power_usage() {
                s.power_w = mw as f64 / 1000.0;
            }
            if let Ok(mw) = dev.enforced_power_limit() {
                s.power_limit_w = mw as f64 / 1000.0;
            }
            if let Ok(mhz) = dev.clock_info(Clock::Graphics) {
                s.core_clock_mhz = mhz;
            }
            if let Ok(mhz) = dev.clock_info(Clock::Memory) {
                s.mem_clock_mhz = mhz;
            }
            if let Ok(mem) = dev.memory_info() {
                s.vram_used_mb = mem.used / 1024 / 1024;
            }
            // Fan zero only. Laptop GPUs whose fan the EC owns return an error
            // here, which correctly leaves the sentinel in place.
            if let Ok(pct) = dev.fan_speed(0) {
                s.fan_pct = pct as f64;
            }
        });
        s
    }

    fn capabilities(&self) -> GpuCapabilities {
        let mut caps = GpuCapabilities {
            voltage: false,
            notes: vec![
                "NVIDIA's Linux driver exposes no voltage curve, so true undervolting \
                 is not possible. Lowering the power limit while raising the clock \
                 offset achieves the same result."
                    .to_string(),
            ],
            ..Default::default()
        };

        let probed = self.with_device(|dev| {
            match dev.power_management_limit_constraints() {
                Ok(c) => {
                    caps.power_limit = true;
                    caps.power_limit_min_w = c.min_limit as f64 / 1000.0;
                    caps.power_limit_max_w = c.max_limit as f64 / 1000.0;
                }
                Err(e) => caps.notes.push(format!(
                    "This GPU does not allow changing its power limit ({e})."
                )),
            }
            // Clock offsets need driver 520 or newer; older drivers need an X11
            // session and Coolbits, which P5 handles as a separate path.
            match dev.gpc_clock_vf_offset() {
                Ok(_) => caps.clock_offset = true,
                Err(e) => caps.notes.push(format!(
                    "Clock offsets are unavailable through NVML ({e}). Driver 520 or newer \
                     is required; older drivers need nvidia-settings under X11."
                )),
            }
            caps.fan_control = dev.fan_speed(0).is_ok();
            if !caps.fan_control {
                caps.notes.push(
                    "This GPU's fan is controlled by the system firmware, not the graphics \
                     driver, which is normal on laptops."
                        .to_string(),
                );
            }
        });

        if probed.is_none() {
            caps.notes = vec![
                "The NVIDIA driver's management library (NVML) is not responding, so this \
                 GPU can be listed but not read or tuned. This usually means the driver and \
                 its libraries are out of step, often after an update without a reboot."
                    .to_string(),
            ];
        }
        caps
    }

    fn tune_state(&self) -> GpuTuneState {
        self.tune_state_impl()
    }

    fn apply_tune(&self, t: &GpuTune) -> Result<(), String> {
        self.apply_tune_impl(t)
    }

    fn reset_tune(&self) -> Result<(), String> {
        self.apply_tune_impl(&GpuTune::default())
    }
}

/// The driver's own bounds for a clock offset, read through the raw
/// library: nvml-wrapper has no call for them, and the symbol is missing on
/// drivers before 520 (checked, never assumed: a missing symbol would abort).
fn offset_bounds(dev: &Device<'_>, mem: bool) -> Option<(i32, i32)> {
    let lib = dev.nvml().lib();
    let present = if mem {
        lib.nvmlDeviceGetMemClkMinMaxVfOffset.is_ok()
    } else {
        lib.nvmlDeviceGetGpcClkMinMaxVfOffset.is_ok()
    };
    if !present {
        return None;
    }
    let (mut lo, mut hi): (std::os::raw::c_int, std::os::raw::c_int) = (0, 0);
    // SAFETY: the handle is valid for `dev`'s lifetime and the pointers are to
    // locals; the symbol's presence was checked above.
    let rc = unsafe {
        if mem {
            lib.nvmlDeviceGetMemClkMinMaxVfOffset(dev.handle(), &mut lo, &mut hi)
        } else {
            lib.nvmlDeviceGetGpcClkMinMaxVfOffset(dev.handle(), &mut lo, &mut hi)
        }
    };
    (rc == 0 && lo < hi).then_some((lo, hi))
}

impl NvidiaGpu {
    /// Everything `tune_state` reports, from an open device.
    fn read_tune(&self, dev: &Device<'_>) -> GpuTuneState {
        let mut st = GpuTuneState {
            pci: self.pci.clone(),
            ..Default::default()
        };
        match (
            dev.power_management_limit_constraints(),
            dev.power_management_limit_default(),
            dev.power_management_limit(),
        ) {
            (Ok(c), Ok(def), Ok(cur)) => {
                st.power_limit = true;
                st.power_min_w = c.min_limit as f64 / 1000.0;
                st.power_max_w = c.max_limit as f64 / 1000.0;
                st.power_default_w = def as f64 / 1000.0;
                st.power_limit_w = cur as f64 / 1000.0;
            }
            (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => {
                st.notes.push(format!(
                    "The power limit cannot be read or changed here ({e})."
                ));
            }
        }
        for mem in [false, true] {
            let cur = if mem {
                dev.mem_clock_vf_offset()
            } else {
                dev.gpc_clock_vf_offset()
            };
            match (cur, offset_bounds(dev, mem)) {
                (Ok(v), Some((lo, hi))) => {
                    if mem {
                        (
                            st.mem_offset,
                            st.mem_offset_mhz,
                            st.mem_min_mhz,
                            st.mem_max_mhz,
                        ) = (true, v, lo, hi);
                    } else {
                        (
                            st.core_offset,
                            st.core_offset_mhz,
                            st.core_min_mhz,
                            st.core_max_mhz,
                        ) = (true, v, lo, hi);
                    }
                }
                (Err(e), _) => st.notes.push(format!(
                    "The {} clock offset is unavailable ({e}); it needs driver 520 or newer.",
                    if mem { "memory" } else { "core" }
                )),
                (Ok(_), None) => st.notes.push(format!(
                    "The driver reports no allowed range for the {} clock offset.",
                    if mem { "memory" } else { "core" }
                )),
            }
        }
        st.notes.push(
            "NVIDIA's Linux driver has no voltage curve: a lower power limit with a higher \
             clock offset is how to get cooler and quieter at the same speed."
                .into(),
        );
        st
    }
}

impl NvidiaGpu {
    pub fn tune_state_impl(&self) -> GpuTuneState {
        self.with_device(|dev| self.read_tune(dev)).unwrap_or_else(|| GpuTuneState {
            pci: self.pci.clone(),
            notes: vec!["The NVIDIA management library (NVML) is not responding, so this GPU cannot be tuned.".into()],
            ..Default::default()
        })
    }

    pub fn apply_tune_impl(&self, t: &GpuTune) -> Result<(), String> {
        let lib = self
            .slot
            .acquire()
            .ok_or("the NVIDIA management library (NVML) is not available")?;
        let mut dev = device_for_pci(&lib, &self.pci).ok_or("the GPU was not found by NVML")?;
        let st = self.read_tune(&dev);
        crate::check_tune(t, &st)?;
        let pl = if t.power_limit_w == 0.0 {
            st.power_default_w
        } else {
            t.power_limit_w
        };
        if st.power_limit && (pl - st.power_limit_w).abs() > 0.5 {
            dev.set_power_management_limit((pl * 1000.0).round() as u32)
                .map_err(|e| format!("power limit: {e}"))?;
        }
        if st.core_offset && t.core_offset_mhz != st.core_offset_mhz {
            dev.set_gpc_clock_vf_offset(t.core_offset_mhz)
                .map_err(|e| format!("core clock offset: {e}"))?;
        }
        if st.mem_offset && t.mem_offset_mhz != st.mem_offset_mhz {
            dev.set_mem_clock_vf_offset(t.mem_offset_mhz)
                .map_err(|e| format!("memory clock offset: {e}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nvml_and_sysfs_pci_addresses_compare_equal() {
        assert_eq!(pci_tail("0000:01:00.0"), pci_tail("00000000:01:00.0"));
        assert_eq!(pci_tail("0000:01:00.0"), "01:00.0");
    }

    #[test]
    fn pci_comparison_is_case_insensitive() {
        assert_eq!(pci_tail("0000:0A:00.0"), pci_tail("00000000:0a:00.0"));
    }

    #[test]
    fn distinct_slots_do_not_collide() {
        assert_ne!(pci_tail("0000:01:00.0"), pci_tail("0000:02:00.0"));
    }

    #[test]
    fn an_unloaded_slot_has_nothing_to_release() {
        let slot = NvmlSlot::new();
        assert!(!slot.loaded());
        assert_eq!(slot.release_if_idle(Duration::ZERO), None);
    }

    #[test]
    fn module_version_is_read_from_sysfs() {
        let fs = ug_testfs::TestFs::new();
        fs.write("sys/module/nvidia/version", "550.163.01");
        assert_eq!(
            module_version(&SysRoot::at(fs.root())).as_deref(),
            Some("550.163.01")
        );
    }

    #[test]
    fn a_missing_module_has_no_version() {
        let fs = ug_testfs::TestFs::new();
        assert_eq!(module_version(&SysRoot::at(fs.root())), None);
    }
}
