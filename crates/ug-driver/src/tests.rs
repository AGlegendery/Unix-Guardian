//! Each rule against a machine that has the problem, and one that does not.

use super::*;
use ug_testfs::TestFs;

/// The development laptop as read on 2026-10-03: Parrot (Debian), RTX 3050
/// + Intel iGPU, driver 550.163.01 through DKMS, nouveau blacklisted,
/// modeset=1 on the command line, sleep services on, video memory not kept.
fn laptop() -> TestFs {
    let fs = TestFs::new();
    let kr = "7.0.13+parrot7-amd64";
    fs.write("etc/os-release", "ID=parrot\nID_LIKE=debian\nPRETTY_NAME=\"Parrot Security 7\"\n")
        .write("proc/sys/kernel/osrelease", &format!("{kr}\n"))
        .write("proc/cmdline", "BOOT_IMAGE=/@/boot/vmlinuz root=UUID=x ro quiet splash nvidia-drm.modeset=1\n")
        .write("sys/kernel/security/lockdown", "[none] integrity confidentiality\n")
        .dir("sys/firmware/efi")
        .write("usr/sbin/update-initramfs", "")
        .write(&format!("lib/modules/{kr}/build/Makefile"), "")
        .write(&format!("lib/modules/{kr}/modules.dep"), "updates/dkms/nvidia-current.ko.xz:\nupdates/dkms/nvidia-current-drm.ko.xz: updates/dkms/nvidia-current.ko.xz\n")
        .dir(&format!("var/lib/dkms/nvidia-current/550.163.01/{kr}/x86_64/module"))
        .write("proc/modules", "nvidia_drm 139264 4 - Live 0x0\nnvidia_modeset 1720320 2 nvidia_drm, Live 0x0\nnvidia 60899328 30 nvidia_modeset, Live 0x0\ni915 4718592 40 - Live 0x0\n")
        .write("proc/driver/nvidia/version", "NVRM version: NVIDIA UNIX x86_64 Kernel Module  550.163.01  Tue Apr  8 12:41:17 UTC 2025\nGCC version:  gcc version 14.2.0\n")
        .write("proc/driver/nvidia/params", "ResmanDebugLevel: 4294967295\nPreserveVideoMemoryAllocations: 0\nDynamicPowerManagement: 3\n")
        .write("usr/lib/x86_64-linux-gnu/libnvidia-ml.so.550.163.01", "")
        .write("usr/lib/x86_64-linux-gnu/libvulkan.so.1", "")
        .write("usr/lib/x86_64-linux-gnu/dri/iHD_drv_video.so", "")
        .write("usr/share/vulkan/icd.d/nvidia_icd.json", "{}")
        .write("usr/share/vulkan/icd.d/intel_icd.json", "{}")
        .write("etc/modprobe.d/nvidia-blacklists-nouveau.conf", "blacklist nouveau\n")
        .write("etc/modprobe.d/nvidia.conf", "options nvidia-drm modeset=1\n")
        .write("etc/modprobe.d/nvidia-options.conf", "#options nvidia NVreg_PreserveVideoMemoryAllocations=1\noptions nvidia NVreg_UsePageAttributeTable=1\n")
        .dir("var/lib/flatpak/app/com.valvesoftware.Steam")
        .dir("var/lib/flatpak/runtime/org.freedesktop.Platform.GL.nvidia-550-163-01");
    for u in ["nvidia-suspend", "nvidia-resume", "nvidia-hibernate"] {
        fs.write(&format!("lib/systemd/system/{u}.service"), "");
    }
    fs.write("etc/systemd/system/systemd-suspend.service.wants/nvidia-suspend.service", "")
        .write("etc/systemd/system/systemd-suspend.service.wants/nvidia-resume.service", "")
        .write("etc/systemd/system/systemd-hibernate.service.wants/nvidia-hibernate.service", "");
    gpu(&fs, "0000:00:02.0", "0x8086", "0x46a3", "i915", true);
    gpu(&fs, "0000:01:00.0", "0x10de", "0x25a2", "nvidia", false);
    fs
}

fn gpu(fs: &TestFs, pci: &str, vendor: &str, device: &str, driver: &str, boot: bool) {
    let d = format!("sys/bus/pci/devices/{pci}");
    fs.write(&format!("{d}/class"), "0x030000\n")
        .write(&format!("{d}/vendor"), &format!("{vendor}\n"))
        .write(&format!("{d}/device"), &format!("{device}\n"))
        .write(&format!("{d}/boot_vga"), if boot { "1\n" } else { "0\n" });
    if !driver.is_empty() {
        fs.dir(&format!("sys/bus/pci/drivers/{driver}"));
        let _ = std::os::unix::fs::symlink(fs.root().join(format!("sys/bus/pci/drivers/{driver}")), fs.root().join(format!("{d}/driver")));
    }
}

fn wayland() -> Env {
    Env { session_type: "wayland".into(), home: None }
}

fn ids(fs: &TestFs, env: &Env) -> Vec<String> {
    diagnose(&SysRoot::at(fs.root()), env).findings.into_iter().map(|f| f.id).collect()
}

#[test]
fn every_fact_a_shipped_rule_names_exists() {
    let rules = rules::shipped().expect("shipped rules parse");
    assert!(rules.len() >= 20, "{} rules", rules.len());
    let facts = inventory::facts(&SysRoot::at(TestFs::new().root()), &Env::default());
    for r in &rules {
        for c in &r.when {
            for f in rules::named_facts(c) {
                assert!(facts.contains_key(&f), "rule {} names unknown fact {f}", r.id);
            }
        }
        assert!(!r.title_fa.is_empty() && !r.how_fa.is_empty(), "rule {} has its Persian text", r.id);
        assert!(r.fix.is_empty() || !fixes::commands(&r.fix, &facts).is_empty() || fixes::AUTO.contains(&r.fix.as_str()), "rule {} fix {} has commands", r.id, r.fix);
    }
    let mut seen = std::collections::HashSet::new();
    assert!(rules.iter().all(|r| seen.insert(r.id.clone())), "rule ids are unique");
}

#[test]
fn the_development_laptop_reads_as_it_really_is() {
    let fs = laptop();
    let f = inventory::facts(&SysRoot::at(fs.root()), &wayland());
    let t = |k: &str| f[k].text();
    assert_eq!(t("nvidia.kernel_version"), "550.163.01");
    assert_eq!(t("nvidia.lib_version"), "550.163.01");
    assert_eq!(t("nvidia.bound_driver"), "nvidia");
    assert_eq!(t("distro.family"), "apt");
    for yes in ["module.nvidia", "nvidia.module_built", "dkms.nvidia_built", "kernel.headers", "nouveau.blacklisted", "nvidia.drm_modeset", "nvidia.suspend_units_enabled", "flatpak.gl_nvidia_matches", "gpu.hybrid", "boot.efi"] {
        assert!(f[yes].truthy(), "{yes}");
    }
    for no in ["module.nouveau", "nvidia.preserve_vram", "nvidia.drm_fbdev", "boot.secure_boot", "nvidia.lib32"] {
        assert!(!f[no].truthy(), "{no}");
    }
    assert_eq!(t("boot.initramfs_tool"), "update-initramfs");
}

#[test]
fn the_laptop_gets_exactly_its_real_problems() {
    let fs = laptop();
    let mut got = ids(&fs, &wayland());
    got.sort();
    assert_eq!(got, ["nvidia-drm-fbdev-off", "nvidia-suspend-vram", "nvidia-wayland-old-driver"]);
    let r = diagnose(&SysRoot::at(fs.root()), &wayland());
    let vram = r.findings.iter().find(|f| f.id == "nvidia-suspend-vram").unwrap();
    assert!(vram.fix_auto, "Unix Guardian can fix this one itself");
    assert_eq!(r.findings[0].severity, "important", "most severe first");
    assert!(r.facts.iter().any(|f| f.key == "nvidia.kernel_version" && f.value == "550.163.01"));
}

#[test]
fn a_driver_update_without_a_restart_is_a_version_mismatch() {
    let fs = laptop();
    std::fs::remove_file(fs.root().join("usr/lib/x86_64-linux-gnu/libnvidia-ml.so.550.163.01")).unwrap();
    fs.write("usr/lib/x86_64-linux-gnu/libnvidia-ml.so.550.180.01", "");
    let r = diagnose(&SysRoot::at(fs.root()), &wayland());
    assert_eq!(r.findings[0].id, "nvidia-version-mismatch");
    assert_eq!(r.findings[0].commands, ["systemctl reboot"]);
    assert!(!r.findings[0].fix_auto);
}

#[test]
fn nouveau_holding_the_card_is_critical_and_fixable() {
    let fs = laptop();
    fs.write("proc/modules", "nouveau 3000000 1 - Live 0x0\n");
    std::fs::remove_file(fs.root().join("etc/modprobe.d/nvidia-blacklists-nouveau.conf")).unwrap();
    let got = ids(&fs, &Env::default());
    assert_eq!(got[0], "nouveau-with-nvidia");
    assert!(!got.contains(&"nouveau-not-blacklisted".to_string()), "one finding, not two, for one cause");
    assert!(!got.contains(&"nvidia-drm-modeset-off".to_string()), "nvidia is not loaded, so modeset is not the question");
}

#[test]
fn missing_headers_and_an_unbuilt_module_are_told_apart() {
    let fs = laptop();
    let kr = "7.0.13+parrot7-amd64";
    std::fs::remove_dir_all(fs.root().join(format!("lib/modules/{kr}/build"))).unwrap();
    std::fs::write(fs.root().join(format!("lib/modules/{kr}/modules.dep")), "").unwrap();
    std::fs::remove_dir_all(fs.root().join(format!("var/lib/dkms/nvidia-current/550.163.01/{kr}"))).unwrap();
    fs.write("proc/modules", "i915 1 1 - Live 0x0\n");
    let got = ids(&fs, &Env::default());
    assert!(got.contains(&"nvidia-kernel-headers-missing".to_string()), "{got:?}");
    assert!(!got.contains(&"nvidia-module-not-built".to_string()), "headers first: DKMS cannot build without them");
    fs.write(&format!("lib/modules/{kr}/build/Makefile"), "");
    let got = ids(&fs, &Env::default());
    assert!(got.contains(&"nvidia-module-not-built".to_string()), "{got:?}");
}

#[test]
fn secure_boot_refusing_the_module_needs_the_user_at_boot() {
    let fs = laptop();
    fs.write("sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c", "\x06\0\0\0\x01");
    fs.write("proc/modules", "i915 1 1 - Live 0x0\n");
    let r = diagnose(&SysRoot::at(fs.root()), &Env::default());
    let sb = r.findings.iter().find(|f| f.id == "nvidia-secure-boot-blocks-module").expect("found");
    assert!(!sb.fix_auto);
    assert!(sb.commands[0].contains("mokutil --import /var/lib/dkms/mok.pub"));
}

#[test]
fn flatpak_apps_need_the_runtime_of_this_exact_driver() {
    let fs = laptop();
    std::fs::remove_dir_all(fs.root().join("var/lib/flatpak/runtime/org.freedesktop.Platform.GL.nvidia-550-163-01")).unwrap();
    fs.dir("var/lib/flatpak/runtime/org.freedesktop.Platform.GL.nvidia-550-120");
    let r = diagnose(&SysRoot::at(fs.root()), &Env::default());
    let f = r.findings.iter().find(|f| f.id == "nvidia-flatpak-gl-mismatch").expect("found");
    assert!(f.commands[1].contains("org.freedesktop.Platform.GL.nvidia-550-163-01 org.freedesktop.Platform.GL32.nvidia-550-163-01"));
}

#[test]
fn old_cards_nomodeset_and_native_steam() {
    let fs = laptop();
    // A pci.ids-style name is what classifies the family.
    assert_eq!(inventory::nvidia_family("GK107 [GeForce GT 640]"), "kepler");
    assert_eq!(inventory::nvidia_family("GA107M [GeForce RTX 3050 Mobile]"), "ampere");
    assert_eq!(inventory::nvidia_family("TU117 [GeForce GTX 1650]"), "turing");
    assert_eq!(inventory::nvidia_family("GP104 [GeForce GTX 1080]"), "pascal");
    fs.write("proc/cmdline", "ro quiet nomodeset\n").write("usr/games/steam", "");
    let got = ids(&fs, &Env::default());
    assert_eq!(got[0], "kernel-nomodeset");
    assert!(got.contains(&"nvidia-steam-lib32-missing".to_string()));
    fs.write("usr/lib/i386-linux-gnu/libGLX_nvidia.so.0", "");
    assert!(!ids(&fs, &Env::default()).contains(&"nvidia-steam-lib32-missing".to_string()));
}

#[test]
fn a_machine_with_nothing_wrong_has_no_findings_and_no_gpu_is_not_an_error() {
    let fs = laptop();
    fs.write("etc/modprobe.d/nvidia-options.conf", "options nvidia NVreg_PreserveVideoMemoryAllocations=1\n")
        .write("proc/driver/nvidia/params", "PreserveVideoMemoryAllocations: 1\n")
        .write("proc/cmdline", "ro quiet nvidia-drm.modeset=1 nvidia-drm.fbdev=1\n");
    assert_eq!(ids(&fs, &Env { session_type: "x11".into(), home: None }), Vec::<String>::new());
    let empty = TestFs::new();
    empty.write("usr/lib/x86_64-linux-gnu/libvulkan.so.1", "");
    assert_eq!(ids(&empty, &Env::default()), Vec::<String>::new());
}
