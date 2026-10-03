//! What the machine's graphics stack looks like, read from files only.
//!
//! Nothing here runs a program or needs root, so `ug diagnose` works from a
//! text console with the daemon down — the moment it matters most. Values
//! that only root may read (`nvidia_drm`'s parameters) fall back to what the
//! kernel command line and `modprobe.d` ask for.
//!
//! Every reading becomes a *fact* (`nvidia.kernel_version = "550.163.01"`),
//! which is what the rules in `data/driver-rules/*.toml` test.

use std::collections::BTreeMap;
use std::path::PathBuf;
use ug_hw::SysRoot;

/// A fact's value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Num(f64),
    Str(String),
}

impl Value {
    pub fn truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Num(n) => *n != 0.0,
            Value::Str(s) => !s.is_empty(),
        }
    }

    pub fn text(&self) -> String {
        match self {
            Value::Bool(b) => b.to_string(),
            Value::Num(n) if n.fract() == 0.0 => format!("{}", *n as i64),
            Value::Num(n) => n.to_string(),
            Value::Str(s) => s.clone(),
        }
    }
}

pub type Facts = BTreeMap<String, Value>;

/// What the caller knows that files do not: the desktop session and the
/// user's home (for Flatpak installed per user). The daemon has neither.
#[derive(Debug, Clone, Default)]
pub struct Env {
    /// `XDG_SESSION_TYPE`: `wayland`, `x11`, `tty` or "".
    pub session_type: String,
    pub home: Option<PathBuf>,
}

impl Env {
    pub fn from_process() -> Env {
        Env {
            session_type: std::env::var("XDG_SESSION_TYPE").unwrap_or_default(),
            home: std::env::var_os("HOME").map(PathBuf::from),
        }
    }
}

/// A display controller on the PCI bus, whether or not a driver took it.
#[derive(Debug, Clone, PartialEq)]
pub struct PciGpu {
    pub pci: String,
    pub vendor: u16,
    pub device: u16,
    /// The bound kernel driver; "" when none (the problem to solve).
    pub driver: String,
    pub boot_vga: bool,
    pub name: String,
}

pub const NVIDIA: u16 = 0x10de;
pub const AMD: u16 = 0x1002;
pub const INTEL: u16 = 0x8086;

/// Display controllers (PCI class 0x03xxxx).
pub fn pci_gpus(root: &SysRoot) -> Vec<PciGpu> {
    let mut v = Vec::new();
    for e in std::fs::read_dir(root.join("sys/bus/pci/devices")).into_iter().flatten().flatten() {
        let p = e.path();
        let read = |f: &str| std::fs::read_to_string(p.join(f)).map(|s| s.trim().to_string()).unwrap_or_default();
        if !read("class").starts_with("0x03") {
            continue;
        }
        let hex = |s: String| u16::from_str_radix(s.trim_start_matches("0x"), 16).unwrap_or(0);
        let (vendor, device) = (hex(read("vendor")), hex(read("device")));
        let driver = std::fs::read_link(p.join("driver"))
            .ok()
            .and_then(|l| l.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        v.push(PciGpu {
            pci: e.file_name().to_string_lossy().into_owned(),
            vendor,
            device,
            driver,
            boot_vga: read("boot_vga") == "1",
            name: ug_gpu::names::lookup(vendor, device),
        });
    }
    v.sort_by(|a, b| a.pci.cmp(&b.pci));
    v
}

/// The NVIDIA chip family from the pci.ids name ("GK107 [GeForce GT 640]").
pub fn nvidia_family(name: &str) -> &'static str {
    let chip = name.split_whitespace().next().unwrap_or("");
    let has = |p: &str| chip.starts_with(p) && chip[p.len()..].chars().next().is_some_and(|c| c.is_ascii_digit());
    if has("GB") {
        "blackwell"
    } else if has("AD") {
        "ada"
    } else if has("GH") {
        "hopper"
    } else if has("GA") {
        "ampere"
    } else if has("TU") {
        "turing"
    } else if has("GV") {
        "volta"
    } else if has("GP") {
        "pascal"
    } else if has("GM") {
        "maxwell"
    } else if has("GK") {
        "kepler"
    } else if has("GF") {
        "fermi"
    } else if has("G") || has("GT") {
        "tesla"
    } else {
        ""
    }
}

/// `550.163.01` from "NVRM version: NVIDIA UNIX x86_64 Kernel Module  550.163.01  …".
fn nvidia_kernel_version(root: &SysRoot) -> String {
    if let Ok(t) = std::fs::read_to_string(root.join("proc/driver/nvidia/version")) {
        if let Some(v) = t.lines().next().and_then(|l| l.split("Kernel Module").nth(1)).and_then(|r| r.split_whitespace().next()) {
            return v.to_string();
        }
    }
    std::fs::read_to_string(root.join("sys/module/nvidia/version")).map(|s| s.trim().to_string()).unwrap_or_default()
}

/// Library folders, 64-bit and 32-bit, across the distribution families.
const LIB64: &[&str] = &["usr/lib/x86_64-linux-gnu", "usr/lib64", "usr/lib", "usr/lib/x86_64-linux-gnu/nvidia/current"];
const LIB32: &[&str] = &["usr/lib/i386-linux-gnu", "usr/lib32", "usr/lib/i386-linux-gnu/nvidia/current"];

/// The version in `libnvidia-ml.so.550.163.01`, the library that has to
/// match the kernel module.
fn nvidia_lib_version(root: &SysRoot) -> String {
    for d in LIB64 {
        for e in std::fs::read_dir(root.join(d)).into_iter().flatten().flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if let Some(v) = n.strip_prefix("libnvidia-ml.so.") {
                if v.matches('.').count() >= 1 {
                    return v.to_string();
                }
            }
        }
    }
    String::new()
}

fn any_file(root: &SysRoot, dirs: &[&str], names: &[&str]) -> bool {
    dirs.iter().any(|d| names.iter().any(|n| root.join(d).join(n).exists()))
}

/// Lines of every modprobe configuration file, comments dropped.
fn modprobe_lines(root: &SysRoot) -> Vec<String> {
    let mut files: Vec<PathBuf> = Vec::new();
    for d in ["etc/modprobe.d", "run/modprobe.d", "usr/lib/modprobe.d", "lib/modprobe.d"] {
        for e in std::fs::read_dir(root.join(d)).into_iter().flatten().flatten() {
            if e.path().extension().is_some_and(|x| x == "conf") {
                files.push(e.path());
            }
        }
    }
    files.sort();
    files
        .iter()
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .flat_map(|t| t.lines().map(|l| l.split('#').next().unwrap_or("").trim().to_string()).collect::<Vec<_>>())
        .filter(|l| !l.is_empty())
        .collect()
}

/// A module option as asked for: the kernel command line wins over
/// `modprobe.d`, and in `modprobe.d` the last setting wins. Module names
/// accept `-` or `_`.
fn asked_option(cmdline: &str, modprobe: &[String], module: &str, option: &str) -> Option<String> {
    let alt = module.replace('_', "-");
    let mods = [module.to_string(), alt.clone(), module.replace('-', "_")];
    for w in cmdline.split_whitespace() {
        if let Some((k, v)) = w.split_once('=') {
            if let Some((m, o)) = k.split_once('.') {
                if mods.iter().any(|x| x == m) && o == option {
                    return Some(v.to_string());
                }
            }
        }
    }
    let mut found = None;
    for l in modprobe {
        let mut w = l.split_whitespace();
        if w.next() == Some("options") && w.next().is_some_and(|m| mods.iter().any(|x| x == m)) {
            for kv in w {
                if let Some((k, v)) = kv.split_once('=') {
                    if k == option {
                        found = Some(v.to_string());
                    }
                }
            }
        }
    }
    found
}

fn yes(v: &str) -> bool {
    matches!(v.trim(), "1" | "Y" | "y" | "true")
}

/// A unit is enabled when some target wants it.
pub(crate) fn unit_enabled(root: &SysRoot, unit: &str) -> bool {
    for d in ["etc/systemd/system"] {
        for e in std::fs::read_dir(root.join(d)).into_iter().flatten().flatten() {
            if e.file_name().to_string_lossy().ends_with(".wants") && e.path().join(unit).exists() {
                return true;
            }
        }
    }
    false
}

pub(crate) fn unit_exists(root: &SysRoot, unit: &str) -> bool {
    ["usr/lib/systemd/system", "lib/systemd/system", "etc/systemd/system"].iter().any(|d| root.join(d).join(unit).exists())
}

/// Secure Boot from the EFI variable: 4 attribute bytes, then 1 = on.
fn secure_boot(root: &SysRoot) -> Option<bool> {
    let b = std::fs::read(root.join("sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c")).ok()?;
    b.get(4).map(|v| *v == 1)
}

/// The initramfs tool this system uses, by the program present.
pub fn initramfs_tool(root: &SysRoot) -> Option<(&'static str, Vec<&'static str>)> {
    let has = |p: &str| ["usr/sbin", "usr/bin", "sbin", "bin"].iter().any(|d| root.join(d).join(p).exists());
    if has("update-initramfs") {
        Some(("update-initramfs", vec!["update-initramfs", "-u", "-k", "all"]))
    } else if has("mkinitcpio") {
        Some(("mkinitcpio", vec!["mkinitcpio", "-P"]))
    } else if has("dracut") {
        Some(("dracut", vec!["dracut", "-f", "--regenerate-all"]))
    } else {
        None
    }
}

/// Flatpak runtimes, system-wide and the user's own.
fn flatpak_runtimes(root: &SysRoot, env: &Env) -> Vec<String> {
    let mut dirs = vec![root.join("var/lib/flatpak/runtime")];
    if let Some(h) = &env.home {
        dirs.push(h.join(".local/share/flatpak/runtime"));
    }
    let mut v: Vec<String> = dirs
        .iter()
        .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v.dedup();
    v
}

fn flatpak_app(root: &SysRoot, env: &Env, app: &str) -> bool {
    root.join("var/lib/flatpak/app").join(app).exists()
        || env.home.as_ref().is_some_and(|h| h.join(".local/share/flatpak/app").join(app).exists())
}

/// Reads everything the rules look at.
pub fn facts(root: &SysRoot, env: &Env) -> Facts {
    let mut f = Facts::new();
    let mut put = |k: &str, v: Value| {
        f.insert(k.to_string(), v);
    };
    let s = |v: &str| Value::Str(v.to_string());

    let distro = ug_core::distro::Distro::detect(root);
    put("distro.id", s(&distro.id));
    put("distro.name", s(&distro.name));
    put("distro.like", s(&distro.id_like.join(" ")));
    put("distro.family", s(&format!("{:?}", distro.family).to_lowercase()));
    put("distro.can_change", Value::Bool(distro.allows_package_changes()));

    let kernel = std::fs::read_to_string(root.join("proc/sys/kernel/osrelease")).map(|s| s.trim().to_string()).unwrap_or_default();
    put("kernel.release", s(&kernel));
    put("kernel.headers", Value::Bool(!kernel.is_empty() && root.join("lib/modules").join(&kernel).join("build").exists()));
    let cmdline = std::fs::read_to_string(root.join("proc/cmdline")).unwrap_or_default().trim().to_string();
    put("kernel.cmdline", s(&cmdline));
    let lockdown = std::fs::read_to_string(root.join("sys/kernel/security/lockdown"))
        .ok()
        .and_then(|t| t.split('[').nth(1).and_then(|r| r.split(']').next()).map(str::to_string))
        .unwrap_or_default();
    put("kernel.lockdown", s(&lockdown));
    put("boot.efi", Value::Bool(root.join("sys/firmware/efi").exists()));
    put("boot.secure_boot", Value::Bool(secure_boot(root).unwrap_or(false)));
    put("boot.initramfs_tool", s(initramfs_tool(root).map(|t| t.0).unwrap_or("")));
    put("session.type", s(&env.session_type));

    let gpus = pci_gpus(root);
    let nv: Vec<&PciGpu> = gpus.iter().filter(|g| g.vendor == NVIDIA).collect();
    put("gpu.count", Value::Num(gpus.len() as f64));
    put("gpu.hybrid", Value::Bool(gpus.len() > 1));
    put("gpu.nvidia", Value::Bool(!nv.is_empty()));
    put("gpu.amd", Value::Bool(gpus.iter().any(|g| g.vendor == AMD)));
    put("gpu.intel", Value::Bool(gpus.iter().any(|g| g.vendor == INTEL)));
    put("gpu.names", s(&gpus.iter().map(|g| g.name.clone()).collect::<Vec<_>>().join("; ")));
    put("nvidia.name", s(nv.first().map(|g| g.name.as_str()).unwrap_or("")));
    put("nvidia.family", s(nv.first().map(|g| nvidia_family(&g.name)).unwrap_or("")));
    put("nvidia.bound_driver", s(nv.first().map(|g| g.driver.as_str()).unwrap_or("")));
    put("amd.bound_driver", s(gpus.iter().find(|g| g.vendor == AMD).map(|g| g.driver.as_str()).unwrap_or("")));

    let modules: Vec<String> = std::fs::read_to_string(root.join("proc/modules"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_whitespace().next().map(str::to_string))
        .collect();
    let loaded = |m: &str| modules.iter().any(|x| x == m);
    put("module.nvidia", Value::Bool(loaded("nvidia")));
    put("module.nvidia_drm", Value::Bool(loaded("nvidia_drm")));
    put("module.nouveau", Value::Bool(loaded("nouveau")));
    put("module.amdgpu", Value::Bool(loaded("amdgpu")));

    let kver = nvidia_kernel_version(root);
    let lver = nvidia_lib_version(root);
    put("nvidia.kernel_version", s(&kver));
    put("nvidia.lib_version", s(&lver));
    let major = |v: &str| v.split('.').next().and_then(|m| m.parse::<f64>().ok()).unwrap_or(0.0);
    put("nvidia.lib_major", Value::Num(major(if lver.is_empty() { &kver } else { &lver })));
    put("nvidia.installed", Value::Bool(!lver.is_empty() || !kver.is_empty()));
    // On Fedora and openSUSE 32-bit libraries live in /usr/lib (64-bit in
    // /usr/lib64); elsewhere /usr/lib is 64-bit.
    let lib32_dirs: Vec<&str> = match distro.family {
        ug_core::distro::PackageFamily::Dnf | ug_core::distro::PackageFamily::Zypper => {
            LIB32.iter().copied().chain(["usr/lib"]).collect()
        }
        _ => LIB32.to_vec(),
    };
    let lib32 = lib32_dirs.iter().any(|d| {
        std::fs::read_dir(root.join(d)).into_iter().flatten().flatten().any(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with("libGLX_nvidia.so") || n.starts_with("libnvidia-glcore.so")
        })
    });
    put("nvidia.lib32", Value::Bool(lib32));

    // Built for the running kernel: in modules.dep (which includes DKMS's
    // updates/ folder) or in DKMS's own tree.
    let deps = std::fs::read_to_string(root.join("lib/modules").join(&kernel).join("modules.dep")).unwrap_or_default();
    let in_deps = deps.lines().any(|l| {
        let m = l.split(':').next().unwrap_or("");
        let file = m.rsplit('/').next().unwrap_or("");
        file.starts_with("nvidia.ko") || file.starts_with("nvidia-current.ko")
    });
    let mut dkms_names = Vec::new();
    let mut dkms_built = false;
    let mut dkms_present = false;
    for e in std::fs::read_dir(root.join("var/lib/dkms")).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.contains("nvidia") {
            continue;
        }
        for v in std::fs::read_dir(e.path()).into_iter().flatten().flatten() {
            if !v.path().is_dir() || v.file_name().to_string_lossy().starts_with("kernel-") {
                continue;
            }
            dkms_present = true;
            dkms_names.push(format!("{name}/{}", v.file_name().to_string_lossy()));
            if !kernel.is_empty() && v.path().join(&kernel).exists() {
                dkms_built = true;
            }
        }
    }
    put("nvidia.module_built", Value::Bool(in_deps || dkms_built));
    put("dkms.nvidia", Value::Bool(dkms_present));
    put("dkms.nvidia_built", Value::Bool(dkms_built));
    put("dkms.nvidia_versions", s(&dkms_names.join(", ")));

    let modprobe = modprobe_lines(root);
    let blacklisted = modprobe.iter().any(|l| l.split_whitespace().collect::<Vec<_>>() == ["blacklist", "nouveau"])
        || cmdline.split_whitespace().any(|w| {
            w == "nouveau.modeset=0"
                || w.strip_prefix("modprobe.blacklist=").is_some_and(|v| v.split(',').any(|m| m == "nouveau"))
                || w.strip_prefix("rd.driver.blacklist=").is_some_and(|v| v.split(',').any(|m| m == "nouveau"))
        });
    put("nouveau.blacklisted", Value::Bool(blacklisted));

    // Root reads the parameter itself; others see what was asked for.
    let param = |p: &str| std::fs::read_to_string(root.join("sys/module/nvidia_drm/parameters").join(p)).ok();
    let modeset = param("modeset").map(|v| yes(&v)).or_else(|| asked_option(&cmdline, &modprobe, "nvidia_drm", "modeset").map(|v| yes(&v)));
    let fbdev = param("fbdev").map(|v| yes(&v)).or_else(|| asked_option(&cmdline, &modprobe, "nvidia_drm", "fbdev").map(|v| yes(&v)));
    put("nvidia.drm_modeset", Value::Bool(modeset.unwrap_or(false)));
    put("nvidia.drm_fbdev", Value::Bool(fbdev.unwrap_or(false)));

    let params = std::fs::read_to_string(root.join("proc/driver/nvidia/params")).unwrap_or_default();
    let preserve = params
        .lines()
        .find_map(|l| l.strip_prefix("PreserveVideoMemoryAllocations:"))
        .map(yes)
        .or_else(|| asked_option(&cmdline, &modprobe, "nvidia", "NVreg_PreserveVideoMemoryAllocations").map(|v| yes(&v)));
    put("nvidia.preserve_vram", Value::Bool(preserve.unwrap_or(false)));
    let units = ["nvidia-suspend.service", "nvidia-resume.service", "nvidia-hibernate.service"];
    put("nvidia.suspend_units_exist", Value::Bool(units.iter().all(|u| unit_exists(root, u))));
    put("nvidia.suspend_units_enabled", Value::Bool(units.iter().all(|u| unit_enabled(root, u))));

    let icds: Vec<String> = ["usr/share/vulkan/icd.d", "etc/vulkan/icd.d"]
        .iter()
        .flat_map(|d| std::fs::read_dir(root.join(d)).into_iter().flatten().flatten())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    put("vulkan.icds", s(&icds.join(", ")));
    put("vulkan.nvidia_icd", Value::Bool(icds.iter().any(|n| n.starts_with("nvidia"))));
    put("vulkan.loader", Value::Bool(LIB64.iter().any(|d| root.join(d).join("libvulkan.so.1").exists())));
    put("vulkan.loader32", Value::Bool(LIB32.iter().any(|d| root.join(d).join("libvulkan.so.1").exists())));

    let dri = ["usr/lib/x86_64-linux-gnu/dri", "usr/lib64/dri", "usr/lib/dri"];
    put("vaapi.nvidia", Value::Bool(any_file(root, &dri, &["nvidia_drv_video.so"])));
    put("vaapi.intel", Value::Bool(any_file(root, &dri, &["iHD_drv_video.so", "i965_drv_video.so"])));
    put("vaapi.amd", Value::Bool(any_file(root, &dri, &["radeonsi_drv_video.so"])));

    let runtimes = flatpak_runtimes(root, env);
    let want = if kver.is_empty() { String::new() } else { format!("org.freedesktop.Platform.GL.nvidia-{}", kver.replace('.', "-")) };
    let apps = |d: PathBuf| std::fs::read_dir(d).into_iter().flatten().count();
    let flatpak_apps = apps(root.join("var/lib/flatpak/app")) + env.home.as_ref().map(|h| apps(h.join(".local/share/flatpak/app"))).unwrap_or(0);
    put("flatpak.apps", Value::Num(flatpak_apps as f64));
    put("flatpak.steam", Value::Bool(flatpak_app(root, env, "com.valvesoftware.Steam")));
    put("flatpak.gl_nvidia", s(&runtimes.iter().filter(|r| r.starts_with("org.freedesktop.Platform.GL.nvidia-")).cloned().collect::<Vec<_>>().join(", ")));
    put("flatpak.gl_nvidia_wanted", s(&want));
    put("flatpak.gl_nvidia_matches", Value::Bool(!want.is_empty() && runtimes.contains(&want)));
    put(
        "steam.native",
        Value::Bool(["usr/bin/steam", "usr/games/steam"].iter().any(|p| root.join(p).exists())),
    );
    put("amd.ppfeaturemask", Value::Bool(cmdline.contains("amdgpu.ppfeaturemask=")));
    f
}
