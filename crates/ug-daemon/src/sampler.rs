//! Assembles one [`Snapshot`] from every reader.
//!
//! The rate-based readers ([`LoadReader`], [`PowerReader`], [`NetReader`]) hold
//! the previous sample, so the sampler owns them for the process lifetime
//! rather than constructing them per call. Two consequences follow, and both
//! are deliberate:
//!
//! - The first snapshot after start-up has zero load and no CPU power. Priming
//!   would mean sleeping inside a D-Bus method call, which is worse.
//! - Two callers asking at once share one set of deltas, so the numbers agree.
//!   That is why the whole sampler sits behind a single lock.

use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::Notify;
use ug_gpu::GpuSet;
use ug_hw::{cpu, hwmon, mem, net, rapl, topology, SysRoot};
use ug_proto::{CpuSample, GpuInfo, Inventory, Snapshot};

/// Everything needed to produce a snapshot, plus the state the rate readers
/// carry between calls.
pub struct Sampler {
    root: SysRoot,
    inner: Mutex<Readers>,
    /// Re-read on every snapshot: hwmon indices are not stable across suspend
    /// or module reload, so caching the chip list would eventually read the
    /// wrong sensor rather than no sensor.
    gpus: GpuSet,
    inventory: Inventory,
    /// Signalled whenever a GPU vendor library may have been loaded, so the
    /// release task knows to start its idle countdown. See [`Self::gpu_activity`].
    gpu_activity: Notify,
}

struct Readers {
    load: cpu::LoadReader,
    power: rapl::PowerReader,
    net: net::NetReader,
}

impl Sampler {
    /// Discovers hardware and builds the readers.
    ///
    /// `privileged` records whether the process has the capabilities its write
    /// paths need; it is reported in the inventory so the UI can show read-only
    /// mode rather than letting calls fail later.
    pub fn new(root: SysRoot, privileged: bool) -> Self {
        let gpus = GpuSet::discover(&root);
        let distro = ug_core::distro::Distro::detect(&root);
        let cpu_topo = topology::read(&root);

        tracing::info!(
            cpu = %cpu_topo.model,
            cores = cpu_topo.logical,
            gpus = gpus.infos().len(),
            distro = %distro.id,
            tier = ?distro.tier,
            privileged,
            "hardware discovered"
        );
        for g in &gpus.infos() {
            tracing::info!(pci = %g.pci, name = %g.name, vendor = %g.vendor, "GPU");
        }

        let inventory = Inventory {
            cpu: cpu_topo,
            gpus: gpus.infos(),
            distro_id: distro.id,
            distro_name: distro.name,
            kernel: kernel_release(&root),
            mem_total_kb: mem::read(&root).total_kb,
            privileged,
        };

        Sampler {
            root,
            inner: Mutex::new(Readers {
                load: cpu::LoadReader::new(),
                power: rapl::PowerReader::new(),
                net: net::NetReader::new(),
            }),
            gpus,
            inventory,
            gpu_activity: Notify::new(),
        }
    }

    /// Static machine description. Cheap; clients still cache it.
    ///
    /// GPU entries are re-read so that details a vendor library supplied after
    /// start-up (an NVIDIA card's VRAM size) appear once known.
    pub fn inventory(&self) -> Inventory {
        Inventory {
            gpus: self.gpus.infos(),
            ..self.inventory.clone()
        }
    }

    /// Every GPU, including ones with no working backend.
    pub fn gpu_infos(&self) -> Vec<GpuInfo> {
        self.gpus.infos()
    }

    /// Looks up a GPU's capabilities by PCI address.
    ///
    /// This is the one call that may wake a sleeping GPU: answering it needs
    /// the vendor library, and it only runs when the user asks.
    pub fn gpu_capabilities(&self, pci: &str) -> Option<ug_proto::GpuCapabilities> {
        let caps = match self.gpus.backend(pci) {
            Some(b) => b.capabilities(),
            None => {
                let infos = self.gpus.infos();
                let info = infos.iter().find(|g| g.pci == pci)?;
                ug_proto::GpuCapabilities {
                    notes: vec![format!(
                        "{} is a display device Unix Guardian has no support for yet.",
                        info.name
                    )],
                    ..Default::default()
                }
            }
        };
        self.gpu_activity.notify_one();
        Some(caps)
    }

    /// Resolves after a call that may have loaded a GPU vendor library.
    pub async fn gpu_activity(&self) {
        self.gpu_activity.notified().await;
    }

    /// Releases GPU vendor libraries unused for `grace`. `None` when none
    /// remain loaded; otherwise how long until the next check is due.
    pub fn release_idle_gpu_libs(&self, grace: Duration) -> Option<Duration> {
        self.gpus.release_idle(grace)
    }

    /// Reads every sensor once.
    pub fn snapshot(&self) -> Snapshot {
        let chips = hwmon::enumerate(&self.root);
        let scaling = cpu::scaling_state(&self.root);

        let mut readers = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let load = readers.load.read(&self.root);
        let cpu_power = readers.power.read(&self.root);
        let net = readers.net.read(&self.root);
        drop(readers);

        let cpu = CpuSample {
            load_pct: load.total_pct,
            temp_c: hwmon::cpu_chip(&chips)
                .and_then(|c| c.package_temp())
                .unwrap_or(f64::NAN),
            power_w: cpu_power.unwrap_or(f64::NAN),
            freq_avg_mhz: scaling.freq_avg_mhz,
            freq_max_mhz: scaling.freq_max_mhz,
            per_core_load: load.per_core_pct,
            governor: scaling.governor.unwrap_or_default(),
            epp: scaling.epp.unwrap_or_default(),
            driver: scaling.driver.unwrap_or_default(),
        };

        let gpus = self.gpus.telemetry();
        self.gpu_activity.notify_one();

        Snapshot {
            ts_unix_ms: now_ms(),
            cpu,
            gpus,
            mem: mem::read(&self.root),
            net,
        }
    }
}

/// `uname -r`, read from procfs so it works against a fixture root.
fn kernel_release(root: &SysRoot) -> String {
    std::fs::read_to_string(root.join("proc/sys/kernel/osrelease"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sampler must survive a machine that exposes nothing at all — a
    /// container, a VM, a fixture root — because refusing to start there would
    /// take the UI down with it.
    #[test]
    fn an_empty_root_still_produces_a_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let s = Sampler::new(SysRoot::at(dir.path()), false);
        let snap = s.snapshot();

        assert_eq!(snap.cpu.load_pct, 0.0);
        assert!(snap.gpus.is_empty());
        assert_eq!(snap.net.iface, "");
        assert!(snap.ts_unix_ms > 0);
        assert!(!s.inventory().privileged);
    }

    #[test]
    fn the_real_machine_reports_a_cpu_temperature() {
        let s = Sampler::new(SysRoot::real(), false);
        let snap = s.snapshot();

        // Every machine this is expected to run on has a package sensor. If
        // this fails on a CI container, the hwmon mapping needs a new name.
        if let Some(t) = snap.cpu.temp() {
            assert!((0.0..150.0).contains(&t), "implausible CPU temperature: {t}");
        }
        assert!(!s.inventory().cpu.model.is_empty(), "CPU model must be readable");
    }
}
