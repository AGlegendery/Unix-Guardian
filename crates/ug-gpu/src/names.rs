//! Marketing names for PCI device IDs, from the system `pci.ids` database.
//!
//! `pci.ids` ships with `hwdata`/`pciutils` on every distribution we target and
//! is read without touching the device — which matters, because the vendor
//! sources of a name (NVML, NVIDIA's `/proc/driver/nvidia/gpus/*/information`)
//! wake a runtime-suspended GPU to answer. When a GPU is awake and its backend
//! has a better name, the backend overrides this one.
//!
//! There is deliberately no hand-maintained table here. One was tried; the
//! first entry checked against real hardware was wrong.

/// Returns the best name `pci.ids` offers, or a vendor-plus-ID string that is
/// still useful in a bug report.
pub fn lookup(vendor: u16, device: u16) -> String {
    const PATHS: [&str; 3] = [
        "/usr/share/hwdata/pci.ids",
        "/usr/share/misc/pci.ids",
        "/usr/share/pci.ids",
    ];
    PATHS
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
        .and_then(|db| lookup_in(&db, vendor, device))
        .unwrap_or_else(|| format!("{} GPU [{vendor:04x}:{device:04x}]", vendor_name(vendor)))
}

/// Short vendor name, used as the prefix of every GPU name.
fn vendor_name(vendor: u16) -> &'static str {
    match vendor {
        0x10de => "NVIDIA",
        0x1002 | 0x1022 => "AMD",
        0x8086 => "Intel",
        _ => "Unknown",
    }
}

/// Looks a vendor/device pair up in `pci.ids` content.
///
/// The format nests devices under vendors with one tab of indentation
/// (subsystems use two). Parsing stops at the end of the matching vendor block
/// so a device ID that also exists under another vendor cannot be misread.
///
/// Device names usually read `Chip codename [Marketing Name]`, e.g.
/// `GA107BM / GN20-P0-R-K2 [GeForce RTX 3050 6GB Laptop GPU]`. The bracketed
/// part is what a user recognises, so it is extracted and given the vendor
/// prefix — which reproduces NVML's own string exactly for NVIDIA cards.
pub(crate) fn lookup_in(db: &str, vendor: u16, device: u16) -> Option<String> {
    let vendor_key = format!("{vendor:04x}");
    let device_key = format!("{device:04x}");
    let mut in_vendor = false;

    for line in db.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        if !line.starts_with('\t') {
            if in_vendor {
                return None; // left our vendor's block without a match
            }
            in_vendor = line.starts_with(&vendor_key);
            continue;
        }
        if !in_vendor || line.starts_with("\t\t") {
            continue;
        }
        let Some(raw) = line.trim_start().strip_prefix(&device_key) else {
            continue;
        };
        let raw = raw.trim();
        let marketing = match (raw.rfind('['), raw.rfind(']')) {
            (Some(a), Some(b)) if b > a + 1 => &raw[a + 1..b],
            _ => raw,
        };
        return Some(format!("{} {}", vendor_name(vendor), marketing.trim()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const DB: &str = "\
# comment line
10de  NVIDIA Corporation
\t25ec  GA107BM / GN20-P0-R-K2 [GeForce RTX 3050 6GB Laptop GPU]
\t\t1043 1ccc  some subsystem [not a device]
\t2c05  GB203 [GeForce RTX 5070 Ti]
1002  Advanced Micro Devices, Inc. [AMD/ATI]
\t73df  Navi 22 [Radeon RX 6700/6700 XT/6750 XT / 6800M/6850M XT]
\t25ec  An AMD device that shares NVIDIA's device ID
8086  Intel Corporation
\t468b  Alder Lake-HX GT1 [UHD Graphics 770]
\t9999  Plain Name Without Brackets
";

    #[test]
    fn extracts_the_marketing_name_and_matches_nvml() {
        assert_eq!(
            lookup_in(DB, 0x10de, 0x25ec).as_deref(),
            Some("NVIDIA GeForce RTX 3050 6GB Laptop GPU"),
            "must equal what NVML reports for this card"
        );
    }

    #[test]
    fn a_device_id_is_resolved_under_its_own_vendor_only() {
        assert_eq!(
            lookup_in(DB, 0x1002, 0x25ec).as_deref(),
            Some("AMD An AMD device that shares NVIDIA's device ID")
        );
    }

    #[test]
    fn subsystem_lines_are_not_mistaken_for_devices() {
        assert_eq!(lookup_in(DB, 0x10de, 0x1043), None);
    }

    #[test]
    fn amd_and_intel_names_get_their_vendor_prefix() {
        assert_eq!(
            lookup_in(DB, 0x1002, 0x73df).as_deref(),
            Some("AMD Radeon RX 6700/6700 XT/6750 XT / 6800M/6850M XT")
        );
        assert_eq!(lookup_in(DB, 0x8086, 0x468b).as_deref(), Some("Intel UHD Graphics 770"));
    }

    #[test]
    fn a_name_without_brackets_is_used_whole() {
        assert_eq!(lookup_in(DB, 0x8086, 0x9999).as_deref(), Some("Intel Plain Name Without Brackets"));
    }

    #[test]
    fn an_unknown_device_falls_through() {
        assert_eq!(lookup_in(DB, 0x10de, 0xffff), None);
        assert_eq!(lookup_in(DB, 0x1234, 0x5678), None);
    }

    #[test]
    fn the_fallback_keeps_the_raw_ids_for_bug_reports() {
        let name = lookup(0x1234, 0xabcd);
        assert!(name.contains("1234") && name.contains("abcd"), "got {name}");
    }
}
