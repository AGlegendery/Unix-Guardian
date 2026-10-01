//! GPU discovery, telemetry, and capability probing.
//!
//! Discovery is vendor-neutral and comes from sysfs: every GPU has a DRM node
//! and a PCI identity whether or not we have a backend for it. Telemetry is
//! vendor-specific, because the vendors expose completely different things:
//!
//! - **AMD** publishes everything through `amdgpu` sysfs attributes, including
//!   voltage, and is readable without special libraries.
//! - **NVIDIA** publishes nothing through sysfs — the proprietary and open
//!   kernel modules register no hwmon chip — so every reading comes from NVML,
//!   a shared library that may not be installed.
//! - **Intel** publishes a little through hwmon and `xe`/`i915` sysfs, enough
//!   to monitor and not enough to tune.
//!
//! [`GpuBackend`] is the seam. The daemon holds a boxed backend per GPU and
//! never branches on vendor itself.
//!
//! # Capabilities are per-machine, not per-model
//!
//! The same card reports different [`GpuCapabilities`] depending on driver
//! version, kernel boot parameters and privilege. A backend reports what it can
//! actually do here, with a written reason for anything it cannot, so the UI can
//! disable a control and say why instead of hiding it or letting it fail.

pub mod amd;
pub mod intel;
pub mod names;
pub mod nvidia;
pub mod pci;

use ug_hw::SysRoot;
use ug_proto::{GpuCapabilities, GpuInfo, GpuSample, GpuTune, GpuTuneState, Vendor};

/// Vendor-specific access to one GPU.
///
/// Tuning methods default to "not possible here"; a backend overrides what
/// its hardware and driver allow. They may wake a sleeping GPU, so they run
/// only when the user asks.
pub trait GpuBackend: Send + Sync {
    /// PCI address this backend is bound to.
    fn pci(&self) -> &str;

    /// Current sensor readings. Never fails on missing sensors — unavailable
    /// values come back as the sentinels described in `ug-proto`.
    fn telemetry(&self) -> GpuSample;

    /// What this GPU permits on this machine, with reasons for what it does not.
    fn capabilities(&self) -> GpuCapabilities;

    /// Improves `info` with anything the backend has learned since discovery,
    /// such as the vendor's own name for the card. Must not touch the hardware.
    fn enrich(&self, _info: &mut GpuInfo) {}

    /// Limits and current values of what can be tuned.
    fn tune_state(&self) -> GpuTuneState {
        GpuTuneState {
            pci: self.pci().to_string(),
            notes: vec!["Tuning this GPU is not supported.".into()],
            ..Default::default()
        }
    }

    /// Applies a tuning request; values outside the card's limits are
    /// refused, never clamped silently.
    fn apply_tune(&self, _t: &GpuTune) -> Result<(), String> {
        Err("Tuning this GPU is not supported.".into())
    }

    /// Puts the card back to its own defaults.
    fn reset_tune(&self) -> Result<(), String> {
        Ok(())
    }
}

/// Checks a request against a card's limits, with a sentence per problem.
pub fn check_tune(t: &GpuTune, s: &GpuTuneState) -> Result<(), String> {
    let mut problems = Vec::new();
    if t.power_limit_w != 0.0 {
        if !s.power_limit {
            problems.push("the power limit cannot be changed on this GPU".to_string());
        } else if t.power_limit_w < s.power_min_w || t.power_limit_w > s.power_max_w {
            problems.push(format!("the power limit must be {:.0}–{:.0} W", s.power_min_w, s.power_max_w));
        }
    }
    for (what, v, ok, lo, hi) in [
        ("core clock offset", t.core_offset_mhz, s.core_offset, s.core_min_mhz, s.core_max_mhz),
        ("memory clock offset", t.mem_offset_mhz, s.mem_offset, s.mem_min_mhz, s.mem_max_mhz),
    ] {
        if v != 0 {
            if !ok {
                problems.push(format!("the {what} cannot be changed on this GPU"));
            } else if v < lo || v > hi {
                problems.push(format!("the {what} must be {lo:+}–{hi:+} MHz"));
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!("Not applied: {}.", problems.join("; ")))
    }
}

/// Discovers every GPU and binds a backend to each.
///
/// GPUs with no usable backend still appear in the returned [`GpuInfo`] list —
/// a card we cannot read is still a card the user has, and showing it as
/// unsupported beats pretending it does not exist. `backends` therefore may be
/// shorter than `infos`.
pub struct GpuSet {
    infos: Vec<GpuInfo>,
    backends: Vec<Box<dyn GpuBackend>>,
    root: SysRoot,
    /// Present when the machine has an NVIDIA GPU. Loaded lazily; see
    /// [`nvidia`] for why it must not be loaded eagerly.
    nvml: Option<std::sync::Arc<nvidia::NvmlSlot>>,
}

impl GpuSet {
    /// Enumerates GPUs under `root` and binds a backend to each.
    ///
    /// Never wakes a GPU and never loads NVML: everything here comes from sysfs
    /// and `pci.ids`. Details only a vendor library knows (the NVIDIA card's
    /// VRAM size, say) are filled in by [`GpuSet::infos`] once a later reading
    /// has had a reason to load it.
    pub fn discover(root: &SysRoot) -> Self {
        let mut infos = pci::enumerate(root);
        let mut backends: Vec<Box<dyn GpuBackend>> = Vec::new();

        let nvml = infos
            .iter()
            .any(|g| g.vendor == Vendor::Nvidia)
            .then(nvidia::NvmlSlot::new);

        for info in &mut infos {
            match info.vendor {
                Vendor::Nvidia => {
                    info.driver_version = nvidia::module_version(root).unwrap_or_default();
                    let slot = std::sync::Arc::clone(nvml.as_ref().expect("slot exists for NVIDIA"));
                    backends.push(Box::new(nvidia::NvidiaGpu::new(slot, info)));
                }
                Vendor::Amd => match amd::AmdGpu::open(root, info) {
                    Some(b) => backends.push(Box::new(b)),
                    None => tracing::info!(pci = %info.pci, "amdgpu sysfs not readable"),
                },
                Vendor::Intel => {
                    if let Some(b) = intel::IntelGpu::open(root, info) {
                        backends.push(Box::new(b));
                    }
                }
                Vendor::Unknown => {
                    tracing::debug!(pci = %info.pci, "display device with no backend");
                }
            }
        }

        GpuSet {
            infos,
            backends,
            root: root.clone(),
            nvml,
        }
    }

    /// Every GPU, including ones with no working backend, with whatever the
    /// backends have learned since discovery folded in.
    pub fn infos(&self) -> Vec<GpuInfo> {
        let mut out = self.infos.clone();
        for info in &mut out {
            if let Some(b) = self.backend(&info.pci) {
                b.enrich(info);
            }
        }
        out
    }

    /// Releases vendor libraries unused for `grace`.
    ///
    /// `None` when nothing is left loaded, so the caller can stop checking
    /// until the next use; otherwise how long until the next check is due.
    pub fn release_idle(&self, grace: std::time::Duration) -> Option<std::time::Duration> {
        self.nvml.as_ref()?.release_if_idle(grace)
    }

    /// Telemetry for every GPU that has a backend, in backend order.
    ///
    /// A runtime-suspended GPU is reported as asleep without being queried.
    /// See [`GpuSample::asleep`] for why waking it would be the wrong trade.
    pub fn telemetry(&self) -> Vec<GpuSample> {
        self.backends
            .iter()
            .map(|b| {
                if pci::runtime_suspended(&self.root, b.pci()) {
                    asleep_sample(b.pci())
                } else {
                    b.telemetry()
                }
            })
            .collect()
    }

    /// Finds a backend by PCI address.
    pub fn backend(&self, pci: &str) -> Option<&dyn GpuBackend> {
        self.backends
            .iter()
            .find(|b| b.pci() == pci)
            .map(|b| b.as_ref())
    }

    /// The GPU a game will run on by default.
    ///
    /// On a hybrid laptop the firmware posts the integrated GPU, so `boot_vga`
    /// points at exactly the card the user does *not* want tuned. The first
    /// discrete GPU is the better default, and falls back to the first GPU on a
    /// machine that has only integrated graphics.
    pub fn default_gpu(&self) -> Option<&GpuInfo> {
        self.infos
            .iter()
            .find(|g| !g.boot_vga && matches!(g.vendor, Vendor::Nvidia | Vendor::Amd))
            .or_else(|| self.infos.first())
    }

    /// True when the machine has both an integrated and a discrete GPU, which
    /// is what makes render-offload settings relevant.
    pub fn is_hybrid(&self) -> bool {
        let discrete = self.infos.iter().filter(|g| !g.boot_vga).count();
        let integrated = self.infos.iter().filter(|g| g.boot_vga).count();
        discrete >= 1 && integrated >= 1
    }
}

/// An empty sample, used when a GPU is present but unreadable.
pub(crate) fn empty_sample(pci: &str) -> GpuSample {
    GpuSample {
        pci: pci.to_string(),
        temp_c: f64::NAN,
        temp_hotspot_c: f64::NAN,
        temp_mem_c: f64::NAN,
        load_pct: f64::NAN,
        power_w: f64::NAN,
        power_limit_w: f64::NAN,
        core_clock_mhz: 0,
        mem_clock_mhz: 0,
        vram_used_mb: 0,
        fan_pct: f64::NAN,
        fan_rpm: 0,
        asleep: false,
    }
}

/// The sample for a GPU that is runtime-suspended and was not woken to read.
pub(crate) fn asleep_sample(pci: &str) -> GpuSample {
    GpuSample {
        load_pct: 0.0,
        asleep: true,
        ..empty_sample(pci)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    /// A backend that fails the test if it is ever asked for telemetry — the
    /// stand-in for "this would have woken the GPU".
    struct MustNotWake(String);

    impl GpuBackend for MustNotWake {
        fn pci(&self) -> &str {
            &self.0
        }
        fn telemetry(&self) -> GpuSample {
            panic!("queried a runtime-suspended GPU, which wakes it");
        }
        fn capabilities(&self) -> GpuCapabilities {
            GpuCapabilities::default()
        }
    }

    struct Awake(String);

    impl GpuBackend for Awake {
        fn pci(&self) -> &str {
            &self.0
        }
        fn telemetry(&self) -> GpuSample {
            GpuSample {
                temp_c: 55.0,
                load_pct: 40.0,
                ..empty_sample(&self.0)
            }
        }
        fn capabilities(&self) -> GpuCapabilities {
            GpuCapabilities::default()
        }
    }

    fn set_with(fs: &TestFs, backends: Vec<Box<dyn GpuBackend>>) -> GpuSet {
        GpuSet {
            infos: Vec::new(),
            backends,
            root: SysRoot::at(fs.root()),
            nvml: None,
        }
    }

    #[test]
    fn a_suspended_gpu_is_reported_asleep_without_being_queried() {
        let fs = TestFs::new();
        fs.write("sys/bus/pci/devices/0000:01:00.0/power/runtime_status", "suspended");
        let set = set_with(&fs, vec![Box::new(MustNotWake("0000:01:00.0".into()))]);

        let s = &set.telemetry()[0];
        assert!(s.asleep);
        assert_eq!(s.load(), Some(0.0), "a suspended GPU is idle");
        assert_eq!(s.temp(), None, "not worth waking it to find out");
    }

    #[test]
    fn a_gpu_mid_suspend_is_left_alone_too() {
        let fs = TestFs::new();
        fs.write("sys/bus/pci/devices/0000:01:00.0/power/runtime_status", "suspending");
        let set = set_with(&fs, vec![Box::new(MustNotWake("0000:01:00.0".into()))]);
        assert!(set.telemetry()[0].asleep);
    }

    #[test]
    fn an_active_gpu_is_read_normally() {
        let fs = TestFs::new();
        fs.write("sys/bus/pci/devices/0000:01:00.0/power/runtime_status", "active");
        let set = set_with(&fs, vec![Box::new(Awake("0000:01:00.0".into()))]);

        let s = &set.telemetry()[0];
        assert!(!s.asleep);
        assert_eq!(s.temp(), Some(55.0));
    }

    #[test]
    fn a_desktop_without_runtime_pm_is_read_normally() {
        let fs = TestFs::new(); // no power/ directory at all
        let set = set_with(&fs, vec![Box::new(Awake("0000:01:00.0".into()))]);
        assert!(!set.telemetry()[0].asleep);
    }

    #[test]
    fn only_the_sleeping_gpu_is_skipped_on_a_hybrid_laptop() {
        let fs = TestFs::new();
        fs.write("sys/bus/pci/devices/0000:00:02.0/power/runtime_status", "active");
        fs.write("sys/bus/pci/devices/0000:01:00.0/power/runtime_status", "suspended");
        let set = set_with(
            &fs,
            vec![
                Box::new(Awake("0000:00:02.0".into())),
                Box::new(MustNotWake("0000:01:00.0".into())),
            ],
        );

        let samples = set.telemetry();
        assert!(!samples[0].asleep);
        assert!(samples[1].asleep);
    }
}

/// Discovery against a capture of a real hybrid laptop.
#[cfg(test)]
mod replay {
    use super::*;

    fn laptop() -> SysRoot {
        SysRoot::at(ug_testfs::fixture("dev-laptop-rtx3050"))
    }

    #[test]
    fn both_gpus_are_found_with_drivers_and_roles() {
        let gpus = pci::enumerate(&laptop());
        assert_eq!(gpus.len(), 2);

        assert_eq!(gpus[0].pci, "0000:00:02.0");
        assert_eq!(gpus[0].vendor, Vendor::Intel);
        assert_eq!(gpus[0].driver, "i915");
        assert!(gpus[0].boot_vga, "firmware posts the iGPU");

        assert_eq!(gpus[1].pci, "0000:01:00.0");
        assert_eq!(gpus[1].vendor, Vendor::Nvidia);
        assert_eq!(gpus[1].driver, "nvidia");
        assert!(!gpus[1].boot_vga);
    }

    #[test]
    fn the_default_gpu_is_the_discrete_one_not_the_boot_gpu() {
        let set = GpuSet {
            infos: pci::enumerate(&laptop()),
            backends: Vec::new(),
            root: laptop(),
            nvml: None,
        };
        assert!(set.is_hybrid());
        assert_eq!(set.default_gpu().unwrap().pci, "0000:01:00.0");
    }
}
