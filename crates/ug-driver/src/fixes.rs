//! What each fix does: steps Unix Guardian can run and undo, or commands
//! for the user to copy.
//!
//! The roadmap's rule for this phase: nothing runs unless the way back
//! exists and is tested. So only fixes made of *our own files* in
//! `modprobe.d` and `modules-load.d`, units we enable, a module we load, a
//! switch in sysfs we flip, and an initramfs rebuild run automatically:
//! each step records how it is undone, the previous file is backed up, and
//! `ug driver rollback` undoes it from a text console. Installing or
//! removing packages needs a filesystem snapshot to roll back to; until that
//! layer exists those fixes are commands for the user, and the UI says so.

use crate::inventory::{initramfs_tool, Facts, Value};
use ug_hw::SysRoot;
use ug_proto::{DriverPlan, DriverStep};

/// One step, as the executor runs it.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Writes a whole file (absolute path); the old one is backed up first.
    Write { path: String, content: String },
    /// Enables systemd units (and starts them with `now`); undo disables
    /// (and stops) the ones that were not enabled.
    EnableUnits { units: Vec<String>, now: bool },
    /// Writes a value to a kernel switch (absolute sysfs path); undo writes
    /// the old value back.
    SysWrite { path: String, value: String },
    /// Loads a kernel module now; undo unloads it if it was not loaded.
    Modprobe { module: String },
    /// Copies the driver configuration and the package list (always, before
    /// a package change); kept, not undone.
    Backup,
    /// A snapshot of the whole system; kept, and the way back to it is
    /// written to the recovery file.
    Snapshot { tool: crate::snapshot::Tool },
    /// Installs packages with the distribution's package manager (after
    /// refreshing its lists where that is safe); undo removes the packages
    /// it added, and only those.
    Install {
        family: String,
        packages: Vec<String>,
    },
    /// Regenerates the boot loader's configuration; run again on undo,
    /// after the files are put back.
    Regen { argv: Vec<String> },
    /// A swap file for hibernation, swapped on, in fstab, and the kernel
    /// told to resume from it (several undoable actions; see `swap`).
    HibernateSwap { plan: crate::swap::Plan },
    /// A program with its exact opposite (kernelstub -a / -d).
    Command {
        argv: Vec<String>,
        undo: Vec<String>,
    },
    /// Rebuilds the initramfs so a module setting also holds at boot. Run
    /// again on undo.
    Initramfs { argv: Vec<String> },
}

impl Step {
    pub fn describe(&self) -> DriverStep {
        match self {
            Step::Write { path, content } => DriverStep {
                kind: "write-file".into(),
                target: path.clone(),
                detail: content.clone(),
                undo: "The previous file is put back (or this one removed if there was none).".into(),
            },
            Step::EnableUnits { units, now } => {
                let flag = if *now { " --now" } else { "" };
                DriverStep {
                    kind: "enable-units".into(),
                    target: format!("systemctl enable{flag} {}", units.join(" ")),
                    detail: if *now {
                        "Starts these services now and at every start.".into()
                    } else {
                        "Starts these services at sleep, wake and hibernate.".into()
                    },
                    undo: format!("systemctl disable{flag} {} (only those that were off before)", units.join(" ")),
                }
            }
            Step::SysWrite { path, value } => DriverStep {
                kind: "write-switch".into(),
                target: path.clone(),
                detail: format!("Writes {value} (takes effect at once)."),
                undo: "The value it had is written back.".into(),
            },
            Step::Install { family, packages } => DriverStep {
                kind: "install".into(),
                target: crate::packages::install_argv(family, packages).join(" "),
                detail: match crate::packages::refresh_argv(family) {
                    Some(r) => format!("Runs {} first. Needs the internet; takes a few minutes.", r.join(" ")),
                    None => "Needs the internet; takes a few minutes.".into(),
                },
                undo: format!("{} (only the packages this added)", crate::packages::remove_argv(family, &["…".to_string()]).join(" ")),
            },
            Step::Regen { argv } => DriverStep {
                kind: "run".into(),
                target: argv.join(" "),
                detail: "Writes the boot menu from the settings above.".into(),
                undo: "Run again after the settings file is put back.".into(),
            },
            Step::HibernateSwap { plan } => {
                let mut what: Vec<String> = crate::swap::make_argv(plan).iter().map(|a| a.join(" ")).collect();
                what.push(format!("swapon {}", plan.file));
                what.push(format!("/etc/fstab gets: {}", crate::swap::fstab_line(plan)));
                what.push(format!("kernel parameters get: resume=UUID=<this filesystem> resume_offset=<read with {}>", crate::swap::offset_argv(plan).join(" ")));
                DriverStep {
                    kind: "swap".into(),
                    target: format!("{} ({} GB)", plan.file, plan.size_gib),
                    detail: what.join("\n"),
                    undo: "swapoff, the boot settings and fstab put back, the file (and a subvolume made for it) removed.".into(),
                }
            }
            Step::Command { argv, undo } => DriverStep {
                kind: "run".into(),
                target: argv.join(" "),
                detail: "Changes the kernel parameters of every installed kernel.".into(),
                undo: undo.join(" "),
            },
            Step::Backup => DriverStep {
                kind: "backup".into(),
                target: "/var/lib/unix-guardian/backups".into(),
                detail: "Copies modprobe.d, modules-load.d, the boot loader settings, the package sources and the list of installed packages.".into(),
                undo: "Nothing to undo; the copy is kept.".into(),
            },
            Step::Snapshot { tool } => DriverStep {
                kind: "snapshot".into(),
                target: crate::snapshot::create_argv(tool, "<id>", "<fix>").join(" "),
                detail: "A snapshot of the whole system, to go back to if anything goes wrong.".into(),
                undo: "Kept. Going back to it is written to the recovery file (ug driver rollback undoes the change itself).".into(),
            },
            Step::Modprobe { module } => DriverStep {
                kind: "run".into(),
                target: format!("modprobe {module}"),
                detail: "Loads the driver now, so there is no need to restart.".into(),
                undo: format!("modprobe -r {module}, if it was not loaded before"),
            },
            Step::Initramfs { argv } => DriverStep {
                kind: "run".into(),
                target: argv.join(" "),
                detail: "Rebuilds the initramfs so the setting also applies early in boot. Takes up to a minute.".into(),
                undo: "Run again after the files are put back.".into(),
            },
        }
    }
}

/// A fix Unix Guardian can run: its steps, ready for the executor.
#[derive(Debug, Clone)]
pub struct AutoFix {
    pub fix_id: String,
    pub title: String,
    pub steps: Vec<Step>,
    pub needs_reboot: bool,
}

/// Fixes that are files, units, modules and switches only, so they can be
/// undone.
pub const AUTO: &[&str] = &[
    "nouveau-blacklist",
    "nvidia-drm-modeset",
    "nvidia-suspend",
    "rfkill-unblock-wifi",
    "rfkill-unblock-bt",
    "enable-bluetooth",
    "enable-cups",
    "enable-avahi",
    "load-i2c-hid",
    "load-uvcvideo",
];

/// Automatic fixes that change nothing at boot, so no initramfs rebuild and
/// no restart; they also work on image-based systems (`/etc` and sysfs
/// stay writable there).
const LIVE: &[&str] = &[
    "rfkill-unblock-wifi",
    "rfkill-unblock-bt",
    "enable-bluetooth",
    "enable-cups",
    "enable-avahi",
    "load-i2c-hid",
    "load-uvcvideo",
];

pub const FILE_NOUVEAU: &str = "/etc/modprobe.d/unix-guardian-blacklist-nouveau.conf";
pub const FILE_DRM: &str = "/etc/modprobe.d/unix-guardian-nvidia-drm.conf";
pub const FILE_POWER: &str = "/etc/modprobe.d/unix-guardian-nvidia-power.conf";

const HEADER_LIVE: &str = "# Written by Unix Guardian's driver troubleshooter. Undo with\n# `ug driver rollback` (or delete this file).\n";

const HEADER: &str = "# Written by Unix Guardian's driver troubleshooter. Undo with\n# `ug driver rollback` (or delete this file and rebuild the initramfs).\n";

fn s(f: &Facts, k: &str) -> String {
    f.get(k).map(Value::text).unwrap_or_default()
}

fn num(f: &Facts, k: &str) -> f64 {
    match f.get(k) {
        Some(Value::Num(n)) => *n,
        _ => 0.0,
    }
}

fn title(fix_id: &str) -> &'static str {
    match fix_id {
        "nouveau-blacklist" => "Block nouveau",
        "nvidia-drm-modeset" => "Turn on nvidia-drm modesetting",
        "nvidia-suspend" => "Keep video memory across sleep",
        "reboot" => "Restart",
        "install-nvidia-driver" => "Install the NVIDIA driver",
        "install-headers" => "Install the kernel headers",
        "dkms-rebuild" => "Rebuild the driver with DKMS",
        "mok-enroll" => "Enroll the module signing key",
        "install-lib32" => "Install the 32-bit NVIDIA libraries",
        "flatpak-gl" => "Install Flatpak's matching NVIDIA runtime",
        "reinstall-driver" => "Reinstall the NVIDIA driver",
        "update-driver" => "Update the NVIDIA driver",
        "legacy-driver" => "Use a driver for older cards",
        "vaapi" => "Install video decoding for NVIDIA",
        "install-vulkan" => "Install Vulkan",
        "remove-nomodeset" => "Remove nomodeset",
        "amd-ppfeaturemask" => "Unlock AMD tuning",
        "rfkill-unblock-wifi" => "Switch Wi-Fi on",
        "rfkill-unblock-bt" => "Switch Bluetooth on",
        "enable-bluetooth" => "Start the Bluetooth service",
        "enable-cups" => "Start the printing service",
        "enable-avahi" => "Find printers on the network",
        "load-i2c-hid" => "Load the touchpad driver",
        "load-uvcvideo" => "Load the webcam driver",
        "broadcom-wl" => "Install the Broadcom Wi-Fi driver",
        "install-firmware" => "Install the missing firmware",
        "install-sof-firmware" => "Install the sound firmware",
        "start-pipewire" => "Start PipeWire",
        "switch-to-pipewire" => "Switch from PulseAudio to PipeWire",
        "install-bluez" => "Install BlueZ",
        "psmouse-intertouch" => "Turn on the touchpad's full mode",
        "remove-xorg-synaptics" => "Remove the old touchpad driver",
        "install-libcamera" => "Install libcamera",
        "install-cups" => "Install CUPS",
        "install-ipp-usb" => "Install driverless printing over USB",
        "install-hplip" => "Install HP's printer driver",
        "mem-sleep-deep" => "Use deep sleep",
        "hibernate-swap" => "Make swap for hibernation",
        "enable-nonfree-firmware" => "Add Debian's firmware packages",
        _ => "",
    }
}

/// Fixes that change the kernel's boot parameters: (added, removed).
/// `param-add:<p>` and `param-remove:<p>` are the editor's own (`ug driver
/// param`), for any parameter made of plain characters.
fn boot_params(fix_id: &str) -> Option<(Vec<String>, Vec<String>)> {
    let one = |p: &str| vec![p.to_string()];
    let plain = |p: &str| {
        !p.is_empty()
            && p.len() <= 200
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-=,:/+".contains(c))
    };
    Some(match fix_id {
        "remove-nomodeset" => (vec![], one("nomodeset")),
        "amd-ppfeaturemask" => (one("amdgpu.ppfeaturemask=0xffffffff"), vec![]),
        "mem-sleep-deep" => (one("mem_sleep_default=deep"), vec![]),
        "psmouse-intertouch" => (one("psmouse.synaptics_intertouch=1"), vec![]),
        _ => {
            if let Some(p) = fix_id.strip_prefix("param-add:").filter(|p| plain(p)) {
                (one(p), vec![])
            } else if let Some(p) = fix_id.strip_prefix("param-remove:").filter(|p| plain(p)) {
                (vec![], one(p))
            } else {
                return None;
            }
        }
    })
}

/// A boot-parameter fix, the boot loader's way.
fn boot_fix(fix_id: &str, root: &SysRoot, facts: &Facts) -> Result<AutoFix, String> {
    let (add, remove) = boot_params(fix_id).expect("a boot fix");
    let add: Vec<&str> = add.iter().map(String::as_str).collect();
    let remove: Vec<&str> = remove.iter().map(String::as_str).collect();
    let (add, remove) = (&add[..], &remove[..]);
    let title = match fix_id.split_once(':') {
        Some(("param-add", p)) => format!("Add the kernel parameter {p}"),
        Some(("param-remove", p)) => format!("Remove the kernel parameter {p}"),
        _ => title(fix_id).to_string(),
    };
    if !facts.get("distro.can_change").is_some_and(Value::truthy) {
        return Err(format!(
            "{} is image-based (its kernel parameters are set with rpm-ostree kargs or similar); run the commands by hand instead.",
            s(facts, "distro.name")
        ));
    }
    let steps = boot_steps(root, add, remove)?;
    if steps.is_empty() {
        return Err("The boot settings are already that way.".into());
    }
    Ok(AutoFix {
        fix_id: fix_id.into(),
        title,
        steps,
        needs_reboot: true,
    })
}

/// The steps that change the kernel parameters, the boot loader's way.
pub(crate) fn boot_steps(
    root: &SysRoot,
    add: &[&str],
    remove: &[&str],
) -> Result<Vec<Step>, String> {
    let read = |p: &str| {
        std::fs::read_to_string(root.join(p.trim_start_matches('/')))
            .map_err(|e| format!("reading {p}: {e}"))
    };
    let write = |path: &str, new: String, old: &str| {
        (new != old).then(|| Step::Write {
            path: path.into(),
            content: new,
        })
    };
    use crate::bootparams as bp;
    let steps: Vec<Step> = match bp::detect(root) {
        None => return Err("The boot loader (GRUB, systemd-boot, rEFInd or kernelstub) was not recognised, so the parameter has to be changed by hand.".into()),
        Some(bp::Loader::Grub { regen }) => {
            let old = read("/etc/default/grub")?;
            match write("/etc/default/grub", bp::edit_grub(&old, add, remove), &old) {
                Some(w) => vec![w, Step::Regen { argv: regen }],
                None => vec![],
            }
        }
        Some(bp::Loader::SystemdBoot { entries, kernel_cmdline }) => {
            let mut v = Vec::new();
            for e in &entries {
                let old = read(e)?;
                v.extend(write(e, bp::edit_entry(&old, add, remove), &old));
            }
            if kernel_cmdline {
                let old = read("/etc/kernel/cmdline")?;
                v.extend(write("/etc/kernel/cmdline", bp::edit_kernel_cmdline(&old, add, remove), &old));
            }
            v
        }
        Some(bp::Loader::Refind) => {
            let old = read("/boot/refind_linux.conf")?;
            write("/boot/refind_linux.conf", bp::edit_refind(&old, add, remove), &old).into_iter().collect()
        }
        Some(bp::Loader::Kernelstub) => {
            let mut v = Vec::new();
            for a in add {
                v.push(Step::Command { argv: vec!["kernelstub".into(), "-a".into(), a.to_string()], undo: vec!["kernelstub".into(), "-d".into(), a.to_string()] });
            }
            for r in remove {
                v.push(Step::Command { argv: vec!["kernelstub".into(), "-d".into(), r.to_string()], undo: vec!["kernelstub".into(), "-a".into(), r.to_string()] });
            }
            v
        }
    };
    Ok(steps)
}

/// Swap for hibernation: the checks, then backup, the swap file (with its
/// resume parameters, which the executor reads once the file exists) and
/// an initramfs rebuild.
fn hibernate_fix(root: &SysRoot, facts: &Facts) -> Result<AutoFix, String> {
    hibernate_fix_with(root, facts, |d| {
        crate::swap::free_bytes(&root.join(d.trim_start_matches('/')))
    })
}

/// The same, with the free space given (tests).
pub(crate) fn hibernate_fix_with(
    root: &SysRoot,
    facts: &Facts,
    free: impl Fn(&str) -> Option<u64>,
) -> Result<AutoFix, String> {
    let lockdown = s(facts, "kernel.lockdown");
    if !lockdown.is_empty() && lockdown != "none" {
        return Err("The kernel is in lockdown (Secure Boot), which turns hibernation off; more swap would not help.".into());
    }
    if !facts.get("distro.can_change").is_some_and(Value::truthy) {
        return Err(format!(
            "{} is image-based; set up the swap by hand instead.",
            s(facts, "distro.name")
        ));
    }
    if crate::bootparams::detect(root).is_none() {
        return Err("The boot loader was not recognised, so the kernel could not be told where to resume from.".into());
    }
    let Some((tool, argv)) = initramfs_tool(root) else {
        return Err(
            "No initramfs tool was found, so the early boot could not be told to resume.".into(),
        );
    };
    if tool == "mkinitcpio" {
        let conf = std::fs::read_to_string(root.join("etc/mkinitcpio.conf")).unwrap_or_default();
        let hooks = conf
            .lines()
            .find(|l| l.trim_start().starts_with("HOOKS="))
            .unwrap_or("");
        if !hooks
            .split(|c: char| !c.is_alphanumeric() && c != '-')
            .any(|h| h == "resume" || h == "systemd")
        {
            return Err("mkinitcpio's HOOKS have neither resume nor systemd, so the early boot would not resume; add the resume hook after filesystems first.".into());
        }
    }
    let mem = std::fs::read_to_string(root.join("proc/meminfo"))
        .unwrap_or_default()
        .lines()
        .find_map(|l| {
            l.strip_prefix("MemTotal:")
                .and_then(|r| r.split_whitespace().next())
                .and_then(|n| n.parse().ok())
        })
        .unwrap_or(0u64);
    let plan = crate::swap::plan(root, mem, free)?;
    let steps = vec![
        Step::Backup,
        Step::HibernateSwap { plan },
        Step::Initramfs {
            argv: argv.iter().map(|a| a.to_string()).collect(),
        },
    ];
    Ok(AutoFix {
        fix_id: "hibernate-swap".into(),
        title: title("hibernate-swap").into(),
        steps,
        needs_reboot: true,
    })
}

/// Whether Unix Guardian can run a fix itself on this machine.
pub fn can_run(fix_id: &str, facts: &Facts) -> bool {
    let yes = |k: &str| facts.get(k).is_some_and(Value::truthy);
    if LIVE.contains(&fix_id) {
        return true;
    }
    if boot_params(fix_id).is_some() || fix_id == "hibernate-swap" {
        return yes("distro.can_change") && yes("boot.loader");
    }
    if AUTO.contains(&fix_id) {
        return yes("distro.can_change");
    }
    yes("distro.can_change")
        && yes("snapshot.tool")
        && crate::packages::for_fix(fix_id, &s(facts, "distro.family"), facts).is_some()
}

/// A package fix: backup, snapshot, install, and what goes with it.
fn package_fix(
    fix_id: &str,
    root: &SysRoot,
    facts: &Facts,
    packages: Vec<String>,
) -> Result<AutoFix, String> {
    if !facts.get("distro.can_change").is_some_and(Value::truthy) {
        return Err(format!(
            "{} is image-based or not one Unix Guardian can safely change; run the commands by hand instead.",
            s(facts, "distro.name")
        ));
    }
    let family = s(facts, "distro.family");
    let snap = crate::snapshot::detect(root, &family);
    let Some(tool) = snap.tool else {
        let get = if snap.get_one.is_empty() {
            String::new()
        } else {
            format!(" To get one: {}.", snap.get_one.join("; "))
        };
        return Err(format!(
            "{} Unix Guardian installs packages only when it can take a snapshot first, so the commands are shown instead.{get}",
            snap.summary
        ));
    };
    let debian_firmware = family == "apt"
        && s(facts, "distro.id") == "debian"
        && packages.iter().any(|p| p.starts_with("firmware-"));
    if debian_firmware && !facts.get("apt.nonfree_firmware").is_some_and(Value::truthy) {
        return Err("These firmware packages are in Debian's non-free-firmware component, which is not in the package sources yet. Add it first (see that finding).".into());
    }
    let mut steps = vec![
        Step::Backup,
        Step::Snapshot { tool },
        Step::Install { family, packages },
    ];
    match fix_id {
        "install-bluez" => steps.push(Step::EnableUnits {
            units: vec!["bluetooth.service".into()],
            now: true,
        }),
        "install-cups" => steps.push(Step::EnableUnits {
            units: vec!["cups.socket".into()],
            now: true,
        }),
        _ => {}
    }
    let needs_reboot = matches!(
        fix_id,
        "install-firmware" | "install-sof-firmware" | "broadcom-wl" | "install-nvidia-driver"
    );
    Ok(AutoFix {
        fix_id: fix_id.into(),
        title: title(fix_id).into(),
        steps,
        needs_reboot,
    })
}

/// Builds an automatic fix, or says why it cannot run here.
pub fn auto(fix_id: &str, root: &SysRoot, facts: &Facts) -> Result<AutoFix, String> {
    if let Some(p) = crate::packages::for_fix(fix_id, &s(facts, "distro.family"), facts) {
        return package_fix(fix_id, root, facts, p);
    }
    if boot_params(fix_id).is_some() {
        return boot_fix(fix_id, root, facts);
    }
    if fix_id == "hibernate-swap" {
        return hibernate_fix(root, facts);
    }
    if !AUTO.contains(&fix_id) {
        return Err(if title(fix_id).is_empty() {
            format!("Unknown fix `{fix_id}`.")
        } else {
            "Unix Guardian does not run this fix itself here (it needs a repository added by hand, a choice only you can make, or you at the keyboard during boot); its commands are shown to copy.".into()
        });
    }
    if LIVE.contains(&fix_id) {
        return live(fix_id, root);
    }
    if !facts.get("distro.can_change").is_some_and(Value::truthy) {
        return Err(format!(
            "{} is image-based or not one Unix Guardian can safely change; run the commands by hand instead.",
            s(facts, "distro.name")
        ));
    }
    let Some((_, argv)) = initramfs_tool(root) else {
        return Err("No initramfs tool (update-initramfs, mkinitcpio or dracut) was found, so the change could not be made to hold at boot.".into());
    };
    let initramfs = Step::Initramfs {
        argv: argv.iter().map(|a| a.to_string()).collect(),
    };
    let steps = match fix_id {
        "nouveau-blacklist" => vec![
            Step::Write {
                path: FILE_NOUVEAU.into(),
                content: format!("{HEADER}blacklist nouveau\noptions nouveau modeset=0\n"),
            },
            initramfs,
        ],
        "nvidia-drm-modeset" => {
            let fbdev = if num(facts, "nvidia.lib_major") >= 545.0 {
                " fbdev=1"
            } else {
                ""
            };
            vec![
                Step::Write {
                    path: FILE_DRM.into(),
                    content: format!("{HEADER}options nvidia-drm modeset=1{fbdev}\n"),
                },
                initramfs,
            ]
        }
        _ => {
            // The file sorts after the distribution's own nvidia*.conf, so
            // its value wins where both set it.
            let mut steps = vec![Step::Write {
                path: FILE_POWER.into(),
                content: format!("{HEADER}options nvidia NVreg_PreserveVideoMemoryAllocations=1 NVreg_TemporaryFilePath=/var/tmp\n"),
            }];
            steps.push(Step::EnableUnits {
                units: [
                    "nvidia-suspend.service",
                    "nvidia-resume.service",
                    "nvidia-hibernate.service",
                ]
                .map(String::from)
                .to_vec(),
                now: false,
            });
            steps.push(initramfs);
            steps
        }
    };
    Ok(AutoFix {
        fix_id: fix_id.into(),
        title: title(fix_id).into(),
        steps,
        needs_reboot: true,
    })
}

pub const FILE_I2C_HID: &str = "/etc/modules-load.d/unix-guardian-i2c-hid.conf";
pub const FILE_UVC: &str = "/etc/modules-load.d/unix-guardian-uvcvideo.conf";

/// The fixes that take effect at once.
fn live(fix_id: &str, root: &SysRoot) -> Result<AutoFix, String> {
    let units = |u: &[&str]| -> Result<Vec<Step>, String> {
        let missing: Vec<&&str> = u
            .iter()
            .filter(|u| !crate::inventory::unit_exists(root, u))
            .collect();
        if missing.len() == u.len() {
            return Err(format!("{} is not installed.", u.join(" / ")));
        }
        let have = u
            .iter()
            .filter(|u| crate::inventory::unit_exists(root, u))
            .map(|u| u.to_string())
            .collect();
        Ok(vec![Step::EnableUnits {
            units: have,
            now: true,
        }])
    };
    let unblock = |kind: &str| -> Result<Vec<Step>, String> {
        let (_, hard, files) = crate::hardware::rfkill(root, kind);
        if files.is_empty() {
            return Err(if hard {
                "It is switched off by a hardware switch or key, which no program can turn on."
                    .into()
            } else {
                "It is not switched off.".into()
            });
        }
        Ok(files
            .into_iter()
            .map(|path| Step::SysWrite {
                path,
                value: "0".into(),
            })
            .collect())
    };
    let load = |module: &str, file: &str| {
        vec![
            Step::Write {
                path: file.into(),
                content: format!("{HEADER_LIVE}{module}\n"),
            },
            Step::Modprobe {
                module: module.into(),
            },
        ]
    };
    let steps = match fix_id {
        "rfkill-unblock-wifi" => unblock("wlan")?,
        "rfkill-unblock-bt" => unblock("bluetooth")?,
        "enable-bluetooth" => units(&["bluetooth.service"])?,
        "enable-cups" => units(&["cups.socket", "cups.service"])?,
        "enable-avahi" => units(&["avahi-daemon.socket", "avahi-daemon.service"])?,
        "load-i2c-hid" => load("i2c_hid_acpi", FILE_I2C_HID),
        _ => load("uvcvideo", FILE_UVC),
    };
    Ok(AutoFix {
        fix_id: fix_id.into(),
        title: title(fix_id).into(),
        steps,
        needs_reboot: false,
    })
}

/// The packages that carry the firmware this machine misses: from the
/// kernel log when it was read, else from what the file checks found.
pub(crate) fn firmware_packages(facts: &Facts) -> Vec<String> {
    let mut files: Vec<String> = s(facts, "firmware.missing")
        .split(", ")
        .filter(|f| !f.is_empty())
        .map(str::to_string)
        .collect();
    let yes = |k: &str| facts.get(k).is_some_and(Value::truthy);
    for (k, f) in [
        ("firmware.iwlwifi_missing", "iwlwifi-"),
        ("firmware.amdgpu_missing", "amdgpu/"),
        ("firmware.i915_missing", "i915/"),
    ] {
        if yes(k) {
            files.push(f.into());
        }
    }
    if s(facts, "wifi.drivers").contains("rtw") || s(facts, "wifi.pci").contains(" 10ec:") {
        files.push("rtw89/".into());
    }
    let fam = s(facts, "distro.family");
    let mut pkgs = crate::hardware::firmware_packages(&files, &fam);
    if pkgs.is_empty() {
        pkgs.push(match fam.as_str() {
            "apt" => "firmware-linux-nonfree".into(),
            "zypper" => "kernel-firmware-all".into(),
            _ => "linux-firmware".into(),
        });
    }
    pkgs
}

pub(crate) fn is_ubuntu(facts: &Facts) -> bool {
    s(facts, "distro.id") == "ubuntu"
        || s(facts, "distro.like")
            .split_whitespace()
            .any(|l| l == "ubuntu")
}

/// The plan as the user reviews it.
pub fn plan(fix_id: &str, root: &SysRoot, facts: &Facts) -> DriverPlan {
    match auto(fix_id, root, facts) {
        Ok(a) => DriverPlan {
            fix_id: a.fix_id,
            title: a.title,
            steps: a.steps.iter().map(Step::describe).collect(),
            needs_reboot: a.needs_reboot,
            refused: String::new(),
            snapshot: if a.steps.iter().any(|s| matches!(s, Step::Snapshot { .. })) {
                crate::snapshot::detect(root, &s(facts, "distro.family")).summary
            } else {
                String::new()
            },
        },
        Err(e) => DriverPlan {
            fix_id: fix_id.into(),
            title: title(fix_id).into(),
            refused: e,
            ..Default::default()
        },
    }
}

/// Commands for the user, for this distribution family. Automatic fixes get
/// theirs too, for anyone who prefers to run them by hand.
pub fn commands(fix_id: &str, facts: &Facts) -> Vec<String> {
    let fam = s(facts, "distro.family");
    let ubuntu = is_ubuntu(facts);
    let kernel = "$(uname -r)";
    let initramfs = match s(facts, "boot.initramfs_tool").as_str() {
        "mkinitcpio" => "sudo mkinitcpio -P",
        "dracut" => "sudo dracut -f --regenerate-all",
        _ => "sudo update-initramfs -u -k all",
    };
    let by = |apt: &[&str], pacman: &[&str], dnf: &[&str], zypper: &[&str]| -> Vec<String> {
        let v = match fam.as_str() {
            "apt" => apt,
            "pacman" => pacman,
            "dnf" => dnf,
            "zypper" => zypper,
            _ => &[][..],
        };
        v.iter().map(|c| c.to_string()).collect()
    };
    let mut v = match fix_id {
        "reboot" => vec!["systemctl reboot".to_string()],
        "nouveau-blacklist" => vec![
            format!("printf 'blacklist nouveau\\noptions nouveau modeset=0\\n' | sudo tee {FILE_NOUVEAU}"),
            initramfs.into(),
            "systemctl reboot".into(),
        ],
        "nvidia-drm-modeset" => {
            let fbdev = if num(facts, "nvidia.lib_major") >= 545.0 { " fbdev=1" } else { "" };
            vec![
                format!("echo 'options nvidia-drm modeset=1{fbdev}' | sudo tee {FILE_DRM}"),
                initramfs.into(),
                "systemctl reboot".into(),
            ]
        }
        "nvidia-suspend" => vec![
            format!("echo 'options nvidia NVreg_PreserveVideoMemoryAllocations=1 NVreg_TemporaryFilePath=/var/tmp' | sudo tee {FILE_POWER}"),
            "sudo systemctl enable nvidia-suspend.service nvidia-resume.service nvidia-hibernate.service".into(),
            initramfs.into(),
            "systemctl reboot".into(),
        ],
        "install-nvidia-driver" if ubuntu => vec!["sudo ubuntu-drivers install".into()],
        "install-nvidia-driver" => by(
            &["sudo apt install nvidia-driver firmware-misc-nonfree"],
            &["sudo pacman -S --needed nvidia-open-dkms nvidia-utils lib32-nvidia-utils"],
            &[
                "# Fedora's NVIDIA driver comes from RPM Fusion; this enables it",
                "sudo dnf install https://mirrors.rpmfusion.org/free/fedora/rpmfusion-free-release-$(rpm -E %fedora).noarch.rpm https://mirrors.rpmfusion.org/nonfree/fedora/rpmfusion-nonfree-release-$(rpm -E %fedora).noarch.rpm",
                "sudo dnf install akmod-nvidia xorg-x11-drv-nvidia-cuda",
                "# with Secure Boot on, enroll akmods' key, then restart and confirm it in the blue MOK screen:",
                "sudo kmodgenca -a && sudo mokutil --import /etc/pki/akmods/certs/public_key.der",
            ],
            &["# add the NVIDIA repository first (YaST or zypper addrepo)", "sudo zypper install-new-recommends --repo NVIDIA"],
        ),
        "install-headers" => by(
            &[&format!("sudo apt install linux-headers-{kernel}"), "sudo dkms autoinstall"],
            &["sudo pacman -S --needed linux-headers", "sudo dkms autoinstall"],
            &[&format!("sudo dnf install kernel-devel-{kernel}"), "sudo akmods --force"],
            &["sudo zypper install kernel-default-devel"],
        ),
        "dkms-rebuild" if fam == "dnf" => vec!["sudo akmods --force".into(), "sudo dracut -f --regenerate-all".into()],
        "dkms-rebuild" => vec![
            "sudo dkms autoinstall".into(),
            "dkms status".into(),
            "# if it failed, the reason is in /var/lib/dkms/<module>/<version>/build/make.log".into(),
        ],
        "mok-enroll" => match fam.as_str() {
            "dnf" => vec!["sudo mokutil --import /etc/pki/akmods/certs/public_key.der".into(), "systemctl reboot".into()],
            "pacman" => vec!["# Arch: sign with your own keys, see the ArchWiki page Unified_Extensible_Firmware_Interface/Secure_Boot".into()],
            _ => vec!["sudo mokutil --import /var/lib/dkms/mok.pub".into(), "systemctl reboot".into()],
        },
        "install-lib32" => by(
            &["sudo dpkg --add-architecture i386", "sudo apt update", "sudo apt install nvidia-driver-libs:i386"],
            &["# enable [multilib] in /etc/pacman.conf first", "sudo pacman -S --needed lib32-nvidia-utils"],
            &["sudo dnf install xorg-x11-drv-nvidia-libs.i686"],
            &["sudo zypper install nvidia-gl-G06-32bit"],
        ),
        "flatpak-gl" => {
            let want = s(facts, "flatpak.gl_nvidia_wanted");
            let want32 = want.replace("Platform.GL.", "Platform.GL32.");
            vec!["flatpak update".into(), format!("flatpak install flathub {want} {want32}")]
        }
        "reinstall-driver" => by(
            &["sudo apt install --reinstall nvidia-driver nvidia-vulkan-icd", "systemctl reboot"],
            &["sudo pacman -S nvidia-utils", "systemctl reboot"],
            &["sudo dnf reinstall 'xorg-x11-drv-nvidia*'", "systemctl reboot"],
            &["sudo zypper install --force nvidia-video-G06", "systemctl reboot"],
        ),
        "update-driver" => by(
            &["sudo apt update", "apt policy nvidia-driver"],
            &["sudo pacman -Syu"],
            &["sudo dnf upgrade --refresh"],
            &["sudo zypper update"],
        ),
        "legacy-driver" => by(
            &["sudo apt install nvidia-tesla-470-driver"],
            &["# from the AUR: nvidia-470xx-dkms"],
            &["sudo dnf install akmod-nvidia-470xx"],
            &[],
        ),
        "vaapi" => by(
            &["sudo apt install nvidia-vaapi-driver"],
            &["sudo pacman -S --needed libva-nvidia-driver"],
            &["sudo dnf install libva-nvidia-driver"],
            &["sudo zypper install nvidia-vaapi-driver"],
        )
        .into_iter()
        // Per program only: set for the whole session it would send every app's
        // video decoding to the NVIDIA card, the session-wide pattern that
        // broke KDE (see ug_core::session_env).
        .chain(["# then per program, e.g.: LIBVA_DRIVER_NAME=nvidia ug run -- mpv <file>".to_string()])
        .collect(),
        "install-vulkan" => by(
            &["sudo apt install libvulkan1 mesa-vulkan-drivers vulkan-tools"],
            &["sudo pacman -S --needed vulkan-icd-loader lib32-vulkan-icd-loader vulkan-tools"],
            &["sudo dnf install vulkan-loader vulkan-tools mesa-vulkan-drivers"],
            &["sudo zypper install libvulkan1 vulkan-tools"],
        ),
        "rfkill-unblock-wifi" => vec!["rfkill unblock wifi".into()],
        "rfkill-unblock-bt" => vec!["rfkill unblock bluetooth".into()],
        "enable-bluetooth" => vec!["sudo systemctl enable --now bluetooth.service".into()],
        "enable-cups" => vec!["sudo systemctl enable --now cups.socket cups.service".into()],
        "enable-avahi" => vec!["sudo systemctl enable --now avahi-daemon.socket avahi-daemon.service".into()],
        "load-i2c-hid" => vec![format!("echo i2c_hid_acpi | sudo tee {FILE_I2C_HID}"), "sudo modprobe i2c_hid_acpi".into()],
        "load-uvcvideo" => vec![format!("echo uvcvideo | sudo tee {FILE_UVC}"), "sudo modprobe uvcvideo".into()],
        "install-firmware" => {
            let pkgs = firmware_packages(facts);
            let mut v = match fam.as_str() {
                "apt" => vec!["sudo apt update".to_string(), format!("sudo apt install {}", pkgs.join(" "))],
                "pacman" => vec![format!("sudo pacman -S --needed {}", pkgs.join(" "))],
                "dnf" => vec![format!("sudo dnf install {}", pkgs.join(" "))],
                "zypper" => vec![format!("sudo zypper install {}", pkgs.join(" "))],
                _ => vec![],
            };
            if !v.is_empty() {
                v.push("systemctl reboot".into());
            }
            v
        }
        "install-sof-firmware" => by(
            &["sudo apt install firmware-sof-signed", "systemctl reboot"],
            &["sudo pacman -S --needed sof-firmware alsa-ucm-conf", "systemctl reboot"],
            &["sudo dnf install alsa-sof-firmware", "systemctl reboot"],
            &["sudo zypper install sof-firmware", "systemctl reboot"],
        ),
        "broadcom-wl" => by(
            &["sudo apt install broadcom-sta-dkms", "systemctl reboot"],
            &["sudo pacman -S --needed broadcom-wl-dkms", "systemctl reboot"],
            &["# needs RPM Fusion (nonfree) enabled", "sudo dnf install broadcom-wl", "systemctl reboot"],
            &["sudo zypper install broadcom-wl", "systemctl reboot"],
        ),
        "start-pipewire" => vec!["systemctl --user enable --now pipewire.socket pipewire-pulse.socket wireplumber.service".into()],
        "switch-to-pipewire" => vec![
            "systemctl --user disable --now pulseaudio.service pulseaudio.socket".into(),
            "systemctl --user enable --now pipewire.socket pipewire-pulse.socket wireplumber.service".into(),
        ],
        "install-bluez" => by(
            &["sudo apt install bluez", "sudo systemctl enable --now bluetooth.service"],
            &["sudo pacman -S --needed bluez bluez-utils", "sudo systemctl enable --now bluetooth.service"],
            &["sudo dnf install bluez", "sudo systemctl enable --now bluetooth.service"],
            &["sudo zypper install bluez", "sudo systemctl enable --now bluetooth.service"],
        ),
        "remove-xorg-synaptics" => by(
            &["sudo apt remove xserver-xorg-input-synaptics"],
            &["sudo pacman -R xf86-input-synaptics"],
            &["sudo dnf remove xorg-x11-drv-synaptics"],
            &["sudo zypper remove xf86-input-synaptics"],
        ),
        "install-libcamera" => by(
            &["sudo apt install libcamera-tools pipewire-libcamera"],
            &["sudo pacman -S --needed libcamera pipewire-libcamera"],
            &["sudo dnf install libcamera pipewire-plugin-libcamera"],
            &["sudo zypper install libcamera-tools pipewire-libcamera"],
        ),
        "install-cups" => by(
            &["sudo apt install cups ipp-usb", "sudo systemctl enable --now cups.socket"],
            &["sudo pacman -S --needed cups ipp-usb", "sudo systemctl enable --now cups.socket"],
            &["sudo dnf install cups ipp-usb", "sudo systemctl enable --now cups.socket"],
            &["sudo zypper install cups ipp-usb", "sudo systemctl enable --now cups.socket"],
        ),
        "install-ipp-usb" => by(
            &["sudo apt install ipp-usb sane-airscan"],
            &["sudo pacman -S --needed ipp-usb sane-airscan"],
            &["sudo dnf install ipp-usb sane-airscan"],
            &["sudo zypper install ipp-usb sane-airscan"],
        ),
        "install-hplip" => by(
            &["sudo apt install hplip", "hp-setup -i"],
            &["sudo pacman -S --needed hplip", "hp-setup -i"],
            &["sudo dnf install hplip", "hp-setup -i"],
            &["sudo zypper install hplip", "hp-setup -i"],
        ),
        "enable-nonfree-firmware" => vec![
            "# add non-free-firmware after main on each Debian line".into(),
            "sudoedit /etc/apt/sources.list".into(),
            "sudo apt update".into(),
            "sudo apt install firmware-linux-nonfree".into(),
        ],
        "mem-sleep-deep" | "psmouse-intertouch" => {
            let what = if fix_id == "mem-sleep-deep" { "mem_sleep_default=deep" } else { "psmouse.synaptics_intertouch=1" };
            let regen = match fam.as_str() {
                "apt" => "sudo update-grub",
                "dnf" => "sudo grub2-mkconfig -o /boot/grub2/grub.cfg",
                _ => "sudo grub-mkconfig -o /boot/grub/grub.cfg",
            };
            let mut v = Vec::new();
            if fix_id == "mem-sleep-deep" {
                v.push("echo deep | sudo tee /sys/power/mem_sleep   # try it once first".to_string());
            }
            v.extend([format!("# GRUB: add {what} to GRUB_CMDLINE_LINUX_DEFAULT"), "sudoedit /etc/default/grub".into(), regen.into()]);
            v
        }
        "hibernate-swap" => {
            let file = s(facts, "hibernate.plan_file");
            if file.is_empty() {
                crate::swap::guide(None, &s(facts, "hibernate.plan_problem"))
            } else {
                let sub = s(facts, "hibernate.plan_subvolume");
                let p = crate::swap::Plan {
                    file,
                    size_gib: num(facts, "hibernate.plan_gib") as u64,
                    fs: s(facts, "hibernate.plan_fs"),
                    new_subvolume: (!sub.is_empty()).then_some(sub),
                };
                crate::swap::guide(Some(&p), "")
            }
        }
        "remove-nomodeset" | "amd-ppfeaturemask" => {
            let (what, verb) = if fix_id == "amd-ppfeaturemask" {
                ("amdgpu.ppfeaturemask=0xffffffff", "add")
            } else {
                ("nomodeset", "remove")
            };
            let regen = match fam.as_str() {
                "apt" => "sudo update-grub",
                "dnf" => "sudo grub2-mkconfig -o /boot/grub2/grub.cfg",
                _ => "sudo grub-mkconfig -o /boot/grub/grub.cfg",
            };
            vec![
                format!("# GRUB: {verb} {what} in GRUB_CMDLINE_LINUX_DEFAULT"),
                "sudoedit /etc/default/grub".into(),
                regen.into(),
                "systemctl reboot".into(),
            ]
        }
        _ => vec![],
    };
    if v.is_empty() && !title(fix_id).is_empty() {
        v.push(format!(
            "# No commands are known for {}; see the explanation.",
            s(facts, "distro.name")
        ));
    }
    v.retain(|c| !c.is_empty());
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    fn debian(fs: &TestFs) -> Facts {
        fs.write(
            "etc/os-release",
            "ID=debian\nPRETTY_NAME=\"Debian GNU/Linux 13\"\n",
        )
        .write("usr/sbin/update-initramfs", "");
        crate::inventory::facts(&SysRoot::at(fs.root()), &Default::default())
    }

    #[test]
    fn automatic_fixes_are_our_own_files_and_an_initramfs_rebuild() {
        let fs = TestFs::new();
        let mut f = debian(&fs);
        f.insert("nvidia.lib_major".into(), Value::Num(550.0));
        let root = SysRoot::at(fs.root());
        let p = auto("nvidia-drm-modeset", &root, &f).unwrap();
        assert_eq!(
            p.steps[0],
            Step::Write {
                path: FILE_DRM.into(),
                content: format!("{HEADER}options nvidia-drm modeset=1 fbdev=1\n")
            }
        );
        assert_eq!(
            p.steps[1],
            Step::Initramfs {
                argv: ["update-initramfs", "-u", "-k", "all"]
                    .map(String::from)
                    .to_vec()
            }
        );
        assert!(p.needs_reboot);
        f.insert("nvidia.lib_major".into(), Value::Num(535.0));
        let Step::Write { content, .. } = &auto("nvidia-drm-modeset", &root, &f).unwrap().steps[0]
        else {
            panic!()
        };
        assert!(!content.contains("fbdev"), "fbdev=1 only exists from 545");
        for id in AUTO.iter().filter(|i| !LIVE.contains(i)) {
            for step in auto(id, &root, &f).unwrap().steps {
                if let Step::Write { path, .. } = step {
                    assert!(
                        path.starts_with("/etc/modprobe.d/unix-guardian-"),
                        "only files of our own: {path}"
                    );
                }
            }
        }
    }

    #[test]
    fn package_fixes_are_never_run_and_unsafe_systems_get_commands_only() {
        let fs = TestFs::new();
        let f = debian(&fs);
        let root = SysRoot::at(fs.root());
        assert!(auto("install-headers", &root, &f)
            .unwrap_err()
            .contains("commands are shown"));
        assert!(!commands("install-headers", &f).is_empty());
        let mut immutable = f.clone();
        immutable.insert("distro.can_change".into(), Value::Bool(false));
        assert!(auto("nouveau-blacklist", &root, &immutable).is_err());
        assert!(plan("nouveau-blacklist", &root, &immutable)
            .refused
            .contains("by hand"));
        assert!(plan("teleport", &root, &f).refused.contains("Unknown fix"));
    }

    #[test]
    fn live_fixes_need_no_restart_and_say_when_there_is_nothing_to_do() {
        let fs = TestFs::new();
        let f = debian(&fs);
        let root = SysRoot::at(fs.root());
        assert!(auto("rfkill-unblock-wifi", &root, &f)
            .unwrap_err()
            .contains("not switched off"));
        fs.write("sys/class/rfkill/rfkill1/type", "wlan\n")
            .write("sys/class/rfkill/rfkill1/soft", "1\n")
            .write("sys/class/rfkill/rfkill1/hard", "0\n");
        let p = auto("rfkill-unblock-wifi", &root, &f).unwrap();
        assert_eq!(
            p.steps,
            [Step::SysWrite {
                path: "/sys/class/rfkill/rfkill1/soft".into(),
                value: "0".into()
            }]
        );
        assert!(!p.needs_reboot);
        fs.write("sys/class/rfkill/rfkill1/hard", "1\n")
            .write("sys/class/rfkill/rfkill1/soft", "0\n");
        assert!(auto("rfkill-unblock-wifi", &root, &f)
            .unwrap_err()
            .contains("hardware switch"));
        assert!(auto("enable-cups", &root, &f)
            .unwrap_err()
            .contains("not installed"));
        fs.write("usr/lib/systemd/system/cups.socket", "");
        assert_eq!(
            auto("enable-cups", &root, &f).unwrap().steps,
            [Step::EnableUnits {
                units: vec!["cups.socket".into()],
                now: true
            }]
        );
        let Step::Write { path, content } = &auto("load-uvcvideo", &root, &f).unwrap().steps[0]
        else {
            panic!()
        };
        assert!(
            path.starts_with("/etc/modules-load.d/unix-guardian-")
                && content.ends_with("uvcvideo\n")
        );
        for id in LIVE {
            assert!(
                !title(id).is_empty() && !commands(id, &f).is_empty(),
                "{id} has a title and commands"
            );
        }
    }

    #[test]
    fn the_parameter_editor_takes_plain_parameters_only() {
        let fs = TestFs::new();
        let f = debian(&fs);
        let root = SysRoot::at(fs.root());
        fs.write("etc/default/grub", "GRUB_CMDLINE_LINUX_DEFAULT=\"quiet\"\n")
            .write("usr/sbin/update-grub", "");
        let p = auto("param-add:split_lock_detect=off", &root, &f).unwrap();
        assert_eq!(p.title, "Add the kernel parameter split_lock_detect=off");
        assert!(
            matches!(&p.steps[0], Step::Write { content, .. } if content.contains("quiet split_lock_detect=off"))
        );
        assert!(auto("param-remove:absent", &root, &f)
            .unwrap_err()
            .contains("already"));
        for bad in [
            "param-add:a b",
            "param-add:x;reboot",
            "param-add:",
            "param-add:$(id)",
        ] {
            assert!(
                auto(bad, &root, &f).unwrap_err().contains("Unknown fix"),
                "{bad}"
            );
        }
    }
}
