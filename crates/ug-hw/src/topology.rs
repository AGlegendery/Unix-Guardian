//! CPU topology, and the L3 layout that decides whether core pinning is worth
//! offering.
//!
//! The interesting case is AMD's chiplet design: cores are grouped into CCDs,
//! each with its own L3, and on X3D parts only one CCD carries the stacked
//! cache. Pinning a game to that CCD is a real, measurable win; pinning it to
//! the wrong one is a loss. The kernel does not label CCDs, so they are derived
//! from L3 cache IDs — cores sharing an L3 are on the same die.

use crate::{parse_cache_size_kb, read_trimmed, read_u64, SysRoot};
use std::collections::BTreeMap;
use ug_proto::{CpuTopology, L3Domain};

/// An L3 of at least this size is 3D V-Cache rather than a normal cache.
///
/// Non-X3D Zen CCDs carry 32 MiB; X3D parts carry 96 MiB or more. The gap is
/// wide enough that a single threshold is safe, and it is checked per domain so
/// asymmetric parts (7950X3D: one 96 MiB CCD, one 32 MiB) are read correctly.
const VCACHE_MIN_KB: u64 = 96 * 1024;

/// Reads the full CPU topology.
pub fn read(root: &SysRoot) -> CpuTopology {
    let cpus = super::cpu::online_cpus(root);
    let l3_domains = l3_domains(root, &cpus);

    CpuTopology {
        model: model_name(root).unwrap_or_default(),
        logical: cpus.len() as u32,
        physical: physical_cores(root, &cpus),
        sockets: socket_count(root, &cpus),
        smt: smt_enabled(root, &cpus),
        has_vcache: l3_domains.iter().any(|d| d.size_kb >= VCACHE_MIN_KB),
        l3_domains,
        p_cores: hybrid_cpus(root, "cpu_core"),
        e_cores: hybrid_cpus(root, "cpu_atom"),
    }
}

/// CPUs of one core type on an Intel hybrid part.
///
/// The kernel registers a separate PMU per core type, each with a `cpus` file;
/// on a non-hybrid CPU neither directory exists and this is empty.
fn hybrid_cpus(root: &SysRoot, pmu: &str) -> Vec<u32> {
    read_trimmed(root.join(format!("sys/devices/{pmu}/cpus")))
        .map(|s| crate::parse_cpu_list(&s))
        .unwrap_or_default()
}

/// First `model name` line from `/proc/cpuinfo`.
fn model_name(root: &SysRoot) -> Option<String> {
    let content = read_trimmed(root.join("proc/cpuinfo"))?;
    content
        .lines()
        .find(|l| l.starts_with("model name"))
        .and_then(|l| l.split_once(':'))
        .map(|(_, v)| v.trim().to_string())
}

/// Groups logical CPUs by the L3 cache they share.
///
/// Walks every CPU's cache indices looking for level 3 rather than assuming
/// `index3`: the index number is not fixed, and assuming it silently produces
/// an empty result on parts that number caches differently.
fn l3_domains(root: &SysRoot, cpus: &[u32]) -> Vec<L3Domain> {
    let mut by_id: BTreeMap<u32, L3Domain> = BTreeMap::new();

    for &cpu in cpus {
        let cache_dir = root.join(format!("sys/devices/system/cpu/cpu{cpu}/cache"));
        let Ok(entries) = std::fs::read_dir(&cache_dir) else {
            continue;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let p = entry.path();
            if read_trimmed(p.join("level")).as_deref() != Some("3") {
                continue;
            }
            let id = read_u64(p.join("id")).unwrap_or(0) as u32;
            let size_kb = read_trimmed(p.join("size"))
                .and_then(|s| parse_cache_size_kb(&s))
                .unwrap_or(0);

            let domain = by_id.entry(id).or_insert_with(|| L3Domain {
                id,
                size_kb,
                cpus: Vec::new(),
            });
            if !domain.cpus.contains(&cpu) {
                domain.cpus.push(cpu);
            }
            break; // one L3 per CPU
        }
    }

    let mut out: Vec<L3Domain> = by_id.into_values().collect();
    for d in &mut out {
        d.cpus.sort_unstable();
    }
    out
}

/// Counts distinct physical cores via `topology/core_cpus_list`, falling back to
/// the older `thread_siblings_list` on kernels that predate it.
fn physical_cores(root: &SysRoot, cpus: &[u32]) -> u32 {
    let mut seen: Vec<Vec<u32>> = Vec::new();
    for &cpu in cpus {
        let topo = root.join(format!("sys/devices/system/cpu/cpu{cpu}/topology"));
        let siblings = read_trimmed(topo.join("core_cpus_list"))
            .or_else(|| read_trimmed(topo.join("thread_siblings_list")))
            .map(|s| crate::parse_cpu_list(&s))
            .unwrap_or_else(|| vec![cpu]);
        if !seen.contains(&siblings) {
            seen.push(siblings);
        }
    }
    if seen.is_empty() {
        cpus.len() as u32
    } else {
        seen.len() as u32
    }
}

/// Counts distinct physical package IDs.
fn socket_count(root: &SysRoot, cpus: &[u32]) -> u32 {
    let mut ids: Vec<i64> = cpus
        .iter()
        .filter_map(|&cpu| {
            read_trimmed(root.join(format!(
                "sys/devices/system/cpu/cpu{cpu}/topology/physical_package_id"
            )))?
            .parse()
            .ok()
        })
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids.len().max(1) as u32
}

/// True when any core has more than one thread.
///
/// Prefers the kernel's own `smt/active` flag, which correctly reports SMT that
/// is present-but-disabled, and only falls back to comparing logical against
/// physical counts when that file is absent.
fn smt_enabled(root: &SysRoot, cpus: &[u32]) -> bool {
    if let Some(v) = read_trimmed(root.join("sys/devices/system/cpu/smt/active")) {
        return v == "1";
    }
    physical_cores(root, cpus) < cpus.len() as u32
}

/// Picks the L3 domain a latency-sensitive workload should be pinned to.
///
/// On an X3D part that is the cache-heavy CCD. On a uniform part every domain is
/// equivalent and there is nothing to choose, so this returns `None` — the UI
/// uses that to disable the control with a reason rather than offer a pin that
/// would do nothing but constrain the scheduler.
pub fn preferred_domain(topo: &CpuTopology) -> Option<&L3Domain> {
    if topo.l3_domains.len() < 2 {
        return None;
    }
    let best = topo.l3_domains.iter().max_by_key(|d| d.size_kb)?;
    let smallest = topo.l3_domains.iter().map(|d| d.size_kb).min()?;
    if best.size_kb > smallest {
        Some(best)
    } else {
        None
    }
}

/// Why a set of CPUs is preferred for games.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreferredReason {
    /// AMD X3D: the CCD carrying the stacked cache.
    VCache,
    /// Intel hybrid: performance cores, excluding efficiency cores.
    PerformanceCores,
}

/// The CPUs a game benefits from being kept on, if the CPU has such a subset.
///
/// This is the one decision behind both "lock to the V-Cache CCD" on AMD and
/// "keep off the E-cores" on Intel; on a uniform CPU there is no such subset
/// and pinning would only take threads away from the game.
pub fn preferred_cpus(topo: &CpuTopology) -> Option<(Vec<u32>, PreferredReason)> {
    if let Some(d) = preferred_domain(topo) {
        return Some((d.cpus.clone(), PreferredReason::VCache));
    }
    if !topo.p_cores.is_empty() && !topo.e_cores.is_empty() {
        return Some((topo.p_cores.clone(), PreferredReason::PerformanceCores));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    /// 7950X3D: two CCDs, only one with stacked cache.
    fn asymmetric_x3d() -> TestFs {
        let fs = TestFs::new();
        fs.write("proc/cpuinfo", "model name\t: AMD Ryzen 9 7950X3D 16-Core Processor");
        for cpu in 0..16 {
            fs.cpu(cpu, "performance", "amd_pstate_epp", 4_200_000);
            fs.cpu_l3(cpu, 0, "96M", "0-15");
        }
        for cpu in 16..32 {
            fs.cpu(cpu, "performance", "amd_pstate_epp", 4_200_000);
            fs.cpu_l3(cpu, 1, "32M", "16-31");
        }
        fs
    }

    #[test]
    fn detects_vcache_on_an_asymmetric_part() {
        let fs = asymmetric_x3d();
        let topo = read(&SysRoot::at(fs.root()));

        assert_eq!(topo.logical, 32);
        assert_eq!(topo.l3_domains.len(), 2);
        assert!(topo.has_vcache);
        assert_eq!(topo.model, "AMD Ryzen 9 7950X3D 16-Core Processor");
    }

    #[test]
    fn preferred_domain_is_the_cache_heavy_ccd() {
        let fs = asymmetric_x3d();
        let topo = read(&SysRoot::at(fs.root()));

        let d = preferred_domain(&topo).expect("asymmetric part has a preference");
        assert_eq!(d.id, 0);
        assert_eq!(d.size_kb, 96 * 1024);
        assert_eq!(d.cpus.len(), 16);
    }

    #[test]
    fn a_uniform_cpu_has_no_preferred_domain() {
        let fs = TestFs::new();
        fs.write("proc/cpuinfo", "model name\t: 12th Gen Intel(R) Core(TM) i5-12450HX");
        for cpu in 0..12 {
            fs.cpu(cpu, "powersave", "intel_pstate", 2_400_000);
            fs.cpu_l3(cpu, 0, "12288K", "0-11");
        }

        let topo = read(&SysRoot::at(fs.root()));
        assert!(!topo.has_vcache);
        assert_eq!(topo.l3_domains.len(), 1);
        assert!(
            preferred_domain(&topo).is_none(),
            "pinning to the only domain would constrain the scheduler for nothing"
        );
        assert!(preferred_cpus(&topo).is_none(), "not hybrid either");
    }

    #[test]
    fn intel_hybrid_prefers_the_performance_cores() {
        let fs = TestFs::new();
        for cpu in 0..12 {
            fs.cpu(cpu, "powersave", "intel_pstate", 2_400_000);
            fs.cpu_l3(cpu, 0, "12288K", "0-11");
        }
        fs.write("sys/devices/cpu_core/cpus", "0-7");
        fs.write("sys/devices/cpu_atom/cpus", "8-11");

        let topo = read(&SysRoot::at(fs.root()));
        assert_eq!(topo.p_cores, (0..8).collect::<Vec<_>>());
        assert_eq!(topo.e_cores, vec![8, 9, 10, 11]);
        let (cpus, why) = preferred_cpus(&topo).unwrap();
        assert_eq!(cpus, (0..8).collect::<Vec<_>>());
        assert_eq!(why, PreferredReason::PerformanceCores);
    }

    #[test]
    fn x3d_prefers_the_cache_ccd_over_anything_else() {
        let fs = asymmetric_x3d();
        let (cpus, why) = preferred_cpus(&read(&SysRoot::at(fs.root()))).unwrap();
        assert_eq!(why, PreferredReason::VCache);
        assert_eq!(cpus, (0..16).collect::<Vec<_>>());
    }

    #[test]
    fn two_equal_ccds_offer_no_preference() {
        let fs = TestFs::new();
        for cpu in 0..8 {
            fs.cpu(cpu, "performance", "amd_pstate", 4_000_000);
            fs.cpu_l3(cpu, 0, "32M", "0-7");
        }
        for cpu in 8..16 {
            fs.cpu(cpu, "performance", "amd_pstate", 4_000_000);
            fs.cpu_l3(cpu, 1, "32M", "8-15");
        }

        let topo = read(&SysRoot::at(fs.root()));
        assert_eq!(topo.l3_domains.len(), 2);
        assert!(!topo.has_vcache);
        assert!(preferred_domain(&topo).is_none());
    }

    #[test]
    fn smt_flag_is_read_from_the_kernel_when_present() {
        let fs = TestFs::new();
        fs.cpu(0, "performance", "amd_pstate", 4_000_000);
        fs.write("sys/devices/system/cpu/smt/active", "1");

        let topo = read(&SysRoot::at(fs.root()));
        assert!(topo.smt);
    }
}
