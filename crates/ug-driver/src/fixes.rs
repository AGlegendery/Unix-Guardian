//! What each fix does: steps Unix Guardian can run and undo, or commands
//! for the user to copy.
//!
//! The roadmap's rule for this phase: nothing runs unless the way back
//! exists and is tested. So only fixes made of *our own files* in
//! `modprobe.d`, units we enable, and an initramfs rebuild run automatically:
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
    /// Enables systemd units; undo disables the ones that were not enabled.
    EnableUnits { units: Vec<String> },
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
            Step::EnableUnits { units } => DriverStep {
                kind: "enable-units".into(),
                target: format!("systemctl enable {}", units.join(" ")),
                detail: "Starts these services at sleep, wake and hibernate.".into(),
                undo: format!("systemctl disable {} (only those that were off before)", units.join(" ")),
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

/// Fixes that are files and units only, so they can be undone.
pub const AUTO: &[&str] = &["nouveau-blacklist", "nvidia-drm-modeset", "nvidia-suspend"];

pub const FILE_NOUVEAU: &str = "/etc/modprobe.d/unix-guardian-blacklist-nouveau.conf";
pub const FILE_DRM: &str = "/etc/modprobe.d/unix-guardian-nvidia-drm.conf";
pub const FILE_POWER: &str = "/etc/modprobe.d/unix-guardian-nvidia-power.conf";

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
        _ => "",
    }
}

/// Builds an automatic fix, or says why it cannot run here.
pub fn auto(fix_id: &str, root: &SysRoot, facts: &Facts) -> Result<AutoFix, String> {
    if !AUTO.contains(&fix_id) {
        return Err(if title(fix_id).is_empty() {
            format!("Unknown fix `{fix_id}`.")
        } else {
            "Unix Guardian does not run this fix itself (it installs or removes packages, or needs you at boot); its commands are shown to copy.".into()
        });
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
    let initramfs = Step::Initramfs { argv: argv.iter().map(|a| a.to_string()).collect() };
    let steps = match fix_id {
        "nouveau-blacklist" => vec![
            Step::Write { path: FILE_NOUVEAU.into(), content: format!("{HEADER}blacklist nouveau\noptions nouveau modeset=0\n") },
            initramfs,
        ],
        "nvidia-drm-modeset" => {
            let fbdev = if num(facts, "nvidia.lib_major") >= 545.0 { " fbdev=1" } else { "" };
            vec![Step::Write { path: FILE_DRM.into(), content: format!("{HEADER}options nvidia-drm modeset=1{fbdev}\n") }, initramfs]
        }
        _ => {
            // The file sorts after the distribution's own nvidia*.conf, so
            // its value wins where both set it.
            let mut steps = vec![Step::Write {
                path: FILE_POWER.into(),
                content: format!("{HEADER}options nvidia NVreg_PreserveVideoMemoryAllocations=1 NVreg_TemporaryFilePath=/var/tmp\n"),
            }];
            steps.push(Step::EnableUnits {
                units: ["nvidia-suspend.service", "nvidia-resume.service", "nvidia-hibernate.service"].map(String::from).to_vec(),
            });
            steps.push(initramfs);
            steps
        }
    };
    Ok(AutoFix { fix_id: fix_id.into(), title: title(fix_id).into(), steps, needs_reboot: true })
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
        },
        Err(e) => DriverPlan { fix_id: fix_id.into(), title: title(fix_id).into(), refused: e, ..Default::default() },
    }
}

/// Commands for the user, for this distribution family. Automatic fixes get
/// theirs too, for anyone who prefers to run them by hand.
pub fn commands(fix_id: &str, facts: &Facts) -> Vec<String> {
    let fam = s(facts, "distro.family");
    let ubuntu = s(facts, "distro.id") == "ubuntu" || s(facts, "distro.like").split_whitespace().any(|l| l == "ubuntu");
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
            &["# needs RPM Fusion (nonfree) enabled", "sudo dnf install akmod-nvidia xorg-x11-drv-nvidia-cuda"],
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
        .chain(["echo 'LIBVA_DRIVER_NAME=nvidia' | sudo tee -a /etc/environment".to_string()])
        .collect(),
        "install-vulkan" => by(
            &["sudo apt install libvulkan1 mesa-vulkan-drivers vulkan-tools"],
            &["sudo pacman -S --needed vulkan-icd-loader lib32-vulkan-icd-loader vulkan-tools"],
            &["sudo dnf install vulkan-loader vulkan-tools mesa-vulkan-drivers"],
            &["sudo zypper install libvulkan1 vulkan-tools"],
        ),
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
        v.push(format!("# No commands are known for {}; see the explanation.", s(facts, "distro.name")));
    }
    v.retain(|c| !c.is_empty());
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    fn debian(fs: &TestFs) -> Facts {
        fs.write("etc/os-release", "ID=debian\nPRETTY_NAME=\"Debian GNU/Linux 13\"\n").write("usr/sbin/update-initramfs", "");
        crate::inventory::facts(&SysRoot::at(fs.root()), &Default::default())
    }

    #[test]
    fn automatic_fixes_are_our_own_files_and_an_initramfs_rebuild() {
        let fs = TestFs::new();
        let mut f = debian(&fs);
        f.insert("nvidia.lib_major".into(), Value::Num(550.0));
        let root = SysRoot::at(fs.root());
        let p = auto("nvidia-drm-modeset", &root, &f).unwrap();
        assert_eq!(p.steps[0], Step::Write { path: FILE_DRM.into(), content: format!("{HEADER}options nvidia-drm modeset=1 fbdev=1\n") });
        assert_eq!(p.steps[1], Step::Initramfs { argv: ["update-initramfs", "-u", "-k", "all"].map(String::from).to_vec() });
        assert!(p.needs_reboot);
        f.insert("nvidia.lib_major".into(), Value::Num(535.0));
        let Step::Write { content, .. } = &auto("nvidia-drm-modeset", &root, &f).unwrap().steps[0] else { panic!() };
        assert!(!content.contains("fbdev"), "fbdev=1 only exists from 545");
        for id in AUTO {
            for step in auto(id, &root, &f).unwrap().steps {
                if let Step::Write { path, .. } = step {
                    assert!(path.starts_with("/etc/modprobe.d/unix-guardian-"), "only files of our own: {path}");
                }
            }
        }
    }

    #[test]
    fn package_fixes_are_never_run_and_unsafe_systems_get_commands_only() {
        let fs = TestFs::new();
        let f = debian(&fs);
        let root = SysRoot::at(fs.root());
        assert!(auto("install-headers", &root, &f).unwrap_err().contains("commands are shown"));
        assert!(!commands("install-headers", &f).is_empty());
        let mut immutable = f.clone();
        immutable.insert("distro.can_change".into(), Value::Bool(false));
        assert!(auto("nouveau-blacklist", &root, &immutable).is_err());
        assert!(plan("nouveau-blacklist", &root, &immutable).refused.contains("by hand"));
        assert!(plan("teleport", &root, &f).refused.contains("Unknown fix"));
    }
}
