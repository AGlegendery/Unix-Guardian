//! CPU package power from RAPL energy counters.
//!
//! RAPL reports cumulative energy in microjoules, not power, so watts come from
//! dividing an energy delta by a time delta. The counter is fixed-width and
//! wraps; `max_energy_range_uj` gives the wrap point, and a wrapped read that is
//! treated as a decrease would otherwise show a large negative spike.
//!
//! Availability is uneven: present on Intel since Sandy Bridge and on AMD since
//! Zen, absent in most VMs, and readable only by root on kernels that restrict
//! it after the PLATYPUS side-channel disclosure. All three cases surface as
//! `None` rather than a fabricated number.

use crate::{read_u64, SysRoot};
use std::path::PathBuf;
use std::time::Instant;

/// One RAPL domain, typically `package-0`.
#[derive(Debug, Clone)]
struct Domain {
    energy_path: PathBuf,
    /// Value at which the counter wraps to zero.
    max_uj: u64,
}

/// Turns successive RAPL energy reads into watts.
///
/// Holds the previous counter and timestamp, so it must live across the
/// sampling loop. Like [`crate::cpu::LoadReader`], the first read has no
/// previous sample and reports `None`.
#[derive(Debug, Default)]
pub struct PowerReader {
    domains: Vec<Domain>,
    prev: Option<(u64, Instant)>,
    /// Set once discovery has run, so a machine without RAPL is not rescanned
    /// every second.
    scanned: bool,
}

impl PowerReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// True when at least one readable RAPL package domain was found.
    pub fn available(&self) -> bool {
        !self.domains.is_empty()
    }

    /// Package power in watts since the previous call, or `None` when RAPL is
    /// unavailable or this is the first read.
    pub fn read(&mut self, root: &SysRoot) -> Option<f64> {
        if !self.scanned {
            self.scan(root);
            self.scanned = true;
        }

        let total: u64 = self
            .domains
            .iter()
            .filter_map(|d| read_u64(&d.energy_path))
            .sum();
        if self.domains.is_empty() {
            return None;
        }

        let now = Instant::now();
        let result = match self.prev {
            Some((prev_uj, prev_at)) => {
                let secs = now.duration_since(prev_at).as_secs_f64();
                if secs <= 0.0 {
                    None
                } else {
                    let delta_uj = if total >= prev_uj {
                        total - prev_uj
                    } else {
                        // Counter wrapped. Recover the real delta using the
                        // domain wrap point rather than reporting a spike.
                        let wrap: u64 = self.domains.iter().map(|d| d.max_uj).sum();
                        if wrap > prev_uj {
                            wrap - prev_uj + total
                        } else {
                            return {
                                self.prev = Some((total, now));
                                None
                            };
                        }
                    };
                    Some(delta_uj as f64 / 1_000_000.0 / secs)
                }
            }
            None => None,
        };

        self.prev = Some((total, now));
        result
    }

    /// Finds package-level RAPL domains under `/sys/class/powercap`.
    ///
    /// Only top-level `intel-rapl:N` entries are used. Their `core`/`uncore`
    /// children are subsets of the package, and summing both would double-count.
    /// The name is `intel-rapl` on AMD too — the kernel reuses the interface.
    fn scan(&mut self, root: &SysRoot) {
        let base = root.join("sys/class/powercap");
        let Ok(entries) = std::fs::read_dir(&base) else {
            tracing::debug!("no powercap interface; CPU power unavailable");
            return;
        };

        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            // `intel-rapl:0` yes, `intel-rapl:0:1` (a subdomain) no.
            if !name.starts_with("intel-rapl:") || name.matches(':').count() != 1 {
                continue;
            }
            let path = entry.path();
            let energy_path = path.join("energy_uj");
            if read_u64(&energy_path).is_none() {
                tracing::debug!(
                    domain = name,
                    "RAPL domain present but not readable; needs root on this kernel"
                );
                continue;
            }
            let max_uj = read_u64(path.join("max_energy_range_uj")).unwrap_or(u64::MAX);
            self.domains.push(Domain {
                energy_path,
                max_uj,
            });
        }

        tracing::debug!(domains = self.domains.len(), "RAPL scan complete");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    fn rapl_fs(energy_uj: u64, max_uj: u64) -> TestFs {
        let fs = TestFs::new();
        fs.write(
            "sys/class/powercap/intel-rapl:0/energy_uj",
            &energy_uj.to_string(),
        );
        fs.write(
            "sys/class/powercap/intel-rapl:0/max_energy_range_uj",
            &max_uj.to_string(),
        );
        // A subdomain that must not be summed into the package total.
        fs.write("sys/class/powercap/intel-rapl:0:0/energy_uj", "999999999");
        fs
    }

    #[test]
    fn first_read_has_no_previous_sample() {
        let fs = rapl_fs(1_000_000, u64::MAX);
        let mut r = PowerReader::new();
        assert_eq!(r.read(&SysRoot::at(fs.root())), None);
        assert!(r.available());
    }

    #[test]
    fn subdomains_are_not_double_counted() {
        let fs = rapl_fs(1_000_000, u64::MAX);
        let mut r = PowerReader::new();
        r.read(&SysRoot::at(fs.root()));
        assert_eq!(r.domains.len(), 1, "only the package domain is summed");
    }

    #[test]
    fn absent_rapl_reports_none_rather_than_zero() {
        let fs = TestFs::new();
        let mut r = PowerReader::new();
        assert_eq!(r.read(&SysRoot::at(fs.root())), None);
        assert!(!r.available(), "a VM without RAPL must not claim 0 W");
    }

    #[test]
    fn a_wrapped_counter_does_not_produce_a_negative_spike() {
        let max = 10_000_000u64;
        let fs = rapl_fs(max - 1_000, max);
        let root = SysRoot::at(fs.root());
        let mut r = PowerReader::new();
        r.read(&root);

        std::thread::sleep(std::time::Duration::from_millis(20));
        // Wrapped past zero to 2000: real delta is 1000 + 2000 uJ.
        fs.write("sys/class/powercap/intel-rapl:0/energy_uj", "2000");
        let watts = r.read(&root).expect("wrap is recovered, not skipped");
        assert!(watts > 0.0, "got {watts}");
        assert!(watts < 10.0, "3000 uJ over ~20 ms is small, got {watts}");
    }

    #[test]
    fn watts_are_energy_over_time() {
        let fs = rapl_fs(0, u64::MAX);
        let root = SysRoot::at(fs.root());
        let mut r = PowerReader::new();
        r.read(&root);

        std::thread::sleep(std::time::Duration::from_millis(100));
        // 1 J over roughly 0.1 s is roughly 10 W.
        fs.write("sys/class/powercap/intel-rapl:0/energy_uj", "1000000");
        let watts = r.read(&root).expect("second read yields a rate");
        assert!((5.0..20.0).contains(&watts), "got {watts}");
    }
}
