//! Vendor-neutral GPU discovery from DRM and PCI sysfs.

use ug_hw::SysRoot;
use ug_proto::{GpuInfo, Vendor};

/// Finds every GPU with a DRM card node.
///
/// Render nodes (`renderD128`) and connector directories (`card0-HDMI-A-1`) are
/// skipped: only `cardN` with nothing after the digits is a device.
pub fn enumerate(root: &SysRoot) -> Vec<GpuInfo> {
    let base = root.join("sys/class/drm");
    let Ok(entries) = std::fs::read_dir(&base) else {
        tracing::debug!(path = %base.display(), "no DRM class directory");
        return Vec::new();
    };

    let mut gpus: Vec<GpuInfo> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let index: i32 = name.strip_prefix("card")?.parse().ok()?;
            read_card(&e.path(), index)
        })
        .collect();

    gpus.sort_by(|a, b| a.pci.cmp(&b.pci));
    gpus
}

/// True when the device at `pci` is runtime-suspended.
///
/// Reads `power/runtime_status` from the PCI device, which the kernel keeps
/// current without waking the device — unlike any vendor query. `suspending`
/// counts as asleep: touching the device mid-transition aborts the suspend.
/// A missing file means runtime PM is not in use (most desktops), so the
/// device is treated as awake and read normally.
pub fn runtime_suspended(root: &SysRoot, pci: &str) -> bool {
    let path = root.join(format!("sys/bus/pci/devices/{pci}/power/runtime_status"));
    matches!(
        std::fs::read_to_string(path).as_deref().map(str::trim),
        Ok("suspended") | Ok("suspending")
    )
}

/// Reads one `cardN` node into a [`GpuInfo`].
fn read_card(card: &std::path::Path, index: i32) -> Option<GpuInfo> {
    let device = card.join("device");
    let vendor_id = read_hex_u16(&device.join("vendor"))?;
    let device_id = read_hex_u16(&device.join("device")).unwrap_or(0);
    let vendor = Vendor::from_pci_id(vendor_id);

    Some(GpuInfo {
        pci: pci_address(&device).unwrap_or_default(),
        name: super::names::lookup(vendor_id, device_id),
        vendor,
        device_id: device_id as u32,
        driver: driver_name(&device).unwrap_or_default(),
        driver_version: String::new(), // filled in by the vendor backend
        vram_total_mb: 0,              // filled in by the vendor backend
        boot_vga: std::fs::read_to_string(device.join("boot_vga"))
            .map(|s| s.trim() == "1")
            .unwrap_or(false),
        card_index: index,
    })
}

/// Parses a sysfs hex attribute such as `0x10de`.
fn read_hex_u16(path: &std::path::Path) -> Option<u16> {
    let s = std::fs::read_to_string(path).ok()?;
    u16::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok()
}

/// Recovers the PCI address (`0000:01:00.0`) for a device directory.
///
/// Prefers `uevent`'s `PCI_SLOT_NAME`, which is plain text and works against a
/// fixture tree. Falls back to the directory name behind the `device` symlink,
/// which is how it reads on a real machine where `uevent` may be restricted.
fn pci_address(device: &std::path::Path) -> Option<String> {
    if let Ok(uevent) = std::fs::read_to_string(device.join("uevent")) {
        if let Some(slot) = uevent
            .lines()
            .find_map(|l| l.strip_prefix("PCI_SLOT_NAME="))
        {
            return Some(slot.trim().to_string());
        }
    }
    let real = std::fs::canonicalize(device).ok()?;
    let name = real.file_name()?.to_str()?;
    // A PCI address is domain:bus:device.function — reject anything else so a
    // platform device does not end up masquerading as one.
    if name.matches(':').count() == 2 && name.contains('.') {
        Some(name.to_string())
    } else {
        None
    }
}

/// Name of the bound kernel module, from the `driver` symlink.
fn driver_name(device: &std::path::Path) -> Option<String> {
    let link = std::fs::read_link(device.join("driver")).ok()?;
    Some(link.file_name()?.to_str()?.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    /// The development laptop: Alder Lake iGPU posted by firmware, plus a
    /// discrete GA107. The shape every hybrid-laptop decision is tested against.
    fn hybrid_laptop() -> TestFs {
        let fs = TestFs::new();
        fs.drm_card(0, "0000:00:02.0", 0x8086, 0x468b, true);
        fs.drm_card(1, "0000:01:00.0", 0x10de, 0x25ec, false);
        fs
    }

    #[test]
    fn enumerates_both_gpus_with_vendors_and_pci_addresses() {
        let fs = hybrid_laptop();
        let gpus = enumerate(&SysRoot::at(fs.root()));

        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0].pci, "0000:00:02.0");
        assert_eq!(gpus[0].vendor, Vendor::Intel);
        assert!(gpus[0].boot_vga);
        assert_eq!(gpus[1].pci, "0000:01:00.0");
        assert_eq!(gpus[1].vendor, Vendor::Nvidia);
        assert!(!gpus[1].boot_vga);
        assert_eq!(gpus[1].device_id, 0x25ec);
    }

    #[test]
    fn connector_and_render_nodes_are_not_devices() {
        let fs = hybrid_laptop();
        fs.dir("sys/class/drm/card0-HDMI-A-1");
        fs.dir("sys/class/drm/card1-eDP-1");
        fs.dir("sys/class/drm/renderD128");

        assert_eq!(enumerate(&SysRoot::at(fs.root())).len(), 2);
    }

    #[test]
    fn an_amd_card_is_recognised() {
        let fs = TestFs::new();
        fs.drm_card(0, "0000:03:00.0", 0x1002, 0x73df, true);

        let gpus = enumerate(&SysRoot::at(fs.root()));
        assert_eq!(gpus[0].vendor, Vendor::Amd);
        assert!(gpus[0].name.contains("AMD"), "got {}", gpus[0].name);
    }

    #[test]
    fn a_machine_with_no_drm_directory_yields_nothing() {
        let fs = TestFs::new();
        assert!(enumerate(&SysRoot::at(fs.root())).is_empty());
    }

    #[test]
    fn gpus_are_ordered_by_pci_address_not_discovery_order() {
        let fs = TestFs::new();
        fs.drm_card(1, "0000:01:00.0", 0x10de, 0x25ec, false);
        fs.drm_card(0, "0000:00:02.0", 0x8086, 0x468b, true);

        let gpus = enumerate(&SysRoot::at(fs.root()));
        assert_eq!(gpus[0].pci, "0000:00:02.0");
    }
}
