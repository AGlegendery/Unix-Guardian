//! Each rule against a machine that has the problem, and one that does not.

use super::*;
use ug_testfs::TestFs;

/// The development laptop as read on 2026-10-03: Parrot (Debian), RTX 3050
/// and Intel iGPU, driver 550.163.01 through DKMS, nouveau blacklisted,
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
        .write(format!("lib/modules/{kr}/build/Makefile"), "")
        .write(format!("lib/modules/{kr}/modules.dep"), "updates/dkms/nvidia-current.ko.xz:\nupdates/dkms/nvidia-current-drm.ko.xz: updates/dkms/nvidia-current.ko.xz\n")
        .dir(format!("var/lib/dkms/nvidia-current/550.163.01/{kr}/x86_64/module"))
        .write("proc/modules", "nvidia_drm 139264 4 - Live 0x0\nnvidia_modeset 1720320 2 nvidia_drm, Live 0x0\nnvidia 60899328 30 nvidia_modeset, Live 0x0\ni915 4718592 40 - Live 0x0\n")
        .write("proc/driver/nvidia/version", "NVRM version: NVIDIA UNIX x86_64 Kernel Module  550.163.01  Tue Apr  8 12:41:17 UTC 2025\nGCC version:  gcc version 14.2.0\n")
        .write("proc/driver/nvidia/params", "ResmanDebugLevel: 4294967295\nPreserveVideoMemoryAllocations: 0\nDynamicPowerManagement: 3\n")
        .write("usr/lib/x86_64-linux-gnu/libnvidia-ml.so.550.163.01", "")
        .write("usr/lib/x86_64-linux-gnu/libvulkan.so.1", "")
        .write("usr/lib/x86_64-linux-gnu/dri/iHD_drv_video.so", "")
        .write("usr/share/vulkan/icd.d/nvidia_icd.json", "{}")
        .write("lib/firmware/i915/adlp_dmc.bin", "")
        .write("usr/share/vulkan/icd.d/intel_icd.json", "{}")
        .write("etc/modprobe.d/nvidia-blacklists-nouveau.conf", "blacklist nouveau\n")
        .write("etc/modprobe.d/nvidia.conf", "options nvidia-drm modeset=1\n")
        .write("etc/modprobe.d/nvidia-options.conf", "#options nvidia NVreg_PreserveVideoMemoryAllocations=1\noptions nvidia NVreg_UsePageAttributeTable=1\n")
        .dir("var/lib/flatpak/app/com.valvesoftware.Steam")
        .dir("var/lib/flatpak/runtime/org.freedesktop.Platform.GL.nvidia-550-163-01");
    for u in ["nvidia-suspend", "nvidia-resume", "nvidia-hibernate"] {
        fs.write(format!("lib/systemd/system/{u}.service"), "");
    }
    fs.write(
        "etc/systemd/system/systemd-suspend.service.wants/nvidia-suspend.service",
        "",
    )
    .write(
        "etc/systemd/system/systemd-suspend.service.wants/nvidia-resume.service",
        "",
    )
    .write(
        "etc/systemd/system/systemd-hibernate.service.wants/nvidia-hibernate.service",
        "",
    );
    gpu(&fs, "0000:00:02.0", "0x8086", "0x46a3", "i915", true);
    gpu(&fs, "0000:01:00.0", "0x10de", "0x25a2", "nvidia", false);
    fs
}

fn gpu(fs: &TestFs, pci: &str, vendor: &str, device: &str, driver: &str, boot: bool) {
    let d = format!("sys/bus/pci/devices/{pci}");
    fs.write(format!("{d}/class"), "0x030000\n")
        .write(format!("{d}/vendor"), &format!("{vendor}\n"))
        .write(format!("{d}/device"), &format!("{device}\n"))
        .write(format!("{d}/boot_vga"), if boot { "1\n" } else { "0\n" });
    if !driver.is_empty() {
        fs.dir(format!("sys/bus/pci/drivers/{driver}"));
        let _ = std::os::unix::fs::symlink(
            fs.root().join(format!("sys/bus/pci/drivers/{driver}")),
            fs.root().join(format!("{d}/driver")),
        );
    }
}

fn wayland() -> Env {
    Env {
        session_type: "wayland".into(),
        ..Default::default()
    }
}

fn ids(fs: &TestFs, env: &Env) -> Vec<String> {
    diagnose(&SysRoot::at(fs.root()), env)
        .findings
        .into_iter()
        .map(|f| f.id)
        .collect()
}

#[test]
fn every_fact_a_shipped_rule_names_exists() {
    let rules = rules::shipped().expect("shipped rules parse");
    assert!(rules.len() >= 20, "{} rules", rules.len());
    let facts = inventory::facts(&SysRoot::at(TestFs::new().root()), &Env::default());
    for r in &rules {
        for c in &r.when {
            for f in rules::named_facts(c) {
                assert!(
                    facts.contains_key(&f),
                    "rule {} names unknown fact {f}",
                    r.id
                );
            }
        }
        assert!(
            !r.title_fa.is_empty() && !r.how_fa.is_empty(),
            "rule {} has its Persian text",
            r.id
        );
        assert!(
            r.fix.is_empty()
                || !fixes::commands(&r.fix, &facts).is_empty()
                || fixes::AUTO.contains(&r.fix.as_str()),
            "rule {} fix {} has commands",
            r.id,
            r.fix
        );
    }
    let mut seen = std::collections::HashSet::new();
    assert!(
        rules.iter().all(|r| seen.insert(r.id.clone())),
        "rule ids are unique"
    );
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
    for yes in [
        "module.nvidia",
        "nvidia.module_built",
        "dkms.nvidia_built",
        "kernel.headers",
        "nouveau.blacklisted",
        "nvidia.drm_modeset",
        "nvidia.suspend_units_enabled",
        "flatpak.gl_nvidia_matches",
        "gpu.hybrid",
        "boot.efi",
    ] {
        assert!(f[yes].truthy(), "{yes}");
    }
    for no in [
        "module.nouveau",
        "nvidia.preserve_vram",
        "nvidia.drm_fbdev",
        "boot.secure_boot",
        "nvidia.lib32",
    ] {
        assert!(!f[no].truthy(), "{no}");
    }
    assert_eq!(t("boot.initramfs_tool"), "update-initramfs");
}

#[test]
fn the_laptop_gets_exactly_its_real_problems() {
    let fs = laptop();
    let mut got = ids(&fs, &wayland());
    got.sort();
    assert_eq!(
        got,
        [
            "nvidia-drm-fbdev-off",
            "nvidia-suspend-vram",
            "nvidia-wayland-old-driver"
        ]
    );
    let r = diagnose(&SysRoot::at(fs.root()), &wayland());
    let vram = r
        .findings
        .iter()
        .find(|f| f.id == "nvidia-suspend-vram")
        .unwrap();
    assert!(vram.fix_auto, "Unix Guardian can fix this one itself");
    assert_eq!(r.findings[0].severity, "important", "most severe first");
    assert!(r
        .facts
        .iter()
        .any(|f| f.key == "nvidia.kernel_version" && f.value == "550.163.01"));
}

#[test]
fn a_driver_update_without_a_restart_is_a_version_mismatch() {
    let fs = laptop();
    std::fs::remove_file(
        fs.root()
            .join("usr/lib/x86_64-linux-gnu/libnvidia-ml.so.550.163.01"),
    )
    .unwrap();
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
    std::fs::remove_file(
        fs.root()
            .join("etc/modprobe.d/nvidia-blacklists-nouveau.conf"),
    )
    .unwrap();
    let got = ids(&fs, &Env::default());
    assert_eq!(got[0], "nouveau-with-nvidia");
    assert!(
        !got.contains(&"nouveau-not-blacklisted".to_string()),
        "one finding, not two, for one cause"
    );
    assert!(
        !got.contains(&"nvidia-drm-modeset-off".to_string()),
        "nvidia is not loaded, so modeset is not the question"
    );
}

#[test]
fn missing_headers_and_an_unbuilt_module_are_told_apart() {
    let fs = laptop();
    let kr = "7.0.13+parrot7-amd64";
    std::fs::remove_dir_all(fs.root().join(format!("lib/modules/{kr}/build"))).unwrap();
    std::fs::write(fs.root().join(format!("lib/modules/{kr}/modules.dep")), "").unwrap();
    std::fs::remove_dir_all(
        fs.root()
            .join(format!("var/lib/dkms/nvidia-current/550.163.01/{kr}")),
    )
    .unwrap();
    fs.write("proc/modules", "i915 1 1 - Live 0x0\n");
    let got = ids(&fs, &Env::default());
    assert!(
        got.contains(&"nvidia-kernel-headers-missing".to_string()),
        "{got:?}"
    );
    assert!(
        !got.contains(&"nvidia-module-not-built".to_string()),
        "headers first: DKMS cannot build without them"
    );
    fs.write(format!("lib/modules/{kr}/build/Makefile"), "");
    let got = ids(&fs, &Env::default());
    assert!(
        got.contains(&"nvidia-module-not-built".to_string()),
        "{got:?}"
    );
}

#[test]
fn secure_boot_refusing_the_module_needs_the_user_at_boot() {
    let fs = laptop();
    fs.write(
        "sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c",
        "\x06\0\0\0\x01",
    );
    fs.write("proc/modules", "i915 1 1 - Live 0x0\n");
    let r = diagnose(&SysRoot::at(fs.root()), &Env::default());
    let sb = r
        .findings
        .iter()
        .find(|f| f.id == "nvidia-secure-boot-blocks-module")
        .expect("found");
    assert!(!sb.fix_auto);
    assert!(sb.commands[0].contains("mokutil --import /var/lib/dkms/mok.pub"));
}

#[test]
fn flatpak_apps_need_the_runtime_of_this_exact_driver() {
    let fs = laptop();
    std::fs::remove_dir_all(
        fs.root()
            .join("var/lib/flatpak/runtime/org.freedesktop.Platform.GL.nvidia-550-163-01"),
    )
    .unwrap();
    fs.dir("var/lib/flatpak/runtime/org.freedesktop.Platform.GL.nvidia-550-120");
    let r = diagnose(&SysRoot::at(fs.root()), &Env::default());
    let f = r
        .findings
        .iter()
        .find(|f| f.id == "nvidia-flatpak-gl-mismatch")
        .expect("found");
    assert!(f.commands[1].contains("org.freedesktop.Platform.GL.nvidia-550-163-01 org.freedesktop.Platform.GL32.nvidia-550-163-01"));
    // The user asked for "do it for me" here too: the service installs the
    // runtime system-wide, and undo uninstalls exactly it.
    assert!(f.fix_auto, "Unix Guardian installs it itself");
    fs.write("usr/bin/flatpak", "");
    let facts = inventory::facts(&SysRoot::at(fs.root()), &Env::default());
    let fix = crate::fixes::auto("flatpak-gl", &SysRoot::at(fs.root()), &facts).unwrap();
    assert!(!fix.needs_reboot);
    match &fix.steps[..] {
        [crate::fixes::Step::Command { argv, undo }] => {
            assert!(argv.join(" ").starts_with("flatpak install --system -y --noninteractive flathub org.freedesktop.Platform.GL.nvidia-550-163-01"), "{argv:?}");
            assert_eq!(undo[..2], ["flatpak".to_string(), "uninstall".to_string()]);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn old_cards_nomodeset_and_native_steam() {
    let fs = laptop();
    // A pci.ids-style name is what classifies the family.
    assert_eq!(inventory::nvidia_family("GK107 [GeForce GT 640]"), "kepler");
    assert_eq!(
        inventory::nvidia_family("GA107M [GeForce RTX 3050 Mobile]"),
        "ampere"
    );
    assert_eq!(
        inventory::nvidia_family("TU117 [GeForce GTX 1650]"),
        "turing"
    );
    assert_eq!(
        inventory::nvidia_family("GP104 [GeForce GTX 1080]"),
        "pascal"
    );
    fs.write("proc/cmdline", "ro quiet nomodeset\n")
        .write("usr/games/steam", "");
    let got = ids(&fs, &Env::default());
    assert_eq!(got[0], "kernel-nomodeset");
    assert!(got.contains(&"nvidia-steam-lib32-missing".to_string()));
    fs.write("usr/lib/i386-linux-gnu/libGLX_nvidia.so.0", "");
    assert!(!ids(&fs, &Env::default()).contains(&"nvidia-steam-lib32-missing".to_string()));
}

#[test]
fn a_machine_with_nothing_wrong_has_no_findings_and_no_gpu_is_not_an_error() {
    let fs = laptop();
    fs.write(
        "etc/modprobe.d/nvidia-options.conf",
        "options nvidia NVreg_PreserveVideoMemoryAllocations=1\n",
    )
    .write(
        "proc/driver/nvidia/params",
        "PreserveVideoMemoryAllocations: 1\n",
    )
    .write(
        "proc/cmdline",
        "ro quiet nvidia-drm.modeset=1 nvidia-drm.fbdev=1\n",
    );
    assert_eq!(
        ids(
            &fs,
            &Env {
                session_type: "x11".into(),
                ..Default::default()
            }
        ),
        Vec::<String>::new()
    );
    let empty = TestFs::new();
    empty.write("usr/lib/x86_64-linux-gnu/libvulkan.so.1", "");
    assert_eq!(ids(&empty, &Env::default()), Vec::<String>::new());
}

/// A plain Debian laptop with no graphics problems, to add one problem at a
/// time to.
fn plain() -> TestFs {
    let fs = TestFs::new();
    fs.write(
        "etc/os-release",
        "ID=debian\nPRETTY_NAME=\"Debian GNU/Linux 13\"\n",
    )
    .write(
        "etc/apt/sources.list",
        "deb http://deb.debian.org/debian trixie main non-free-firmware\n",
    )
    .write("usr/lib/x86_64-linux-gnu/libvulkan.so.1", "")
    .write("sys/class/power_supply/BAT0/type", "Battery\n");
    fs
}

fn pci(fs: &TestFs, slot: &str, class: &str, vendor: &str, device: &str, driver: &str) {
    let d = format!("sys/bus/pci/devices/{slot}");
    fs.write(format!("{d}/class"), &format!("{class}\n"))
        .write(format!("{d}/vendor"), &format!("{vendor}\n"))
        .write(format!("{d}/device"), &format!("{device}\n"));
    if !driver.is_empty() {
        fs.dir(format!("sys/bus/pci/drivers/{driver}"));
        let _ = std::os::unix::fs::symlink(
            fs.root().join(format!("sys/bus/pci/drivers/{driver}")),
            fs.root().join(format!("{d}/driver")),
        );
    }
}

fn usb(fs: &TestFs, dev: &str, class: &str, sub: &str, proto: &str, vendor: &str, driver: &str) {
    let d = format!("sys/bus/usb/devices/{dev}");
    fs.write(format!("{d}/idVendor"), &format!("{vendor}\n"))
        .write(format!("{d}/idProduct"), "0001\n")
        .write(format!("{d}/product"), "Thing\n");
    let i = format!("sys/bus/usb/devices/{dev}:1.0");
    fs.write(format!("{i}/bInterfaceClass"), &format!("{class}\n"))
        .write(format!("{i}/bInterfaceSubClass"), &format!("{sub}\n"))
        .write(format!("{i}/bInterfaceProtocol"), &format!("{proto}\n"));
    if !driver.is_empty() {
        fs.dir(format!("sys/bus/usb/drivers/{driver}"));
        let _ = std::os::unix::fs::symlink(
            fs.root().join(format!("sys/bus/usb/drivers/{driver}")),
            fs.root().join(format!("{i}/driver")),
        );
    }
}

fn has(fs: &TestFs, env: &Env, id: &str) -> bool {
    ids(fs, env).iter().any(|i| i == id)
}

#[test]
fn a_plain_laptop_has_nothing_wrong() {
    assert_eq!(ids(&plain(), &Env::default()), Vec::<String>::new());
}

#[test]
fn wifi_switched_off_without_a_driver_or_without_firmware() {
    let fs = plain();
    pci(
        &fs,
        "0000:08:00.0",
        "0x028000",
        "0x10ec",
        "0xb852",
        "rtw89_8852be",
    );
    fs.write("sys/class/net/wlan0/phy80211/name", "phy0\n");
    fs.write("sys/class/rfkill/rfkill0/type", "wlan\n")
        .write("sys/class/rfkill/rfkill0/soft", "1\n")
        .write("sys/class/rfkill/rfkill0/hard", "0\n");
    let r = diagnose(&SysRoot::at(fs.root()), &Env::default());
    let f = r
        .findings
        .iter()
        .find(|f| f.id == "wifi-blocked-soft")
        .expect("airplane mode found");
    assert_eq!(
        (f.category.as_str(), f.fix_auto),
        ("wifi", true),
        "switched on by Unix Guardian, no package needed"
    );
    fs.write("sys/class/rfkill/rfkill0/hard", "1\n");
    assert!(
        has(&fs, &Env::default(), "wifi-blocked-hard")
            && !has(&fs, &Env::default(), "wifi-blocked-soft")
    );

    let fs = plain();
    pci(
        &fs,
        "0000:08:00.0",
        "0x028000",
        "0x10ec",
        "0xb852",
        "rtw89_8852be",
    );
    assert!(
        has(&fs, &Env::default(), "wifi-driver-no-interface"),
        "a driver but no wlan0: firmware"
    );
    let r = diagnose(&SysRoot::at(fs.root()), &Env::default());
    let f = r
        .findings
        .iter()
        .find(|f| f.id == "wifi-driver-no-interface")
        .unwrap();
    assert!(
        f.commands.iter().any(|c| c.contains("firmware-realtek")),
        "{:?}",
        f.commands
    );

    let fs = plain();
    pci(&fs, "0000:03:00.0", "0x028000", "0x14e4", "0x43b1", "");
    assert!(
        has(&fs, &Env::default(), "wifi-broadcom-no-driver")
            && !has(&fs, &Env::default(), "wifi-no-driver")
    );
}

#[test]
fn bluetooth_off_without_its_service_or_without_bluez() {
    let fs = plain();
    usb(&fs, "1-10", "e0", "01", "01", "8087", "btusb");
    fs.dir("sys/class/bluetooth/hci0");
    assert!(has(&fs, &Env::default(), "bt-no-bluez"));
    fs.write("usr/libexec/bluetooth/bluetoothd", "")
        .write("usr/lib/systemd/system/bluetooth.service", "");
    assert!(has(&fs, &Env::default(), "bt-service-off"));
    fs.write(
        "etc/systemd/system/bluetooth.target.wants/bluetooth.service",
        "",
    );
    assert!(!has(&fs, &Env::default(), "bt-service-off"));
    std::fs::remove_dir(fs.root().join("sys/class/bluetooth/hci0")).unwrap();
    assert!(has(&fs, &Env::default(), "bt-adapter-not-ready"));
}

#[test]
fn no_sound_because_of_sof_firmware_or_no_sound_server() {
    let fs = plain();
    pci(
        &fs,
        "0000:00:1f.3",
        "0x040100",
        "0x8086",
        "0x51c8",
        "sof-audio-pci-intel-tgl",
    );
    fs.write("proc/asound/cards", "--- no soundcards ---\n");
    let got = ids(&fs, &Env::default());
    assert!(
        got.contains(&"audio-sof-firmware-missing".into())
            && !got.contains(&"audio-no-cards".into()),
        "{got:?}"
    );
    fs.write("lib/firmware/intel/sof/sof-adl.ri", "");
    assert!(!has(&fs, &Env::default(), "audio-sof-firmware-missing"));

    let fs = plain();
    pci(
        &fs,
        "0000:00:1f.3",
        "0x040300",
        "0x8086",
        "0x7ad0",
        "snd_hda_intel",
    );
    fs.write(
        "proc/asound/cards",
        " 0 [PCH            ]: HDA-Intel - HDA Intel PCH\n",
    );
    let run = tempfile::tempdir().unwrap();
    let env = Env {
        runtime_dir: Some(run.path().to_path_buf()),
        ..Default::default()
    };
    assert!(has(&fs, &env, "audio-no-server"));
    std::fs::write(run.path().join("pipewire-0"), "").unwrap();
    assert!(!has(&fs, &env, "audio-no-server"));
    assert!(
        !has(&fs, &Env::default(), "audio-no-server"),
        "unknown without the user's runtime directory"
    );
}

#[test]
fn touchpad_camera_and_printer_problems() {
    let fs = plain();
    fs.write("sys/bus/acpi/devices/PNP0C50:00/hid", "PNP0C50\n")
        .write("sys/bus/acpi/devices/PNP0C50:00/status", "15\n");
    assert!(has(&fs, &Env::default(), "touchpad-i2c-no-driver"));
    fs.write("sys/bus/acpi/devices/PNP0C50:00/status", "0\n");
    assert!(
        !has(&fs, &Env::default(), "touchpad-i2c-no-driver"),
        "listed but absent (_STA 0) is not a touchpad"
    );

    let fs = plain();
    usb(&fs, "3-6", "0e", "01", "00", "0c45", "");
    assert!(has(&fs, &Env::default(), "camera-uvc-no-driver"));
    pci(
        &fs,
        "0000:00:05.0",
        "0x048000",
        "0x8086",
        "0xa75d",
        "intel-ipu6",
    );
    assert!(has(&fs, &Env::default(), "camera-ipu6"));

    let fs = plain();
    usb(&fs, "1-2", "07", "01", "04", "03f0", "");
    let got = ids(&fs, &Env::default());
    assert!(
        got.contains(&"print-no-cups".into()) && got.contains(&"print-ipp-usb-missing".into()),
        "{got:?}"
    );
    assert!(
        !got.contains(&"print-hp-no-hplip".into()),
        "a driverless HP printer needs no HPLIP"
    );
    fs.write("usr/sbin/cupsd", "")
        .write("usr/sbin/ipp-usb", "")
        .write("usr/lib/systemd/system/cups.socket", "");
    assert!(has(&fs, &Env::default(), "print-cups-off"));
}

#[test]
fn sleep_firmware_and_debian_s_firmware_component() {
    let fs = plain();
    fs.write("sys/power/mem_sleep", "[s2idle] deep\n");
    assert!(has(&fs, &Env::default(), "sleep-s2idle-with-deep"));
    fs.write("sys/power/mem_sleep", "s2idle [deep]\n");
    assert!(!has(&fs, &Env::default(), "sleep-s2idle-with-deep"));
    fs.write("sys/power/state", "freeze mem disk\n")
        .write("proc/meminfo", "MemTotal:       16000000 kB\n");
    fs.write(
        "proc/swaps",
        "Filename Type Size Used Priority\n/dev/zram0 partition 4000000 0 100\n",
    );
    assert!(has(&fs, &Env::default(), "hibernate-swap-too-small"));
    // The laptop (2026-10-15): 27 GB of memory and a 16 GB swap file. The
    // kernel shrinks the image to image_size (2/5 of memory by default), so
    // 16 GB is plenty; the old 60 % rule called it too small.
    fs.write(
        "proc/meminfo",
        "MemTotal:       28429964 kB
",
    )
    .write(
        "sys/power/image_size",
        "11534872576
",
    )
    .write(
        "proc/swaps",
        "Filename Type Size Used Priority
/swap/swapfile file 16777212 0 -1
",
    );
    assert!(!has(&fs, &Env::default(), "hibernate-swap-too-small"));
    fs.write(
        "proc/swaps",
        "Filename Type Size Used Priority
/swap/swapfile file 8388604 0 -1
",
    );
    assert!(has(&fs, &Env::default(), "hibernate-swap-too-small"));

    let fs = plain();
    let log = Env { kernel_log: "iwlwifi 0000:00:14.3: Direct firmware load for iwlwifi-so-a0-gf-a0-86.ucode failed with error -2\n".into(), ..Default::default() };
    let r = diagnose(&SysRoot::at(fs.root()), &log);
    let f = r
        .findings
        .iter()
        .find(|f| f.id == "firmware-missing-from-log")
        .expect("found in the log");
    assert!(
        f.commands
            .iter()
            .any(|c| c == "sudo apt install firmware-iwlwifi"),
        "{:?}",
        f.commands
    );
    fs.write("lib/firmware/iwlwifi-so-a0-gf-a0-83.ucode", "");
    assert!(
        !has(&fs, &log, "firmware-missing-from-log"),
        "an older version on disk is what the driver uses"
    );

    fs.write("etc/apt/sources.list", "deb http://deb.debian.org/debian trixie main\n# deb http://deb.debian.org/debian trixie main non-free-firmware\n");
    assert!(has(&fs, &Env::default(), "firmware-debian-nonfree-off"));
    fs.write("proc/modules", "iwlwifi 1 0 - Live 0x0\n");
    fs.write("lib/firmware/iwlwifi-so-a0-gf-a0-83.ucode", "");
    assert!(!has(&fs, &Env::default(), "firmware-iwlwifi-missing"));
    std::fs::remove_file(fs.root().join("lib/firmware/iwlwifi-so-a0-gf-a0-83.ucode")).unwrap();
    assert!(has(&fs, &Env::default(), "firmware-iwlwifi-missing"));
}

#[test]
fn the_open_kernel_module_version_is_read() {
    // Arch's nvidia-open: "Open Kernel Module for x86_64", the version after "for x86_64".
    let fs = TestFs::new();
    fs.write("proc/driver/nvidia/version", "NVRM version: NVIDIA UNIX Open Kernel Module for x86_64  615.71.09  Release Build  (root@)  \nGCC version:  gcc version 15.2.1\n")
        .write("usr/lib/libnvidia-ml.so.615.71.09", "");
    let f = inventory::facts(&SysRoot::at(fs.root()), &Env::default());
    assert_eq!(f["nvidia.kernel_version"].text(), "615.71.09");
    assert!(!ids(&fs, &Env::default()).contains(&"nvidia-version-mismatch".to_string()));
}
