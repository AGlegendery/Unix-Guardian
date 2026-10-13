//! Installing packages through the distribution's own package manager
//! (roadmap 8.4), only ever under a snapshot (see `snapshot`).
//!
//! Commands are argument lists, never a shell line, and run without
//! questions (the user already reviewed the plan). Undo removes exactly the
//! packages the install added, found by comparing the package database
//! before and after; packages that were already there are never removed.

/// The packages a fix installs on this family, or None when Unix Guardian
/// does not install them itself here (the commands are shown instead).
pub fn for_fix(fix_id: &str, family: &str, facts: &crate::Facts) -> Option<Vec<String>> {
    let by =
        |apt: &[&str], pacman: &[&str], dnf: &[&str], zypper: &[&str]| -> Option<Vec<String>> {
            let v = match family {
                "apt" => apt,
                "pacman" => pacman,
                "dnf" => dnf,
                "zypper" => zypper,
                _ => &[][..],
            };
            (!v.is_empty()).then(|| v.iter().map(|s| s.to_string()).collect())
        };
    let ubuntu = crate::fixes::is_ubuntu(facts);
    match fix_id {
        "install-firmware" => Some(crate::fixes::firmware_packages(facts)).filter(|_| family != ""),
        "install-sof-firmware" => by(
            &["firmware-sof-signed"],
            &["sof-firmware", "alsa-ucm-conf"],
            &["alsa-sof-firmware"],
            &["sof-firmware"],
        ),
        "install-bluez" => by(
            &["bluez"],
            &["bluez", "bluez-utils"],
            &["bluez"],
            &["bluez"],
        ),
        "install-cups" => by(
            &["cups", "ipp-usb"],
            &["cups", "ipp-usb"],
            &["cups", "ipp-usb"],
            &["cups", "ipp-usb"],
        ),
        "install-ipp-usb" => by(
            &["ipp-usb", "sane-airscan"],
            &["ipp-usb", "sane-airscan"],
            &["ipp-usb", "sane-airscan"],
            &["ipp-usb", "sane-airscan"],
        ),
        "install-hplip" => by(&["hplip"], &["hplip"], &["hplip"], &["hplip"]),
        "install-libcamera" => by(
            &["libcamera-tools", "pipewire-libcamera"],
            &["libcamera", "pipewire-libcamera"],
            &["libcamera", "pipewire-plugin-libcamera"],
            &["libcamera-tools", "pipewire-libcamera"],
        ),
        "install-vulkan" => by(
            &["libvulkan1", "mesa-vulkan-drivers"],
            &["vulkan-icd-loader", "lib32-vulkan-icd-loader"],
            &["vulkan-loader", "mesa-vulkan-drivers"],
            &["libvulkan1"],
        ),
        "broadcom-wl" => by(&["broadcom-sta-dkms"], &["broadcom-wl-dkms"], &[], &[]),
        "install-headers" => {
            let k = facts
                .get("kernel.release")
                .map(crate::Value::text)
                .unwrap_or_default();
            if k.is_empty() {
                return None;
            }
            by(
                &[&format!("linux-headers-{k}")],
                &[],
                &[&format!("kernel-devel-{k}")],
                &[],
            )
        }
        // Ubuntu picks the right NVIDIA branch with ubuntu-drivers, which is
        // not a package install; Fedora and openSUSE need a repository added
        // first. Those stay commands.
        "install-nvidia-driver" if ubuntu => None,
        "install-nvidia-driver" => by(
            &["nvidia-driver", "firmware-misc-nonfree"],
            &["nvidia-open-dkms", "nvidia-utils"],
            &[],
            &[],
        ),
        _ => None,
    }
}

/// Programs to run before installing (refreshing the package lists).
pub fn refresh_argv(family: &str) -> Option<Vec<String>> {
    // Never `pacman -Sy` alone: installing after a bare refresh is a
    // partial upgrade, which Arch does not support.
    (family == "apt").then(|| vec!["apt-get".into(), "update".into()])
}

pub fn install_argv(family: &str, pkgs: &[String]) -> Vec<String> {
    let head: &[&str] = match family {
        "apt" => &[
            "env",
            "DEBIAN_FRONTEND=noninteractive",
            "apt-get",
            "install",
            "-y",
        ],
        "pacman" => &["pacman", "-S", "--needed", "--noconfirm"],
        "dnf" => &["dnf", "install", "-y"],
        _ => &["zypper", "--non-interactive", "install"],
    };
    head.iter()
        .map(|s| s.to_string())
        .chain(pkgs.iter().cloned())
        .collect()
}

pub fn remove_argv(family: &str, pkgs: &[String]) -> Vec<String> {
    let head: &[&str] = match family {
        "apt" => &[
            "env",
            "DEBIAN_FRONTEND=noninteractive",
            "apt-get",
            "remove",
            "-y",
        ],
        "pacman" => &["pacman", "-Rns", "--noconfirm"],
        "dnf" => &["dnf", "remove", "-y"],
        _ => &["zypper", "--non-interactive", "remove"],
    };
    head.iter()
        .map(|s| s.to_string())
        .chain(pkgs.iter().cloned())
        .collect()
}

/// Packages in `after` that were not in `before`.
pub fn added(before: &[String], after: &[String]) -> Vec<String> {
    after
        .iter()
        .filter(|p| !before.contains(p))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_and_removals_are_argument_lists_without_questions() {
        let p = vec!["bluez".to_string()];
        assert_eq!(
            install_argv("apt", &p),
            [
                "env",
                "DEBIAN_FRONTEND=noninteractive",
                "apt-get",
                "install",
                "-y",
                "bluez"
            ]
        );
        assert_eq!(
            remove_argv("pacman", &p),
            ["pacman", "-Rns", "--noconfirm", "bluez"]
        );
        assert_eq!(refresh_argv("pacman"), None, "no partial upgrades on Arch");
        let before = vec!["a".to_string(), "b".to_string()];
        let after = vec![
            "a".to_string(),
            "b".to_string(),
            "bluez".to_string(),
            "libbluetooth3".to_string(),
        ];
        assert_eq!(added(&before, &after), ["bluez", "libbluetooth3"]);
    }

    #[test]
    fn some_families_and_fixes_stay_commands() {
        let mut f = crate::Facts::new();
        assert_eq!(for_fix("install-bluez", "apt", &f).unwrap(), ["bluez"]);
        assert!(for_fix("install-bluez", "unknown", &f).is_none());
        assert!(
            for_fix("install-nvidia-driver", "dnf", &f).is_none(),
            "needs RPM Fusion first"
        );
        f.insert("distro.id".into(), crate::Value::Str("ubuntu".into()));
        assert!(
            for_fix("install-nvidia-driver", "apt", &f).is_none(),
            "ubuntu-drivers chooses the branch"
        );
        assert!(
            for_fix("mok-enroll", "apt", &f).is_none(),
            "needs the user at boot"
        );
    }
}
