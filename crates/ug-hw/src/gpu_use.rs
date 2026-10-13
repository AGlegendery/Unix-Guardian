//! Which GPU a process renders on, from the device files it holds open.
//!
//! Opening a render node proves little: browsers and Electron apps open every
//! GPU's node just to list them. What proves use is (1) the NVIDIA driver's
//! own node `/dev/nvidia0`, which only a program really using that card
//! opens, or (2) a DRM file whose `fdinfo` shows engine time (`drm-engine-*`
//! above zero; i915, xe and amdgpu report it). Neither asks the GPU itself,
//! so a sleeping discrete GPU stays asleep.

use crate::SysRoot;
use std::collections::BTreeSet;

/// PCI addresses of the GPUs `pid` has open, e.g. `0000:01:00.0`. Empty when
/// its file descriptors cannot be read (another user's process).
pub fn gpus_of(root: &SysRoot, pid: i32) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Ok(dir) = std::fs::read_dir(root.join(format!("proc/{pid}/fd"))) else {
        return out;
    };
    for e in dir.flatten() {
        let Ok(target) = std::fs::read_link(e.path()) else {
            continue;
        };
        let Some(t) = target.to_str() else { continue };
        if t.starts_with("/dev/dri/") {
            let fd = e.file_name();
            let info = std::fs::read_to_string(
                root.join(format!("proc/{pid}/fdinfo/{}", fd.to_string_lossy())),
            )
            .unwrap_or_default();
            if let Some(pci) = busy_pdev(&info) {
                out.insert(pci);
            }
        } else if let Some(minor) = t
            .strip_prefix("/dev/nvidia")
            .and_then(|m| m.parse::<u32>().ok())
        {
            if let Some(pci) = nvidia_pci(root, minor) {
                out.insert(pci);
            }
        }
    }
    out
}

/// The PCI device of a DRM file that has done work: `drm-pdev` when some
/// `drm-engine-*` counter is above zero.
fn busy_pdev(fdinfo: &str) -> Option<String> {
    let mut pdev = None;
    let mut busy = false;
    for l in fdinfo.lines() {
        if let Some(v) = l.strip_prefix("drm-pdev:") {
            pdev = Some(v.trim().to_string());
        } else if l.starts_with("drm-engine-") && !l.starts_with("drm-engine-capacity") {
            let n: u64 = l
                .split(':')
                .nth(1)
                .and_then(|v| v.split_whitespace().next()?.parse().ok())
                .unwrap_or(0);
            busy |= n > 0;
        }
    }
    pdev.filter(|_| busy)
}

/// The PCI address of `/dev/nvidia<minor>`, from the driver's own listing.
fn nvidia_pci(root: &SysRoot, minor: u32) -> Option<String> {
    for e in std::fs::read_dir(root.join("proc/driver/nvidia/gpus"))
        .ok()?
        .flatten()
    {
        let info = std::fs::read_to_string(e.path().join("information")).unwrap_or_default();
        let m = info
            .lines()
            .find_map(|l| l.strip_prefix("Device Minor:")?.trim().parse::<u32>().ok());
        if m == Some(minor) {
            return e.file_name().to_str().map(str::to_string);
        }
    }
    None
}

/// The GPU a set of processes (a game) renders on, preferring one that is
/// not the boot display when they use both (an app offloaded to the discrete
/// GPU still uses the integrated one to present its frames).
pub fn gpu_of_game(root: &SysRoot, pids: &[i32], boot_vga: &str) -> Option<String> {
    let mut all = BTreeSet::new();
    for &p in pids {
        all.extend(gpus_of(root, p));
    }
    all.iter()
        .find(|g| g.as_str() != boot_vga)
        .or_else(|| all.iter().next())
        .cloned()
}

/// A display GPU the machine has, as the kernel lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gpu {
    pub pci: String,
    pub vendor: u16,
    pub device: u16,
    /// The firmware showed the boot screen on it: the integrated GPU on a
    /// hybrid laptop.
    pub boot_vga: bool,
    /// The kernel driver, e.g. `nvidia`, `amdgpu`, `i915`, `xe`.
    pub driver: String,
}

impl Gpu {
    pub fn vendor_name(&self) -> &'static str {
        match self.vendor {
            0x10de => "nvidia",
            0x1002 => "amd",
            0x8086 => "intel",
            _ => "other",
        }
    }
}

/// Every GPU with a DRM card, boot display first.
pub fn gpus(root: &SysRoot) -> Vec<Gpu> {
    let mut v: Vec<Gpu> = std::fs::read_dir(root.join("sys/class/drm"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with("card") && !n.contains('-')
        })
        .filter_map(|e| {
            let dev = e.path().join("device");
            let pci = std::fs::read_link(&dev)
                .ok()?
                .file_name()?
                .to_str()?
                .to_string();
            let hex = |f: &str| {
                std::fs::read_to_string(dev.join(f))
                    .ok()
                    .and_then(|s| u16::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok())
            };
            let driver = std::fs::read_link(dev.join("driver"))
                .ok()
                .and_then(|l| l.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_default();
            Some(Gpu {
                pci,
                vendor: hex("vendor")?,
                device: hex("device").unwrap_or(0),
                boot_vga: std::fs::read_to_string(dev.join("boot_vga"))
                    .is_ok_and(|b| b.trim() == "1"),
                driver,
            })
        })
        .collect();
    v.sort_by(|a, b| b.boot_vga.cmp(&a.boot_vga).then(a.pci.cmp(&b.pci)));
    v.dedup_by(|a, b| a.pci == b.pci);
    v
}

/// Finds a GPU by `discrete`, `integrated`, a vendor (`nvidia`, `amd`,
/// `intel`) or a PCI address.
pub fn pick<'a>(gpus: &'a [Gpu], want: &str) -> Option<&'a Gpu> {
    let w = want.trim().to_lowercase();
    match w.as_str() {
        "discrete" | "dgpu" => gpus.iter().find(|g| !g.boot_vga),
        "integrated" | "igpu" => gpus.iter().find(|g| g.boot_vga),
        "nvidia" | "amd" | "intel" => gpus.iter().find(|g| g.vendor_name() == w),
        _ => gpus.iter().find(|g| g.pci == w || g.pci.ends_with(&w)),
    }
}

/// The environment that makes a program render on `gpu` (PRIME render
/// offload): NVIDIA's own variables for its proprietary driver, Mesa's
/// `DRI_PRIME` (which also steers Vulkan through the device-select layer)
/// for everything else. For the integrated GPU, Vulkan is pinned to it too:
/// programs such as vkcube otherwise pick the discrete card on their own.
pub fn offload_env(root: &SysRoot, gpu: &Gpu) -> Vec<(String, String)> {
    let dri = (
        "DRI_PRIME".to_string(),
        format!("pci-{}", gpu.pci.replace([':', '.'], "_")),
    );
    if gpu.driver == "nvidia" {
        [
            ("__NV_PRIME_RENDER_OFFLOAD", "1"),
            ("__GLX_VENDOR_LIBRARY_NAME", "nvidia"),
            ("__VK_LAYER_NV_optimus", "NVIDIA_only"),
        ]
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .to_vec()
    } else if gpu.boot_vga {
        vec![
            dri,
            // Mesa's device-select layer: this GPU only (`!`), which also
            // keeps Vulkan programs off the NVIDIA card.
            (
                "MESA_VK_DEVICE_SELECT".into(),
                format!("{:04x}:{:04x}!", gpu.vendor, gpu.device),
            ),
            ("__GLX_VENDOR_LIBRARY_NAME".into(), "mesa".into()),
        ]
        .into_iter()
        .chain(mesa_icds(root).map(|files| ("VK_DRIVER_FILES".to_string(), files)))
        .collect()
    } else {
        vec![dri]
    }
}

/// Every Vulkan driver except NVIDIA's, so a program meant for the
/// integrated GPU never loads NVIDIA's (which opens, and wakes, the card).
fn mesa_icds(root: &SysRoot) -> Option<String> {
    let mut v: Vec<String> = ["usr/share/vulkan/icd.d", "etc/vulkan/icd.d"]
        .iter()
        .flat_map(|d| {
            std::fs::read_dir(root.join(d))
                .into_iter()
                .flatten()
                .flatten()
        })
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_lowercase();
            n.ends_with(".json") && !n.contains("nvidia")
        })
        .map(|e| {
            format!(
                "/{}",
                e.path()
                    .strip_prefix(root.join(""))
                    .unwrap_or(&e.path())
                    .display()
            )
        })
        .collect();
    v.sort();
    (!v.is_empty()).then(|| v.join(":"))
}

/// The same as a line for a game's launch options in Steam.
pub fn launch_options(root: &SysRoot, gpu: &Gpu) -> String {
    let env: Vec<String> = offload_env(root, gpu)
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    format!("{} %command%", env.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use ug_testfs::TestFs;

    fn laptop() -> TestFs {
        let fs = TestFs::new();
        for (node, pci) in [
            ("renderD128", "0000:00:02.0"),
            ("renderD129", "0000:01:00.0"),
            ("card0", "0000:00:02.0"),
        ] {
            fs.dir(format!("sys/devices/pci0000:00/{pci}"))
                .dir(format!("sys/class/drm/{node}"));
            symlink(
                format!("../../../devices/pci0000:00/{pci}"),
                fs.root().join(format!("sys/class/drm/{node}/device")),
            )
            .unwrap();
        }
        fs.write(
            "proc/driver/nvidia/gpus/0000:01:00.0/information",
            "Model: RTX 3050\nDevice Minor: \t 0\n",
        );
        fs
    }

    fn open(fs: &TestFs, pid: i32, fd: u32, target: &str) {
        fs.dir(format!("proc/{pid}/fd"));
        symlink(target, fs.root().join(format!("proc/{pid}/fd/{fd}"))).unwrap();
    }

    /// A DRM file with engine time, as i915 or amdgpu report it.
    fn used(fs: &TestFs, pid: i32, fd: u32, pci: &str, ns: u64) {
        fs.write(
            format!("proc/{pid}/fdinfo/{fd}"),
            &format!("pos:\t0\ndrm-driver:\ti915\ndrm-pdev:\t{pci}\ndrm-engine-render:\t{ns} ns\ndrm-engine-copy:\t0 ns"),
        );
    }

    #[test]
    fn a_process_on_the_integrated_gpu() {
        let fs = laptop();
        open(&fs, 10, 3, "/dev/dri/renderD128");
        used(&fs, 10, 3, "0000:00:02.0", 6_826_769);
        open(&fs, 10, 4, "/dev/null");
        let root = SysRoot::at(fs.root());
        assert_eq!(gpus_of(&root, 10), ["0000:00:02.0".to_string()].into());
        assert_eq!(
            gpu_of_game(&root, &[10], "0000:00:02.0").as_deref(),
            Some("0000:00:02.0")
        );
    }

    #[test]
    fn an_offloaded_game_counts_as_on_the_discrete_gpu() {
        let fs = laptop();
        // PRIME offload: renders on NVIDIA, presents through Intel.
        open(&fs, 20, 3, "/dev/dri/card0");
        used(&fs, 20, 3, "0000:00:02.0", 5_000);
        open(&fs, 20, 4, "/dev/nvidia0");
        open(&fs, 20, 5, "/dev/nvidiactl");
        let root = SysRoot::at(fs.root());
        assert_eq!(gpus_of(&root, 20).len(), 2, "nvidiactl is not a GPU");
        assert_eq!(
            gpu_of_game(&root, &[20, 21], "0000:00:02.0").as_deref(),
            Some("0000:01:00.0")
        );
    }

    #[test]
    fn a_browser_listing_every_gpu_uses_only_the_one_it_draws_on() {
        let fs = laptop();
        // Chrome's GPU process opens both render nodes; only Intel did work.
        open(&fs, 30, 7, "/dev/dri/renderD128");
        used(&fs, 30, 7, "0000:00:02.0", 99);
        open(&fs, 30, 8, "/dev/dri/renderD129");
        fs.write("proc/30/fdinfo/8", "pos:\t0\nflags:\t02100002");
        let root = SysRoot::at(fs.root());
        assert_eq!(
            gpu_of_game(&root, &[30], "0000:00:02.0").as_deref(),
            Some("0000:00:02.0")
        );
    }

    fn card(fs: &TestFs, n: u32, pci: &str, vendor: &str, boot: bool, driver: &str) {
        let dev = format!("sys/devices/pci0000:00/{pci}");
        fs.write(format!("{dev}/vendor"), vendor)
            .write(format!("{dev}/device"), "0x25a2")
            .write(format!("{dev}/boot_vga"), if boot { "1" } else { "0" });
        fs.dir(format!("sys/bus/pci/drivers/{driver}"))
            .dir(format!("sys/class/drm/card{n}"));
        symlink(
            format!("../../../../bus/pci/drivers/{driver}"),
            fs.root().join(format!("{dev}/driver")),
        )
        .unwrap();
        symlink(
            format!("../../../devices/pci0000:00/{pci}"),
            fs.root().join(format!("sys/class/drm/card{n}/device")),
        )
        .unwrap();
    }

    #[test]
    fn offload_uses_nvidias_variables_or_dri_prime() {
        let fs = TestFs::new();
        card(&fs, 0, "0000:00:02.0", "0x8086", true, "i915");
        card(&fs, 1, "0000:01:00.0", "0x10de", false, "nvidia");
        card(&fs, 2, "0000:03:00.0", "0x1002", false, "amdgpu");
        let root = SysRoot::at(fs.root());
        fs.write("usr/share/vulkan/icd.d/intel_icd.json", "{}")
            .write("usr/share/vulkan/icd.d/nvidia_icd.json", "{}");
        let g = gpus(&root);
        assert_eq!(g.len(), 3);
        assert!(g[0].boot_vga);
        let nv = pick(&g, "discrete").unwrap();
        assert_eq!(nv.driver, "nvidia");
        assert_eq!(launch_options(&root, nv), "__NV_PRIME_RENDER_OFFLOAD=1 __GLX_VENDOR_LIBRARY_NAME=nvidia __VK_LAYER_NV_optimus=NVIDIA_only %command%");
        let amd = pick(&g, "amd").unwrap();
        assert_eq!(
            offload_env(&root, amd),
            vec![("DRI_PRIME".to_string(), "pci-0000_03_00_0".to_string())]
        );
        let igpu = offload_env(&root, pick(&g, "integrated").unwrap());
        assert!(
            igpu.contains(&(
                "VK_DRIVER_FILES".to_string(),
                "/usr/share/vulkan/icd.d/intel_icd.json".to_string()
            )),
            "{igpu:?}"
        );
        assert!(igpu.contains(&(
            "MESA_VK_DEVICE_SELECT".to_string(),
            "8086:25a2!".to_string()
        )));
        assert!(igpu.contains(&("DRI_PRIME".to_string(), "pci-0000_00_02_0".to_string())));
        assert_eq!(pick(&g, "01:00.0").unwrap().vendor_name(), "nvidia");
        assert!(pick(&g, "matrox").is_none());
    }

    #[test]
    fn nothing_open_or_unreadable_is_none() {
        let fs = laptop();
        let root = SysRoot::at(fs.root());
        assert!(gpu_of_game(&root, &[99], "0000:00:02.0").is_none());
    }
}
