//! Everything else a normal Linux user fights with: Wi-Fi, Bluetooth,
//! sound, touchpads, cameras, printers and scanners, sleep, and missing
//! firmware. Same rules as the graphics inventory: files only, no root
//! needed, so `ug diagnose` works from a text console.
//!
//! The kernel log is the one thing files cannot give an ordinary user (it
//! needs root or the journal's group); the caller passes what it could read
//! in [`Env::kernel_log`], and the checks that need it simply find nothing
//! without it.

use crate::inventory::{Env, Facts, Value};
use std::path::{Path, PathBuf};
use ug_hw::SysRoot;

/// A device on the PCI bus: what it is and the driver that took it.
#[derive(Debug, Clone, PartialEq)]
pub struct PciDevice {
    pub pci: String,
    /// The class as read, e.g. `0x028000` (network, wireless).
    pub class: u32,
    pub vendor: u16,
    pub device: u16,
    /// "" when no driver took it.
    pub driver: String,
}

impl PciDevice {
    /// The top two bytes: `0x0280` wireless, `0x0403` audio…
    pub fn class16(&self) -> u32 {
        self.class >> 8
    }
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn driver_of(dev: &Path) -> String {
    std::fs::read_link(dev.join("driver"))
        .ok()
        .and_then(|l| l.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

fn hex(s: &str) -> u32 {
    u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).unwrap_or(0)
}

pub fn pci_devices(root: &SysRoot) -> Vec<PciDevice> {
    let mut v: Vec<PciDevice> = std::fs::read_dir(root.join("sys/bus/pci/devices"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| {
            let p = e.path();
            PciDevice {
                pci: e.file_name().to_string_lossy().into_owned(),
                class: hex(&read(&p.join("class"))),
                vendor: hex(&read(&p.join("vendor"))) as u16,
                device: hex(&read(&p.join("device"))) as u16,
                driver: driver_of(&p),
            }
        })
        .collect();
    v.sort_by(|a, b| a.pci.cmp(&b.pci));
    v
}

/// One USB interface (`1-3:1.0`): its class triple and driver, with the
/// device's ids and product name.
#[derive(Debug, Clone, PartialEq)]
pub struct UsbInterface {
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub driver: String,
    pub vendor: u16,
    pub product_id: u16,
    pub product: String,
}

pub fn usb_interfaces(root: &SysRoot) -> Vec<UsbInterface> {
    let mut v = Vec::new();
    for e in std::fs::read_dir(root.join("sys/bus/usb/devices"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let name = e.file_name().to_string_lossy().into_owned();
        // Interfaces are `<device>:<config>.<n>`; root hubs `usbN` have none.
        let Some((dev, _)) = name.split_once(':') else {
            continue;
        };
        let p = e.path();
        let d = root.join("sys/bus/usb/devices").join(dev);
        v.push(UsbInterface {
            class: hex(&read(&p.join("bInterfaceClass"))) as u8,
            subclass: hex(&read(&p.join("bInterfaceSubClass"))) as u8,
            protocol: hex(&read(&p.join("bInterfaceProtocol"))) as u8,
            driver: driver_of(&p),
            vendor: hex(&read(&d.join("idVendor"))) as u16,
            product_id: hex(&read(&d.join("idProduct"))) as u16,
            product: read(&d.join("product")),
        });
    }
    v
}

/// Firmware files the kernel asked for and did not find, from its log:
/// "Direct firmware load for iwlwifi-so-a0-gf-a0-86.ucode failed with
/// error -2" and "firmware: failed to load rtw89/rtw8852b_fw-1.bin (-2)".
/// Drivers try versions newest first, so a device that then logged a
/// loaded firmware has nothing missing.
pub fn missing_firmware(log: &str) -> Vec<String> {
    // The device is what precedes the message, after any "[  5.1]" time.
    let device = |l: &str, at: usize| {
        let head = l[..at].trim();
        let head = match head.strip_prefix('[') {
            Some(r) => r.split_once(']').map(|(_, rest)| rest).unwrap_or(r),
            None => head,
        };
        head.trim().trim_end_matches(':').to_string()
    };
    let loaded: Vec<String> = log
        .lines()
        .filter_map(|l| l.find("loaded firmware").map(|at| device(l, at)))
        .collect();
    let mut v: Vec<String> = log
        .lines()
        .filter_map(|l| {
            let (at, rest) = if let Some(at) = l.find("Direct firmware load for ") {
                (at, l[at + 25..].split(" failed").next())
            } else {
                let at = l.find("firmware: failed to load ")?;
                (at, l[at + 25..].split_whitespace().next())
            };
            let dev = device(l, at);
            if !dev.is_empty() && loaded.contains(&dev) {
                return None;
            }
            rest.map(str::to_string)
        })
        .filter(|f| !f.is_empty())
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Whether a firmware file, or another version of it, is on disk: the
/// kernel asks for `rtw8852b_fw-1.bin` and is happy with `rtw8852b_fw.bin`,
/// for `iwlwifi-so-a0-gf-a0-86.ucode` and takes `-83`.
pub fn firmware_variant_present(root: &SysRoot, file: &str) -> bool {
    let (dir, name) = file.rsplit_once('/').unwrap_or(("", file));
    let stem = name.split('.').next().unwrap_or(name);
    let base = match stem.rsplit_once('-') {
        Some((b, n)) if n.chars().all(|c| c.is_ascii_digit()) => b,
        _ => stem,
    };
    firmware_has(root, dir, base)
}

/// The kernel log lines about firmware, all the troubleshooter needs (no
/// other kernel messages leave the daemon).
pub fn firmware_lines(log: &str) -> String {
    log.lines()
        .filter(|l| l.contains("firmware"))
        .map(|l| format!("{l}\n"))
        .collect()
}

/// This boot's kernel messages about firmware, read with journalctl. Empty
/// when the caller may not read the journal (most users; the daemon can).
pub fn read_kernel_log() -> String {
    std::process::Command::new("journalctl")
        .args(["-k", "-b", "-q", "-o", "cat", "--no-pager"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| firmware_lines(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

/// The packages that carry a firmware file, for a distribution family.
/// Unknown files fall back to the family's general firmware package.
pub fn firmware_packages(files: &[String], family: &str) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for f in files {
        let pkg = match family {
            "apt" => {
                if f.starts_with("iwlwifi") || f.starts_with("intel/ibt") {
                    "firmware-iwlwifi"
                } else if f.starts_with("rtw") || f.starts_with("rtl") || f.starts_with("rtl_bt") {
                    "firmware-realtek"
                } else if f.starts_with("amdgpu") || f.starts_with("radeon") {
                    "firmware-amd-graphics"
                } else if f.starts_with("ath") || f.starts_with("qca") {
                    "firmware-atheros"
                } else if f.starts_with("brcm") {
                    "firmware-brcm80211"
                } else if f.starts_with("intel/sof") || f.starts_with("intel/avs") {
                    "firmware-sof-signed"
                } else if f.starts_with("mediatek") || f.starts_with("mt7") {
                    "firmware-mediatek"
                } else {
                    "firmware-misc-nonfree"
                }
            }
            "pacman" => {
                if f.starts_with("intel/sof") {
                    "sof-firmware"
                } else {
                    "linux-firmware"
                }
            }
            "dnf" => {
                if f.starts_with("iwlwifi") {
                    "iwlwifi-mvm-firmware"
                } else if f.starts_with("intel/sof") {
                    "alsa-sof-firmware"
                } else if f.starts_with("amdgpu") {
                    "amd-gpu-firmware"
                } else {
                    "linux-firmware"
                }
            }
            "zypper" => {
                if f.starts_with("iwlwifi") {
                    "kernel-firmware-iwlwifi"
                } else if f.starts_with("intel/sof") {
                    "sof-firmware"
                } else if f.starts_with("amdgpu") {
                    "kernel-firmware-amdgpu"
                } else if f.starts_with("rtw") || f.starts_with("rtl") {
                    "kernel-firmware-realtek"
                } else {
                    "kernel-firmware-all"
                }
            }
            _ => continue,
        };
        if !v.iter().any(|p| p == pkg) {
            v.push(pkg.to_string());
        }
    }
    v
}

/// Firmware folders, in the order the kernel searches them.
fn firmware_dirs(root: &SysRoot) -> Vec<PathBuf> {
    ["lib/firmware/updates", "lib/firmware", "usr/lib/firmware"]
        .iter()
        .map(|d| root.join(d))
        .collect()
}

fn firmware_has(root: &SysRoot, rel_dir: &str, prefix: &str) -> bool {
    firmware_dirs(root).iter().any(|d| {
        std::fs::read_dir(d.join(rel_dir))
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with(prefix))
    })
}

fn program(root: &SysRoot, names: &[&str]) -> bool {
    let dirs = [
        "usr/bin",
        "usr/sbin",
        "bin",
        "sbin",
        "usr/lib/bluetooth",
        "usr/libexec/bluetooth",
        "usr/libexec",
    ];
    names
        .iter()
        .any(|n| dirs.iter().any(|d| root.join(d).join(n).exists()))
}

/// Intel IPU6 camera controllers (laptops since Tiger Lake): the camera
/// sits behind an image processor that needs its own driver and libcamera.
const IPU6: &[u16] = &[0x9a19, 0x9a39, 0x4e19, 0x465d, 0x462e, 0xa75d, 0x7d19];

/// rfkill switches of a type ("wlan", "bluetooth"): (soft blocked, hard
/// blocked, the sysfs files to unblock).
pub fn rfkill(root: &SysRoot, kind: &str) -> (bool, bool, Vec<String>) {
    let (mut soft, mut hard, mut files) = (false, false, Vec::new());
    for e in std::fs::read_dir(root.join("sys/class/rfkill"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let p = e.path();
        if read(&p.join("type")) != kind {
            continue;
        }
        if read(&p.join("soft")) == "1" {
            soft = true;
            files.push(format!(
                "/sys/class/rfkill/{}/soft",
                e.file_name().to_string_lossy()
            ));
        }
        hard |= read(&p.join("hard")) == "1";
    }
    files.sort();
    (soft, hard, files)
}

fn kb(text: &str, key: &str) -> f64 {
    text.lines()
        .find_map(|l| l.strip_prefix(key))
        .and_then(|r| r.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0.0)
}

/// Adds the facts for every hardware class but graphics.
pub fn facts(root: &SysRoot, env: &Env, modules: &[String], f: &mut Facts) {
    let mut put = |k: &str, v: Value| {
        f.insert(k.to_string(), v);
    };
    let s = |v: &str| Value::Str(v.to_string());
    let b = Value::Bool;
    let loaded = |m: &str| modules.iter().any(|x| x == m);
    let pci = pci_devices(root);
    let usb = usb_interfaces(root);
    let names = |list: Vec<&PciDevice>| {
        list.iter()
            .map(|d| format!("{} {:04x}:{:04x}", d.pci, d.vendor, d.device))
            .collect::<Vec<_>>()
            .join(", ")
    };
    put(
        "machine.laptop",
        b(std::fs::read_dir(root.join("sys/class/power_supply"))
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| read(&e.path().join("type")) == "Battery")),
    );

    // Wi-Fi.
    let wifi: Vec<&PciDevice> = pci.iter().filter(|d| d.class16() == 0x0280).collect();
    let ifaces = std::fs::read_dir(root.join("sys/class/net"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("wireless").exists() || e.path().join("phy80211").exists())
        .count();
    put("wifi.pci", s(&names(wifi.clone())));
    put(
        "wifi.drivers",
        s(&wifi
            .iter()
            .map(|d| d.driver.as_str())
            .filter(|d| !d.is_empty())
            .collect::<Vec<_>>()
            .join(", ")),
    );
    put("wifi.interfaces", Value::Num(ifaces as f64));
    put("wifi.present", b(!wifi.is_empty() || ifaces > 0));
    put(
        "wifi.unbound",
        s(&names(
            wifi.iter()
                .copied()
                .filter(|d| d.driver.is_empty())
                .collect(),
        )),
    );
    put(
        "wifi.broadcom_unbound",
        b(wifi
            .iter()
            .any(|d| d.driver.is_empty() && d.vendor == 0x14e4)),
    );
    put(
        "wifi.bound_no_interface",
        b(wifi.iter().any(|d| !d.driver.is_empty()) && ifaces == 0),
    );
    let (soft, hard, _) = rfkill(root, "wlan");
    put("rfkill.wifi_soft", b(soft));
    put("rfkill.wifi_hard", b(hard));
    put("net.networkmanager", b(program(root, &["NetworkManager"])));

    // Bluetooth.
    let bt_usb: Vec<&UsbInterface> = usb
        .iter()
        .filter(|i| (i.class, i.subclass, i.protocol) == (0xe0, 0x01, 0x01))
        .collect();
    let adapters = std::fs::read_dir(root.join("sys/class/bluetooth"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("hci"))
        .count();
    put("bt.usb", b(!bt_usb.is_empty()));
    put(
        "bt.usb_unbound",
        b(bt_usb.iter().any(|i| i.driver.is_empty())),
    );
    put("bt.adapters", Value::Num(adapters as f64));
    put("bt.present", b(!bt_usb.is_empty() || adapters > 0));
    put("bt.bluez", b(program(root, &["bluetoothd"])));
    put(
        "bt.service_exists",
        b(crate::inventory::unit_exists(root, "bluetooth.service")),
    );
    put(
        "bt.service_enabled",
        b(crate::inventory::unit_enabled(root, "bluetooth.service")
            || root
                .join("etc/systemd/system/dbus-org.bluez.service")
                .exists()),
    );
    let (soft, hard, _) = rfkill(root, "bluetooth");
    put("rfkill.bt_soft", b(soft));
    put("rfkill.bt_hard", b(hard));

    // Sound.
    let audio: Vec<&PciDevice> = pci
        .iter()
        .filter(|d| matches!(d.class16(), 0x0401 | 0x0403))
        .collect();
    let cards = std::fs::read_to_string(root.join("proc/asound/cards"))
        .unwrap_or_default()
        .lines()
        .filter(|l| {
            l.split_whitespace()
                .next()
                .is_some_and(|n| n.parse::<u32>().is_ok())
        })
        .count();
    let sof = audio
        .iter()
        .any(|d| d.driver.starts_with("sof-audio") || d.driver.starts_with("snd_sof"));
    put("audio.pci", Value::Num(audio.len() as f64));
    put(
        "audio.unbound",
        s(&names(
            audio
                .iter()
                .copied()
                .filter(|d| d.driver.is_empty())
                .collect(),
        )),
    );
    put("audio.cards", Value::Num(cards as f64));
    put("audio.sof", b(sof));
    put("audio.sof_firmware", b(firmware_has(root, "intel", "sof")));
    let runtime = env.runtime_dir.as_ref();
    let server = match runtime {
        Some(r) if r.join("pipewire-0").exists() => "pipewire",
        Some(r) if r.join("pulse/native").exists() => "pulseaudio",
        Some(_) => "none",
        None => "",
    };
    put("audio.server", s(server));
    put("audio.pipewire", b(program(root, &["pipewire"])));
    put(
        "audio.pipewire_pulse",
        b(program(root, &["pipewire-pulse"])),
    );
    put("audio.pulseaudio", b(program(root, &["pulseaudio"])));
    put("audio.wireplumber", b(program(root, &["wireplumber"])));

    // Touchpads and other input.
    let mut i2c_present = false;
    let mut i2c_bound = false;
    for e in std::fs::read_dir(root.join("sys/bus/acpi/devices"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let p = e.path();
        let hid = read(&p.join("hid"));
        if hid != "PNP0C50" && hid != "ACPI0C50" {
            continue;
        }
        // `status` is the ACPI _STA: bit 0 says the device is there.
        let sta = read(&p.join("status")).parse::<u32>().unwrap_or(15);
        if sta & 1 == 0 {
            continue;
        }
        i2c_present = true;
        i2c_bound |= !driver_of(&p.join("physical_node")).is_empty();
    }
    let input = std::fs::read_to_string(root.join("proc/bus/input/devices")).unwrap_or_default();
    let names_in: Vec<&str> = input
        .lines()
        .filter_map(|l| l.strip_prefix("N: Name="))
        .map(|n| n.trim_matches('"'))
        .collect();
    put("input.i2c_hid", b(i2c_present));
    put("input.i2c_hid_bound", b(i2c_bound));
    put(
        "input.touchpad",
        b(names_in
            .iter()
            .any(|n| n.to_lowercase().contains("touchpad"))),
    );
    put(
        "input.ps2_mouse_only",
        b(names_in
            .iter()
            .any(|n| n.contains("PS/2 Generic Mouse") || n.contains("PS/2 Logitech Wheel Mouse"))),
    );
    put(
        "input.xorg_synaptics",
        b([
            "usr/share/X11/xorg.conf.d/70-synaptics.conf",
            "etc/X11/xorg.conf.d/70-synaptics.conf",
        ]
        .iter()
        .any(|p| root.join(p).exists())),
    );

    // Cameras.
    let uvc: Vec<&UsbInterface> = usb
        .iter()
        .filter(|i| i.class == 0x0e && i.subclass == 0x01)
        .collect();
    put("camera.uvc", b(!uvc.is_empty()));
    put(
        "camera.uvc_unbound",
        b(uvc.iter().any(|i| i.driver.is_empty())),
    );
    let v4l = std::fs::read_dir(root.join("sys/class/video4linux"))
        .into_iter()
        .flatten()
        .count();
    put("camera.video_devices", Value::Num(v4l as f64));
    let ipu = pci
        .iter()
        .find(|d| d.vendor == 0x8086 && IPU6.contains(&d.device));
    put("camera.ipu6", b(ipu.is_some()));
    put(
        "camera.ipu6_driver",
        s(ipu.map(|d| d.driver.as_str()).unwrap_or("")),
    );
    put(
        "camera.libcamera",
        b(program(root, &["cam", "libcamerify"])
            || ["usr/lib/x86_64-linux-gnu", "usr/lib64", "usr/lib"]
                .iter()
                .any(|d| {
                    root.join(d).join("libcamera.so").exists()
                        || std::fs::read_dir(root.join(d))
                            .into_iter()
                            .flatten()
                            .flatten()
                            .any(|e| e.file_name().to_string_lossy().starts_with("libcamera.so"))
                })),
    );

    // Printers and scanners.
    let printers: Vec<&UsbInterface> = usb.iter().filter(|i| i.class == 0x07).collect();
    put("print.usb_printer", b(!printers.is_empty()));
    put(
        "print.usb_printer_names",
        s(&printers
            .iter()
            .map(|i| i.product.clone())
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join(", ")),
    );
    put(
        "print.ipp_usb_device",
        b(printers
            .iter()
            .any(|i| i.subclass == 0x01 && i.protocol == 0x04)),
    );
    put("print.hp", b(printers.iter().any(|i| i.vendor == 0x03f0)));
    put("print.cups", b(program(root, &["cupsd"])));
    put(
        "print.cups_enabled",
        b(["cups.socket", "cups.service", "cups.path"]
            .iter()
            .any(|u| crate::inventory::unit_enabled(root, u))),
    );
    put("print.ipp_usb", b(program(root, &["ipp-usb"])));
    put("print.hplip", b(program(root, &["hp-setup"])));
    put("scan.sane", b(program(root, &["scanimage"])));
    let sane_dirs = [
        "usr/lib/x86_64-linux-gnu/sane",
        "usr/lib64/sane",
        "usr/lib/sane",
    ];
    put(
        "scan.airscan",
        b(sane_dirs.iter().any(|d| {
            std::fs::read_dir(root.join(d))
                .into_iter()
                .flatten()
                .flatten()
                .any(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with("libsane-airscan")
                })
        })),
    );
    put("net.avahi", b(program(root, &["avahi-daemon"])));
    put(
        "net.avahi_enabled",
        b(crate::inventory::unit_enabled(root, "avahi-daemon.service")
            || crate::inventory::unit_enabled(root, "avahi-daemon.socket")),
    );

    // Sleep and hibernation.
    let mem_sleep = read(&root.join("sys/power/mem_sleep"));
    let chosen = mem_sleep
        .split('[')
        .nth(1)
        .and_then(|r| r.split(']').next())
        .unwrap_or("");
    put("sleep.mem_sleep", s(chosen));
    put(
        "sleep.deep_available",
        b(mem_sleep
            .split_whitespace()
            .any(|w| w.trim_matches(['[', ']']) == "deep")),
    );
    let states = read(&root.join("sys/power/state"));
    put("sleep.states", s(&states));
    put(
        "hibernate.supported",
        b(states.split_whitespace().any(|w| w == "disk")),
    );
    // The image goes to one swap area, and never to zram (it is in RAM).
    let swap: f64 = std::fs::read_to_string(root.join("proc/swaps"))
        .unwrap_or_default()
        .lines()
        .skip(1)
        .filter(|l| !l.starts_with("/dev/zram"))
        .filter_map(|l| {
            l.split_whitespace()
                .nth(2)
                .and_then(|n| n.parse::<f64>().ok())
        })
        .fold(0.0, f64::max);
    let mem = kb(
        &std::fs::read_to_string(root.join("proc/meminfo")).unwrap_or_default(),
        "MemTotal:",
    );
    put("hibernate.swap_mb", Value::Num((swap / 1024.0).round()));
    put("hibernate.mem_mb", Value::Num((mem / 1024.0).round()));
    // The image is compressed and the kernel frees caches first, aiming
    // for image_size (2/5 of memory); but that is a target, not a promise:
    // when memory in use cannot be freed the image is bigger and hibernation
    // fails. 60 % of memory is the usual working minimum (and the smallest
    // file the hibernate-swap fix makes).
    put("hibernate.swap_enough", b(mem > 0.0 && swap >= mem * 0.6));
    // Where a hibernation swap file would go, for the fix and its guide.
    let plan = crate::swap::plan(root, mem as u64, |d| {
        crate::swap::free_bytes(&root.join(d.trim_start_matches('/')))
    });
    let (file, gib, fs, sub, problem) = match &plan {
        Ok(p) => (
            p.file.clone(),
            p.size_gib as f64,
            p.fs.clone(),
            p.new_subvolume.clone().unwrap_or_default(),
            String::new(),
        ),
        Err(e) => (String::new(), 0.0, String::new(), String::new(), e.clone()),
    };
    put("hibernate.plan_file", s(&file));
    put("hibernate.plan_gib", Value::Num(gib));
    put("hibernate.plan_fs", s(&fs));
    put("hibernate.plan_subvolume", s(&sub));
    put("hibernate.plan_problem", s(&problem));

    // Firmware.
    let mut missing = missing_firmware(&env.kernel_log);
    missing.retain(|m| !firmware_variant_present(root, m));
    put("firmware.log_read", b(!env.kernel_log.is_empty()));
    put("firmware.missing", s(&missing.join(", ")));
    put("firmware.missing_count", Value::Num(missing.len() as f64));
    put(
        "firmware.iwlwifi_missing",
        b(loaded("iwlwifi") && !firmware_has(root, "", "iwlwifi-")),
    );
    put(
        "firmware.amdgpu_missing",
        b(loaded("amdgpu") && !firmware_has(root, "amdgpu", "")),
    );
    put(
        "firmware.i915_missing",
        b(loaded("i915") && !firmware_has(root, "i915", "")),
    );
    put(
        "firmware.any_dir",
        b(firmware_dirs(root).iter().any(|d| d.is_dir())),
    );
    // Debian keeps most firmware in the non-free-firmware component, which
    // a netinst without it leaves off.
    let mut apt = String::new();
    {
        let p = "etc/apt/sources.list";
        apt.push_str(&std::fs::read_to_string(root.join(p)).unwrap_or_default());
    }
    for e in std::fs::read_dir(root.join("etc/apt/sources.list.d"))
        .into_iter()
        .flatten()
        .flatten()
    {
        apt.push_str(&std::fs::read_to_string(e.path()).unwrap_or_default());
    }
    let active: String = apt
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    put(
        "apt.nonfree_firmware",
        b(active.contains("non-free-firmware")),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_firmware_is_read_from_the_kernel_log() {
        let log = "\
iwlwifi 0000:00:14.3: Direct firmware load for iwlwifi-so-a0-gf-a0-86.ucode failed with error -2
iwlwifi 0000:00:14.3: Direct firmware load for iwlwifi-so-a0-gf-a0-85.ucode failed with error -2
rtw89_8852be 0000:08:00.0: firmware: failed to load rtw89/rtw8852b_fw-1.bin (-2)
sof-audio-pci-intel-tgl 0000:00:1f.3: Direct firmware load for intel/sof/sof-adl.ri failed with error -2
";
        assert_eq!(
            missing_firmware(log),
            [
                "intel/sof/sof-adl.ri",
                "iwlwifi-so-a0-gf-a0-85.ucode",
                "iwlwifi-so-a0-gf-a0-86.ucode",
                "rtw89/rtw8852b_fw-1.bin"
            ]
        );
        // A driver trying versions newest-first, then loading an older one.
        let tried = "\
[    5.1] iwlwifi 0000:00:14.3: Direct firmware load for iwlwifi-so-a0-gf-a0-89.ucode failed with error -2
[    5.2] iwlwifi 0000:00:14.3: loaded firmware version 86.fb5c9aeb.0 so-a0-gf-a0-86.ucode op_mode iwlmvm
";
        assert!(
            missing_firmware(tried).is_empty(),
            "nothing is missing once one version loaded"
        );
        assert!(missing_firmware("").is_empty());
    }

    #[test]
    fn firmware_maps_to_the_family_s_packages() {
        let f = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let files = f(&[
            "iwlwifi-so-a0-gf-a0-86.ucode",
            "rtw89/rtw8852b_fw.bin",
            "intel/sof/sof-adl.ri",
            "rtw89/x.bin",
        ]);
        assert_eq!(
            firmware_packages(&files, "apt"),
            [
                "firmware-iwlwifi",
                "firmware-realtek",
                "firmware-sof-signed"
            ]
        );
        assert_eq!(
            firmware_packages(&files, "pacman"),
            ["linux-firmware", "sof-firmware"]
        );
        assert_eq!(
            firmware_packages(&files, "dnf"),
            [
                "iwlwifi-mvm-firmware",
                "linux-firmware",
                "alsa-sof-firmware"
            ]
        );
        assert!(firmware_packages(&files, "unknown").is_empty());
    }
}
