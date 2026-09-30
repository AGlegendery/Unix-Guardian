//! Reading the machine: CPU, memory, thermals, power, network.
//!
//! Every reader here takes a [`SysRoot`] rather than hard-coding `/`, so the
//! whole layer can be pointed at a fixture tree in tests. That is the only
//! reason this code is testable on a machine that lacks the hardware in
//! question, which is most machines for most of this crate.
//!
//! Nothing here writes. Applying changes lives in the daemon behind polkit;
//! this crate is safe to call from anywhere, at any time, unprivileged.
//!
//! # Missing sensors are normal
//!
//! A laptop has a battery and no motherboard fan chip; a VM has neither; a
//! desktop has both and no `platform_profile`. Readers return `Option` and
//! callers turn that into the sentinel convention at the D-Bus boundary. No
//! reader fails because a file is absent — absence is information, not an error.

pub mod cpu;
pub mod hwmon;
pub mod mem;
pub mod net;
pub mod procnet;
pub mod rapl;
pub mod topology;

use std::path::{Path, PathBuf};

/// Filesystem root the readers resolve `sys/...` and `proc/...` against.
///
/// [`SysRoot::real`] is `/` and is what the daemon uses. Tests pass a
/// `ug-testfs` temporary directory instead.
#[derive(Debug, Clone)]
pub struct SysRoot(PathBuf);

impl Default for SysRoot {
    fn default() -> Self {
        Self::real()
    }
}

impl SysRoot {
    /// The running machine.
    pub fn real() -> Self {
        SysRoot(PathBuf::from("/"))
    }

    /// A fixture tree. `root` is treated as if it were `/`.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        SysRoot(root.into())
    }

    /// Resolves a path relative to this root, e.g. `"sys/class/hwmon"`.
    pub fn join(&self, rel: impl AsRef<Path>) -> PathBuf {
        self.0.join(rel)
    }
}

/// Reads a sysfs file and trims the trailing newline.
///
/// Returns `None` for any failure — missing, unreadable, or (as several sysfs
/// attributes do on unsupported hardware) returning `EINVAL` on read. Callers
/// treat all of those the same way, so distinguishing them would only push the
/// decision outward.
pub(crate) fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    match std::fs::read_to_string(path.as_ref()) {
        Ok(s) => Some(s.trim().to_string()),
        Err(e) => {
            tracing::trace!(path = %path.as_ref().display(), error = %e, "sysfs read failed");
            None
        }
    }
}

/// Reads a sysfs file holding a single integer.
pub(crate) fn read_i64(path: impl AsRef<Path>) -> Option<i64> {
    read_trimmed(path)?.parse().ok()
}

/// Reads a sysfs file holding a single unsigned integer.
pub(crate) fn read_u64(path: impl AsRef<Path>) -> Option<u64> {
    read_trimmed(path)?.parse().ok()
}

/// Parses a kernel CPU list (`"0-3,8,10-11"`) into individual CPU numbers.
///
/// This format appears in `shared_cpu_list`, `cpuset.cpus` and several other
/// places, and gets it wrong in a quiet way if treated as a plain comma list.
pub(crate) fn parse_cpu_list(s: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for part in s.trim().split(',').filter(|p| !p.is_empty()) {
        match part.split_once('-') {
            Some((a, b)) => {
                if let (Ok(a), Ok(b)) = (a.trim().parse::<u32>(), b.trim().parse::<u32>()) {
                    out.extend(a..=b);
                }
            }
            None => {
                if let Ok(v) = part.trim().parse::<u32>() {
                    out.push(v);
                }
            }
        }
    }
    out
}

/// Parses a cache size string as sysfs writes it: `"12288K"`, `"96M"`, `"512"`.
/// Returns KiB.
pub(crate) fn parse_cache_size_kb(s: &str) -> Option<u64> {
    let s = s.trim();
    let (digits, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: u64 = digits.parse().ok()?;
    Some(match unit.trim().to_ascii_uppercase().as_str() {
        "K" | "KB" | "KIB" | "" => n,
        "M" | "MB" | "MIB" => n * 1024,
        "G" | "GB" | "GIB" => n * 1024 * 1024,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_list_handles_ranges_and_singles() {
        assert_eq!(parse_cpu_list("0-3"), vec![0, 1, 2, 3]);
        assert_eq!(parse_cpu_list("0,2,4"), vec![0, 2, 4]);
        assert_eq!(parse_cpu_list("0-1,4,6-7"), vec![0, 1, 4, 6, 7]);
        assert_eq!(parse_cpu_list(""), Vec::<u32>::new());
        assert_eq!(parse_cpu_list("8"), vec![8]);
    }

    #[test]
    fn cpu_list_ignores_garbage_without_losing_the_rest() {
        assert_eq!(parse_cpu_list("0,x,2"), vec![0, 2]);
    }

    #[test]
    fn cache_sizes_carry_their_unit() {
        assert_eq!(parse_cache_size_kb("12288K"), Some(12288));
        assert_eq!(parse_cache_size_kb("96M"), Some(98304));
        assert_eq!(parse_cache_size_kb("512"), Some(512));
        assert_eq!(parse_cache_size_kb("nonsense"), None);
    }
}

/// The whole reader stack against a capture of a real machine.
#[cfg(test)]
mod replay {
    use super::*;

    fn laptop() -> SysRoot {
        SysRoot::at(ug_testfs::fixture("dev-laptop-rtx3050"))
    }

    #[test]
    fn topology_matches_the_captured_cpu() {
        let t = topology::read(&laptop());
        assert_eq!(t.model, "12th Gen Intel(R) Core(TM) i5-12450HX");
        assert_eq!(t.logical, 12);
        assert_eq!(t.physical, 8, "4 P-cores with SMT + 4 E-cores");
        assert!(t.smt);
        assert_eq!(t.l3_domains.len(), 1);
        assert!(!t.has_vcache);
        assert!(topology::preferred_domain(&t).is_none());
        let (cpus, why) = topology::preferred_cpus(&t).expect("Alder Lake-HX is hybrid");
        assert_eq!(why, topology::PreferredReason::PerformanceCores);
        assert_eq!(cpus, (0..8).collect::<Vec<_>>(), "4 P-cores with SMT = CPUs 0-7");
    }

    #[test]
    fn hwmon_finds_the_package_sensor_among_seven_chips() {
        let chips = hwmon::enumerate(&laptop());
        assert_eq!(chips.len(), 7);
        let cpu = hwmon::cpu_chip(&chips).expect("coretemp present");
        assert_eq!(cpu.name, "coretemp");
        let t = cpu.package_temp().expect("package temperature captured");
        assert!((20.0..110.0).contains(&t), "implausible: {t}");
        assert!(chips.iter().any(|c| c.kind == hwmon::ChipKind::Power), "battery/AC classified");
    }

    #[test]
    fn scaling_state_reads_intel_pstate() {
        let st = cpu::scaling_state(&laptop());
        assert_eq!(st.driver.as_deref(), Some("intel_pstate"));
        assert!(st.governor.is_some());
        assert!(st.epp.is_some(), "intel_pstate exposes EPP");
        assert!(st.freq_max_mhz >= st.freq_avg_mhz && st.freq_avg_mhz > 0);
    }

    #[test]
    fn memory_and_network_parse() {
        let m = mem::read(&laptop());
        assert!(m.total_kb > 16 * 1024 * 1024, "machine has ~27 GiB");
        assert!(m.used_pct() > 0.0 && m.used_pct() < 100.0);
        assert_eq!(net::default_iface(&laptop()).as_deref(), Some("wlp8s0"));
    }

    #[test]
    fn rapl_is_present_but_root_only_on_this_kernel() {
        // The capture ran unprivileged, so energy_uj was skipped: this is the
        // shape a non-root reader sees, and it must read as "unavailable".
        let mut r = rapl::PowerReader::new();
        assert_eq!(r.read(&laptop()), None);
        assert!(!r.available());
    }
}
