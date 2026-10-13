//! System memory and zram.

use crate::{read_trimmed, SysRoot};
use ug_proto::MemSample;

/// Reads `/proc/meminfo` and any active zram devices.
pub fn read(root: &SysRoot) -> MemSample {
    let mut sample = MemSample {
        total_kb: 0,
        available_kb: 0,
        swap_total_kb: 0,
        swap_used_kb: 0,
        zram_used_kb: 0,
    };

    if let Some(content) = read_trimmed(root.join("proc/meminfo")) {
        let mut swap_free_kb = 0u64;
        for line in content.lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            // "MemTotal:       16273init kB" -> take the first numeric field.
            let Some(kb) = value
                .split_whitespace()
                .next()
                .and_then(|v| v.parse::<u64>().ok())
            else {
                continue;
            };
            match key {
                "MemTotal" => sample.total_kb = kb,
                "MemAvailable" => sample.available_kb = kb,
                "SwapTotal" => sample.swap_total_kb = kb,
                "SwapFree" => swap_free_kb = kb,
                _ => {}
            }
        }
        sample.swap_used_kb = sample.swap_total_kb.saturating_sub(swap_free_kb);
    }

    sample.zram_used_kb = zram_used_kb(root);
    sample
}

/// Sums compressed data held across all zram devices, in KiB.
///
/// `mem_used_total` is in bytes and counts the compressed footprint plus
/// allocator overhead — the memory zram is actually costing, not the logical
/// size of what was swapped into it.
fn zram_used_kb(root: &SysRoot) -> u64 {
    let Ok(entries) = std::fs::read_dir(root.join("sys/block")) else {
        return 0;
    };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("zram"))
        .filter_map(|e| {
            // mm_stat is one whitespace-separated row; field 2 is mem_used_total.
            let row = read_trimmed(e.path().join("mm_stat"))?;
            row.split_whitespace().nth(2)?.parse::<u64>().ok()
        })
        .map(|bytes| bytes / 1024)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    #[test]
    fn reads_totals_and_derives_used() {
        let fs = TestFs::new();
        fs.proc_meminfo(32_000_000, 20_000_000, 8_000_000, 6_000_000);

        let m = read(&SysRoot::at(fs.root()));
        assert_eq!(m.total_kb, 32_000_000);
        assert_eq!(m.available_kb, 20_000_000);
        assert_eq!(m.used_kb(), 12_000_000);
        assert_eq!(m.swap_used_kb, 2_000_000);
        assert!((m.used_pct() - 37.5).abs() < 0.01);
    }

    #[test]
    fn a_machine_without_swap_reports_zero_not_garbage() {
        let fs = TestFs::new();
        fs.proc_meminfo(16_000_000, 8_000_000, 0, 0);

        let m = read(&SysRoot::at(fs.root()));
        assert_eq!(m.swap_total_kb, 0);
        assert_eq!(m.swap_used_kb, 0);
    }

    #[test]
    fn missing_meminfo_yields_zeroes_and_a_nan_percentage() {
        let fs = TestFs::new();
        let m = read(&SysRoot::at(fs.root()));
        assert_eq!(m.total_kb, 0);
        assert!(m.used_pct().is_nan(), "0/0 must not read as 0% used");
    }

    #[test]
    fn zram_footprint_is_summed_across_devices() {
        let fs = TestFs::new();
        fs.proc_meminfo(16_000_000, 8_000_000, 0, 0);
        // orig_data compr_data mem_used_total ...
        fs.write(
            "sys/block/zram0/mm_stat",
            "4194304 1048576 2097152 0 0 0 0 0",
        );
        fs.write(
            "sys/block/zram1/mm_stat",
            "4194304 1048576 1048576 0 0 0 0 0",
        );

        let m = read(&SysRoot::at(fs.root()));
        assert_eq!(m.zram_used_kb, (2_097_152 + 1_048_576) / 1024);
    }
}
