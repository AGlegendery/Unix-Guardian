//! CPU load, frequency, and the scaling knobs the game profile will later change.
//!
//! Load is a rate, not a level: `/proc/stat` gives monotonically increasing jiffy
//! counters, so a single read says nothing. [`LoadReader`] holds the previous
//! counters and turns two reads into a percentage. The first call after
//! construction has no previous sample and reports zero rather than a spike.

use crate::{read_trimmed, read_u64, SysRoot};

/// Cumulative jiffy counters for one CPU line in `/proc/stat`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Jiffies {
    /// Everything except idle and iowait.
    busy: u64,
    /// idle + iowait.
    idle: u64,
}

impl Jiffies {
    /// Parses one `cpu`/`cpuN` line. Kernels have added fields over time, so
    /// this sums whatever is present rather than requiring a fixed count.
    fn parse(line: &str) -> Option<Self> {
        let mut fields = line.split_whitespace();
        fields.next()?; // the "cpu" / "cpuN" label
        let vals: Vec<u64> = fields.filter_map(|f| f.parse().ok()).collect();
        if vals.len() < 4 {
            return None;
        }
        // user nice system idle iowait irq softirq steal guest guest_nice
        let idle = vals[3] + vals.get(4).copied().unwrap_or(0);
        let busy: u64 = vals.iter().sum::<u64>() - idle;
        Some(Jiffies { busy, idle })
    }

    /// Busy percentage between two counter reads.
    ///
    /// Returns `0.0` when the counters did not move — that is a genuinely idle
    /// interval, and dividing by the zero delta would otherwise produce `NaN`.
    fn delta_pct(now: Self, prev: Self) -> f64 {
        let busy = now.busy.saturating_sub(prev.busy);
        let idle = now.idle.saturating_sub(prev.idle);
        let total = busy + idle;
        if total == 0 {
            return 0.0;
        }
        busy as f64 * 100.0 / total as f64
    }
}

/// Turns successive `/proc/stat` reads into load percentages.
///
/// Holds one sample of state, so it must be kept alive across the sampling loop
/// rather than constructed per read.
#[derive(Debug, Default)]
pub struct LoadReader {
    prev_total: Option<Jiffies>,
    prev_per_core: Vec<Jiffies>,
}

/// Aggregate and per-core load, 0–100.
#[derive(Debug, Clone, Default)]
pub struct Load {
    pub total_pct: f64,
    /// One entry per logical CPU, in kernel order.
    pub per_core_pct: Vec<f64>,
}

impl LoadReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// Samples `/proc/stat` and reports load since the previous call.
    ///
    /// The first call establishes the baseline and reports zero; callers that
    /// want a meaningful first number should prime the reader one interval early.
    pub fn read(&mut self, root: &SysRoot) -> Load {
        let Some(content) = read_trimmed(root.join("proc/stat")) else {
            return Load::default();
        };

        let mut total = None;
        let mut per_core = Vec::new();
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("cpu") {
                match Jiffies::parse(line) {
                    Some(j) if rest.starts_with(char::is_whitespace) => total = Some(j),
                    Some(j) => per_core.push(j),
                    None => {}
                }
            } else if !line.starts_with("cpu") {
                break; // cpu lines are contiguous at the top of the file
            }
        }

        let total_pct = match (total, self.prev_total) {
            (Some(now), Some(prev)) => Jiffies::delta_pct(now, prev),
            _ => 0.0,
        };

        let per_core_pct = per_core
            .iter()
            .enumerate()
            .map(|(i, now)| match self.prev_per_core.get(i) {
                Some(prev) => Jiffies::delta_pct(*now, *prev),
                None => 0.0,
            })
            .collect();

        self.prev_total = total;
        self.prev_per_core = per_core;

        Load {
            total_pct,
            per_core_pct,
        }
    }
}

/// The cpufreq policy state for the package.
#[derive(Debug, Clone, Default)]
pub struct ScalingState {
    /// Active governor, e.g. `"powersave"`, `"performance"`, `"schedutil"`.
    pub governor: Option<String>,
    /// Energy/performance preference. Only present on `intel_pstate` and
    /// `amd_pstate_epp`; absent on `acpi-cpufreq`, which has no such concept.
    pub epp: Option<String>,
    /// Scaling driver, which decides which knobs exist at all.
    pub driver: Option<String>,
    /// Mean current frequency across online CPUs, MHz.
    pub freq_avg_mhz: u32,
    /// Highest current frequency across online CPUs, MHz. Worth showing
    /// separately: a single boosting core disappears into the average.
    pub freq_max_mhz: u32,
}

/// Reads governor, EPP, driver and current frequencies.
///
/// Frequencies come from `scaling_cur_freq`, which on `intel_pstate` is the
/// kernel's requested frequency rather than a hardware measurement. It is what
/// every other Linux tool shows, so we match them rather than inventing a
/// different number from `aperf`/`mperf`.
pub fn scaling_state(root: &SysRoot) -> ScalingState {
    let cpu0 = root.join("sys/devices/system/cpu/cpu0/cpufreq");
    let mut st = ScalingState {
        governor: read_trimmed(cpu0.join("scaling_governor")),
        epp: read_trimmed(cpu0.join("energy_performance_preference")),
        driver: read_trimmed(cpu0.join("scaling_driver")),
        ..Default::default()
    };

    let mut sum = 0u64;
    let mut count = 0u64;
    let mut max = 0u64;
    for n in online_cpus(root) {
        let path = root.join(format!(
            "sys/devices/system/cpu/cpu{n}/cpufreq/scaling_cur_freq"
        ));
        if let Some(khz) = read_u64(path) {
            sum += khz;
            max = max.max(khz);
            count += 1;
        }
    }
    if let Some(avg) = sum.checked_div(count) {
        st.freq_avg_mhz = (avg / 1000) as u32;
        st.freq_max_mhz = (max / 1000) as u32;
    }
    st
}

/// Lists online logical CPUs.
///
/// Prefers `/sys/devices/system/cpu/online`, which is authoritative and handles
/// offlined cores; falls back to counting `cpuN` directories on trees (including
/// fixtures) that do not provide it.
pub fn online_cpus(root: &SysRoot) -> Vec<u32> {
    if let Some(list) = read_trimmed(root.join("sys/devices/system/cpu/online")) {
        let cpus = crate::parse_cpu_list(&list);
        if !cpus.is_empty() {
            return cpus;
        }
    }

    let Ok(entries) = std::fs::read_dir(root.join("sys/devices/system/cpu")) else {
        return Vec::new();
    };
    let mut cpus: Vec<u32> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            e.file_name()
                .to_str()?
                .strip_prefix("cpu")?
                .parse::<u32>()
                .ok()
        })
        .collect();
    cpus.sort_unstable();
    cpus
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    #[test]
    fn first_read_establishes_a_baseline_without_spiking() {
        let fs = TestFs::new();
        fs.proc_stat(2, 100, 900);

        let mut r = LoadReader::new();
        let first = r.read(&SysRoot::at(fs.root()));
        assert_eq!(first.total_pct, 0.0, "no previous sample yet");
        assert_eq!(first.per_core_pct.len(), 2);
    }

    #[test]
    fn load_is_the_delta_between_two_reads() {
        let fs = TestFs::new();
        let root = SysRoot::at(fs.root());
        let mut r = LoadReader::new();

        fs.proc_stat(2, 100, 900);
        r.read(&root);

        // 50 more busy jiffies, 50 more idle => 50%.
        fs.proc_stat(2, 150, 950);
        let load = r.read(&root);
        assert!(
            (load.total_pct - 50.0).abs() < 0.001,
            "got {}",
            load.total_pct
        );
        assert!((load.per_core_pct[0] - 50.0).abs() < 0.001);
    }

    #[test]
    fn an_idle_interval_reports_zero_not_nan() {
        let fs = TestFs::new();
        let root = SysRoot::at(fs.root());
        let mut r = LoadReader::new();

        fs.proc_stat(1, 100, 900);
        r.read(&root);
        let load = r.read(&root); // identical counters
        assert_eq!(load.total_pct, 0.0);
        assert!(!load.total_pct.is_nan());
    }

    #[test]
    fn counters_that_go_backwards_do_not_panic() {
        let fs = TestFs::new();
        let root = SysRoot::at(fs.root());
        let mut r = LoadReader::new();

        fs.proc_stat(1, 500, 900);
        r.read(&root);
        fs.proc_stat(1, 100, 900); // e.g. a fixture rewritten mid-test
        let load = r.read(&root);
        assert!(load.total_pct.is_finite());
    }

    #[test]
    fn scaling_state_reads_governor_driver_and_frequencies() {
        let fs = TestFs::new();
        fs.cpu(0, "powersave", "intel_pstate", 2_400_000);
        fs.cpu(1, "powersave", "intel_pstate", 3_600_000);
        fs.write(
            "sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference",
            "balance_power",
        );

        let st = scaling_state(&SysRoot::at(fs.root()));
        assert_eq!(st.governor.as_deref(), Some("powersave"));
        assert_eq!(st.driver.as_deref(), Some("intel_pstate"));
        assert_eq!(st.epp.as_deref(), Some("balance_power"));
        assert_eq!(st.freq_avg_mhz, 3000);
        assert_eq!(
            st.freq_max_mhz, 3600,
            "a boosting core survives the average"
        );
    }

    #[test]
    fn missing_epp_is_absent_not_empty() {
        let fs = TestFs::new();
        fs.cpu(0, "schedutil", "acpi-cpufreq", 2_000_000);

        let st = scaling_state(&SysRoot::at(fs.root()));
        assert_eq!(st.epp, None, "acpi-cpufreq has no EPP");
        assert_eq!(st.governor.as_deref(), Some("schedutil"));
    }

    #[test]
    fn online_list_wins_over_directory_scan() {
        let fs = TestFs::new();
        fs.cpu(0, "performance", "amd_pstate_epp", 4_000_000);
        fs.cpu(1, "performance", "amd_pstate_epp", 4_000_000);
        fs.cpu(2, "performance", "amd_pstate_epp", 4_000_000);
        fs.write("sys/devices/system/cpu/online", "0-1");

        assert_eq!(online_cpus(&SysRoot::at(fs.root())), vec![0, 1]);
    }
}
