//! The safety net under package changes (roadmap 8.5).
//!
//! Before Unix Guardian installs or removes a package it takes a snapshot of
//! the whole system with the tool this machine already has, and keeps a
//! light copy of the configuration that drivers touch. Without a snapshot
//! tool, package changes are not made at all: the page shows the commands
//! and suggests installing Timeshift (the golden rule of P8).
//!
//! In order of preference: snapper, Timeshift, ZFS, and a plain read-only
//! Btrfs snapshot of the root subvolume when the root is Btrfs and
//! `btrfs-progs` is there. LVM snapshots are not taken: they need free
//! space in the volume group that cannot be checked safely beforehand.
//!
//! Detection reads files only; taking a snapshot runs the tool through the
//! executor's runner.

use ug_hw::SysRoot;

/// A snapshot tool and what it needs to know.
#[derive(Debug, Clone, PartialEq)]
pub enum Tool {
    Snapper,
    Timeshift,
    /// The root dataset, e.g. `rpool/ROOT/debian`.
    Zfs { dataset: String },
    /// Root on Btrfs: its device and the subvolume mounted at `/`.
    Btrfs { device: String, subvol: String },
}

/// What protects a package change here.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub tool: Option<Tool>,
    /// In a sentence, for the review panel.
    pub summary: String,
    /// Commands that would give this machine a snapshot tool.
    pub get_one: Vec<String>,
}

/// Where plain Btrfs snapshots go: inside the root subvolume, which a
/// snapshot does not include, so they do not pile up inside each other.
pub const BTRFS_DIR: &str = "/var/lib/unix-guardian/snapshots";

fn has(root: &SysRoot, p: &str) -> bool {
    ["usr/bin", "usr/sbin", "bin", "sbin"].iter().any(|d| root.join(d).join(p).exists())
}

/// The root mount from `/proc/mounts`: (device, type, options).
fn root_mount(root: &SysRoot) -> Option<(String, String, String)> {
    let t = std::fs::read_to_string(root.join("proc/mounts")).ok()?;
    t.lines().rev().find_map(|l| {
        let w: Vec<&str> = l.split_whitespace().collect();
        (w.len() >= 4 && w[1] == "/").then(|| (w[0].to_string(), w[2].to_string(), w[3].to_string()))
    })
}

pub fn detect(root: &SysRoot, family: &str) -> Status {
    let mount = root_mount(root);
    let fs = mount.as_ref().map(|m| m.1.as_str()).unwrap_or("");
    let get_one = match family {
        "apt" => vec!["sudo apt install timeshift".to_string()],
        "pacman" => vec!["sudo pacman -S --needed timeshift".to_string()],
        "dnf" => vec!["sudo dnf install timeshift".to_string()],
        "zypper" => vec!["sudo zypper install snapper".to_string()],
        _ => vec![],
    };
    let tool = if has(root, "snapper") && root.join("etc/snapper/configs/root").exists() {
        Some(Tool::Snapper)
    } else if has(root, "timeshift") && root.join("etc/timeshift/timeshift.json").exists() {
        Some(Tool::Timeshift)
    } else if fs == "zfs" && has(root, "zfs") {
        mount.as_ref().map(|m| Tool::Zfs { dataset: m.0.clone() })
    } else if fs == "btrfs" && has(root, "btrfs") {
        let opts = mount.as_ref().map(|m| m.2.clone()).unwrap_or_default();
        let subvol = opts.split(',').find_map(|o| o.strip_prefix("subvol=")).unwrap_or("/").to_string();
        mount.as_ref().map(|m| Tool::Btrfs { device: m.0.clone(), subvol })
    } else {
        None
    };
    let summary = match &tool {
        Some(Tool::Snapper) => "A snapper snapshot of the system is taken first.".to_string(),
        Some(Tool::Timeshift) => "A Timeshift snapshot of the system is taken first.".to_string(),
        Some(Tool::Zfs { dataset }) => format!("A ZFS snapshot of {dataset} is taken first."),
        Some(Tool::Btrfs { subvol, .. }) => {
            format!("A read-only Btrfs snapshot of the root subvolume ({subvol}) is taken first, into {BTRFS_DIR}.")
        }
        None if fs == "btrfs" => {
            "The system is on Btrfs, but no snapshot tool (snapper, Timeshift or btrfs-progs) is installed, so there is no way back from a package change.".to_string()
        }
        None => "No snapshot tool (snapper, Timeshift, ZFS) is set up, so there is no way back from a package change.".to_string(),
    };
    Status { tool, summary, get_one }
}

/// The command that takes a snapshot named after operation `id`.
pub fn create_argv(tool: &Tool, id: &str, title: &str) -> Vec<String> {
    let label = format!("Unix Guardian: before {title} ({id})");
    let v: Vec<String> = match tool {
        Tool::Snapper => vec![
            "snapper", "-c", "root", "create", "--type", "single", "--cleanup-algorithm", "number", "--print-number",
            "--description", &label,
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        Tool::Timeshift => ["timeshift", "--create", "--scripted", "--tags", "O", "--comments", &label].into_iter().map(String::from).collect(),
        Tool::Zfs { dataset } => vec!["zfs".into(), "snapshot".into(), format!("{dataset}@unix-guardian-{id}")],
        Tool::Btrfs { .. } => vec!["btrfs".into(), "subvolume".into(), "snapshot".into(), "-r".into(), "/".into(), format!("{BTRFS_DIR}/{id}")],
    };
    v
}

/// The snapshot's name from the tool's output (snapper prints its number,
/// Timeshift "Tagged snapshot '2026-10-03_18-00-01': ondemand").
pub fn created_name(tool: &Tool, id: &str, output: &str) -> String {
    match tool {
        Tool::Snapper => output.split_whitespace().rev().find(|w| w.chars().all(|c| c.is_ascii_digit())).unwrap_or("").to_string(),
        Tool::Timeshift => output
            .lines()
            .find_map(|l| l.split("snapshot '").nth(1).and_then(|r| r.split('\'').next()))
            .unwrap_or("")
            .to_string(),
        Tool::Zfs { dataset } => format!("{dataset}@unix-guardian-{id}"),
        Tool::Btrfs { .. } => format!("{BTRFS_DIR}/{id}"),
    }
}

/// How to go back to the snapshot by hand, for the recovery file (a text
/// console or a live USB, when nothing else works).
pub fn restore_commands(tool: &Tool, name: &str) -> Vec<String> {
    match tool {
        Tool::Snapper => vec![
            format!("sudo snapper -c root rollback {name}"),
            "systemctl reboot".into(),
        ],
        Tool::Timeshift => vec![format!("sudo timeshift --restore --snapshot '{name}'")],
        Tool::Zfs { .. } => vec![format!("sudo zfs rollback -r {name}"), "systemctl reboot".into()],
        Tool::Btrfs { device, subvol } => {
            let sv = subvol.trim_start_matches('/');
            if sv.is_empty() {
                // The root is the top-level subvolume: it cannot be swapped,
                // only copied back from.
                return vec![format!("# the snapshot is {name}; copy files back from it, e.g. sudo cp -a {name}/etc/<file> /etc/")];
            }
            vec![
                format!("sudo mount -o subvolid=5 {device} /mnt"),
                format!("sudo mv /mnt/{sv} /mnt/{sv}.broken"),
                format!("sudo btrfs subvolume snapshot /mnt/{sv}.broken{name} /mnt/{sv}"),
                "systemctl reboot".into(),
            ]
        }
    }
}

/// Configuration a driver change can touch, copied before every change
/// even with a snapshot (relative to the root).
pub const CONFIG: &[&str] = &[
    "etc/modprobe.d",
    "etc/modules-load.d",
    "etc/default/grub",
    "etc/mkinitcpio.conf",
    "etc/dracut.conf.d",
    "etc/X11/xorg.conf",
    "etc/X11/xorg.conf.d",
    "etc/apt/sources.list",
    "etc/apt/sources.list.d",
    "etc/kernel/cmdline",
    "boot/loader/entries",
];

/// The installed packages, read from the package database's files.
pub fn installed_packages(root: &SysRoot) -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(t) = std::fs::read_to_string(root.join("var/lib/dpkg/status")) {
        let mut name = None;
        for l in t.lines() {
            if let Some(n) = l.strip_prefix("Package: ") {
                name = Some(n.to_string());
            } else if l.starts_with("Status: ") && l.ends_with(" installed") {
                if let Some(n) = name.take() {
                    v.push(n);
                }
            }
        }
    }
    for e in std::fs::read_dir(root.join("var/lib/pacman/local")).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        // `name-version-release`: the last two dashes start the version.
        let parts: Vec<&str> = n.rsplitn(3, '-').collect();
        if parts.len() == 3 {
            v.push(parts[2].to_string());
        }
    }
    v.sort();
    v.dedup();
    v
}

/// Copies the configuration and the package list into `dir`; returns how
/// many files were copied.
pub fn backup_config(root: &SysRoot, dir: &std::path::Path) -> std::io::Result<usize> {
    fn copy(from: &std::path::Path, to: &std::path::Path, n: &mut usize) -> std::io::Result<()> {
        let meta = std::fs::symlink_metadata(from)?;
        if meta.is_dir() {
            std::fs::create_dir_all(to)?;
            for e in std::fs::read_dir(from)?.flatten() {
                copy(&e.path(), &to.join(e.file_name()), n)?;
            }
        } else if meta.is_file() {
            if let Some(p) = to.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::copy(from, to)?;
            *n += 1;
        }
        Ok(())
    }
    let mut n = 0;
    std::fs::create_dir_all(dir)?;
    for c in CONFIG {
        let from = root.join(c);
        if from.exists() {
            copy(&from, &dir.join(c), &mut n)?;
        }
    }
    std::fs::write(dir.join("packages.txt"), installed_packages(root).join("\n") + "\n")?;
    Ok(n + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    #[test]
    fn the_best_tool_this_machine_has_is_found() {
        let fs = TestFs::new();
        let root = SysRoot::at(fs.root());
        fs.write("proc/mounts", "/dev/nvme0n1p2 / btrfs rw,relatime,ssd,subvolid=256,subvol=/@ 0 0\n");
        let s = detect(&root, "apt");
        assert_eq!(s.tool, None);
        assert!(s.summary.contains("on Btrfs, but no snapshot tool"));
        assert_eq!(s.get_one, ["sudo apt install timeshift"]);
        fs.write("usr/bin/btrfs", "");
        assert_eq!(detect(&root, "apt").tool, Some(Tool::Btrfs { device: "/dev/nvme0n1p2".into(), subvol: "/@".into() }));
        fs.write("usr/bin/timeshift", "").write("etc/timeshift/timeshift.json", "{}");
        assert_eq!(detect(&root, "apt").tool, Some(Tool::Timeshift));
        fs.write("usr/bin/snapper", "").write("etc/snapper/configs/root", "");
        assert_eq!(detect(&root, "apt").tool, Some(Tool::Snapper));

        let fs = TestFs::new();
        fs.write("proc/mounts", "rpool/ROOT/debian / zfs rw,xattr 0 0\n").write("usr/sbin/zfs", "");
        assert_eq!(detect(&SysRoot::at(fs.root()), "apt").tool, Some(Tool::Zfs { dataset: "rpool/ROOT/debian".into() }));
        let fs = TestFs::new();
        fs.write("proc/mounts", "/dev/sda2 / ext4 rw 0 0\n");
        assert!(detect(&SysRoot::at(fs.root()), "pacman").summary.starts_with("No snapshot tool"));
    }

    #[test]
    fn snapshots_are_named_and_the_way_back_is_written_out() {
        let t = Tool::Snapper;
        assert_eq!(created_name(&t, "d1", "42\n"), "42");
        assert_eq!(restore_commands(&t, "42")[0], "sudo snapper -c root rollback 42");
        let t = Tool::Timeshift;
        assert!(create_argv(&t, "d1", "Install the missing firmware").contains(&"Unix Guardian: before Install the missing firmware (d1)".to_string()));
        assert_eq!(created_name(&t, "d1", "Creating new snapshot...\nTagged snapshot '2026-10-03_18-00-01': ondemand\n"), "2026-10-03_18-00-01");
        let t = Tool::Btrfs { device: "/dev/nvme0n1p2".into(), subvol: "/@".into() };
        assert_eq!(create_argv(&t, "d1", "x"), ["btrfs", "subvolume", "snapshot", "-r", "/", "/var/lib/unix-guardian/snapshots/d1"]);
        let back = restore_commands(&t, &created_name(&t, "d1", ""));
        assert_eq!(back[2], "sudo btrfs subvolume snapshot /mnt/@.broken/var/lib/unix-guardian/snapshots/d1 /mnt/@");
    }

    #[test]
    fn the_light_backup_copies_driver_configuration_and_the_package_list() {
        let fs = TestFs::new();
        fs.write("etc/modprobe.d/nvidia.conf", "options nvidia-drm modeset=1\n")
            .write("etc/default/grub", "GRUB_CMDLINE_LINUX_DEFAULT=\"quiet\"\n")
            .write("etc/passwd", "secret\n")
            .write("var/lib/dpkg/status", "Package: bluez\nStatus: install ok installed\n\nPackage: gone\nStatus: deinstall ok config-files\n");
        let out = tempfile::tempdir().unwrap();
        let n = backup_config(&SysRoot::at(fs.root()), out.path()).unwrap();
        assert_eq!(n, 3);
        assert!(out.path().join("etc/modprobe.d/nvidia.conf").exists() && out.path().join("etc/default/grub").exists());
        assert!(!out.path().join("etc/passwd").exists(), "only driver configuration");
        assert_eq!(std::fs::read_to_string(out.path().join("packages.txt")).unwrap(), "bluez\n");
    }
}
