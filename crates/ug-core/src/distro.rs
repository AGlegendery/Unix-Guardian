//! Which distribution we are on, and which support tier that puts it in.
//!
//! Almost everything Unix Guardian does differs by distribution somewhere:
//! package names, initramfs tooling, whether systemd is even present. The tier
//! is not a judgement about the distribution — it is a promise about what this
//! project will actually do on it, which matters most in the driver
//! troubleshooter, where acting on a system we cannot reliably roll back is
//! worse than declining to act.

use serde::{Deserialize, Serialize};
use ug_hw::SysRoot;

/// How much of Unix Guardian is supported on a distribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tier {
    /// Tested every release; packaged natively; the troubleshooter may make
    /// changes given a snapshot.
    One,
    /// Expected to work and tested occasionally. Immutable systems land here:
    /// monitoring and tuning work, package operations do not.
    Two,
    /// Detected and monitored. Anything that would modify the system is shown
    /// as a command to run by hand rather than executed, because we cannot
    /// verify the rollback path.
    Three,
}

/// The package manager family, which decides how driver operations are shaped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PackageFamily {
    Pacman,
    Apt,
    Dnf,
    Zypper,
    /// Declarative or image-based: the system is not changed by installing a
    /// package, so we only ever print configuration for the user to apply.
    Declarative,
    Unknown,
}

/// A detected distribution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Distro {
    /// `ID` from os-release, e.g. `"debian"`.
    pub id: String,
    /// `ID_LIKE` entries, which is how derivatives are matched without listing
    /// every one of them.
    pub id_like: Vec<String>,
    /// `PRETTY_NAME`, for display.
    pub name: String,
    /// `VERSION_ID`, empty on rolling releases.
    pub version_id: String,
    pub family: PackageFamily,
    pub tier: Tier,
    /// False on systems where systemd is absent, which disables the service and
    /// scheduling features rather than failing them at runtime.
    pub has_systemd: bool,
}

impl Distro {
    /// Reads `/etc/os-release` and classifies the result.
    pub fn detect(root: &SysRoot) -> Self {
        let fields = read_os_release(root);
        let id = fields.get("ID").cloned().unwrap_or_default();
        let id_like: Vec<String> = fields
            .get("ID_LIKE")
            .map(|v| v.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default();

        let family = classify_family(&id, &id_like);
        let tier = classify_tier(&id, &id_like, family);

        Distro {
            name: fields
                .get("PRETTY_NAME")
                .cloned()
                .unwrap_or_else(|| id.clone()),
            version_id: fields.get("VERSION_ID").cloned().unwrap_or_default(),
            has_systemd: root.join("run/systemd/system").exists(),
            id,
            id_like,
            family,
            tier,
        }
    }

    /// True when this distribution's root filesystem is image-based, so package
    /// operations would not persist and must not be attempted.
    pub fn is_immutable(&self) -> bool {
        matches!(self.family, PackageFamily::Declarative)
            || matches!(
                self.id.as_str(),
                "bazzite" | "silverblue" | "kinoite" | "nixos" | "fedora-ostree"
            )
    }

    /// Whether the driver troubleshooter may run package operations at all.
    ///
    /// Tier 3 and immutable systems get diagnosis and copyable commands instead:
    /// the rollback path there is either unverified or not ours to drive.
    pub fn allows_package_changes(&self) -> bool {
        self.tier != Tier::Three && !self.is_immutable()
    }
}

/// Parses `/etc/os-release`, stripping the quoting the format allows.
fn read_os_release(root: &SysRoot) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for path in ["etc/os-release", "usr/lib/os-release"] {
        let Ok(content) = std::fs::read_to_string(root.join(path)) else {
            continue;
        };
        for line in content.lines() {
            if let Some((k, v)) = line.split_once('=') {
                let v = v.trim().trim_matches('"').trim_matches('\'');
                out.insert(k.trim().to_string(), v.to_string());
            }
        }
        if !out.is_empty() {
            break;
        }
    }
    out
}

/// Maps a distribution to its package manager, following `ID_LIKE` so
/// derivatives work without being listed.
fn classify_family(id: &str, id_like: &[String]) -> PackageFamily {
    let matches = |names: &[&str]| {
        names.contains(&id) || id_like.iter().any(|l| names.contains(&l.as_str()))
    };
    if matches(&["nixos", "guix"]) {
        PackageFamily::Declarative
    } else if matches(&["arch", "archlinux", "manjaro", "cachyos", "endeavouros"]) {
        PackageFamily::Pacman
    } else if matches(&["debian", "ubuntu"]) {
        PackageFamily::Apt
    } else if matches(&["fedora", "rhel", "centos"]) {
        PackageFamily::Dnf
    } else if matches(&["opensuse", "suse", "opensuse-tumbleweed", "opensuse-leap"]) {
        PackageFamily::Zypper
    } else {
        PackageFamily::Unknown
    }
}

/// Assigns a support tier.
fn classify_tier(id: &str, id_like: &[String], family: PackageFamily) -> Tier {
    const TIER_ONE: &[&str] = &[
        "arch", "cachyos", "manjaro", "endeavouros", "debian", "parrot", "ubuntu", "pop", "linuxmint",
        "fedora", "nobara",
    ];
    const TIER_TWO: &[&str] = &["opensuse-tumbleweed", "opensuse-leap", "bazzite", "silverblue", "kinoite"];

    if TIER_ONE.contains(&id) {
        return Tier::One;
    }
    if TIER_TWO.contains(&id) {
        return Tier::Two;
    }
    // An unlisted derivative inherits its parent's tier one step down: it is
    // probably fine, but nobody has checked it.
    if id_like.iter().any(|l| TIER_ONE.contains(&l.as_str())) {
        return Tier::Two;
    }
    match family {
        PackageFamily::Unknown | PackageFamily::Declarative => Tier::Three,
        _ => Tier::Three,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    fn os_release(content: &str) -> TestFs {
        let fs = TestFs::new();
        fs.write("etc/os-release", content);
        fs.dir("run/systemd/system");
        fs
    }

    #[test]
    fn detects_the_development_machine() {
        let fs = os_release(
            r#"PRETTY_NAME="Parrot Security 7.3 (echo)"
ID=parrot
ID_LIKE=debian
VERSION_ID="7.3""#,
        );
        let d = Distro::detect(&SysRoot::at(fs.root()));

        assert_eq!(d.id, "parrot");
        assert_eq!(d.family, PackageFamily::Apt);
        assert_eq!(d.tier, Tier::One);
        assert!(d.has_systemd);
        assert!(d.allows_package_changes());
    }

    #[test]
    fn derivatives_are_matched_through_id_like() {
        let fs = os_release("PRETTY_NAME=\"CachyOS\"\nID=cachyos\nID_LIKE=arch");
        let d = Distro::detect(&SysRoot::at(fs.root()));
        assert_eq!(d.family, PackageFamily::Pacman);
        assert_eq!(d.tier, Tier::One);
    }

    #[test]
    fn an_unknown_arch_derivative_is_tier_two_not_tier_one() {
        let fs = os_release("PRETTY_NAME=\"SomeOS\"\nID=someos\nID_LIKE=arch");
        let d = Distro::detect(&SysRoot::at(fs.root()));
        assert_eq!(d.family, PackageFamily::Pacman, "packaging is still pacman");
        assert_eq!(d.tier, Tier::Two, "nobody has tested this one");
    }

    #[test]
    fn immutable_systems_do_not_allow_package_changes() {
        let fs = os_release("PRETTY_NAME=\"Bazzite\"\nID=bazzite\nID_LIKE=fedora");
        let d = Distro::detect(&SysRoot::at(fs.root()));
        assert!(d.is_immutable());
        assert!(
            !d.allows_package_changes(),
            "an rpm-ostree system must not be touched with dnf"
        );
    }

    #[test]
    fn nixos_is_declarative_and_read_only_to_us() {
        let fs = os_release("PRETTY_NAME=\"NixOS 25.05\"\nID=nixos");
        let d = Distro::detect(&SysRoot::at(fs.root()));
        assert_eq!(d.family, PackageFamily::Declarative);
        assert_eq!(d.tier, Tier::Three);
        assert!(!d.allows_package_changes());
    }

    #[test]
    fn a_system_without_systemd_is_detected_not_assumed() {
        let fs = TestFs::new();
        fs.write("etc/os-release", "PRETTY_NAME=\"Void Linux\"\nID=void");
        let d = Distro::detect(&SysRoot::at(fs.root()));
        assert!(!d.has_systemd);
        assert_eq!(d.tier, Tier::Three);
    }

    #[test]
    fn a_machine_with_no_os_release_does_not_panic() {
        let fs = TestFs::new();
        let d = Distro::detect(&SysRoot::at(fs.root()));
        assert_eq!(d.id, "");
        assert_eq!(d.tier, Tier::Three);
        assert!(!d.allows_package_changes());
    }
}

#[cfg(test)]
mod replay {
    use super::*;

    #[test]
    fn the_captured_machine_is_tier_one_debian_family() {
        let d = Distro::detect(&SysRoot::at(ug_testfs::fixture("dev-laptop-rtx3050")));
        assert_eq!(d.id, "parrot");
        assert_eq!(d.family, PackageFamily::Apt);
        assert_eq!(d.tier, Tier::One);
    }
}
