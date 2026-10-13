//! Swap for hibernation: a swap file large enough for the memory image,
//! and the kernel told where to resume from.
//!
//! The file goes where it cannot hurt snapshots: on Btrfs into a
//! subvolume of its own (an existing `/swap` mount, or a new nested
//! subvolume `/swap`, which a snapshot of the root does not include), made
//! with `btrfs filesystem mkswapfile` (no copy-on-write, no compression, as
//! Btrfs requires); on ext4 or XFS with fallocate and mkswap. Then fstab,
//! swapon, and `resume=UUID=… resume_offset=…` on the kernel command line,
//! whose offset can only be read once the file exists, so the executor
//! finds it (see `apply`). The existing swap stays as it is.

use ug_hw::SysRoot;

/// Where and how the hibernation swap file is made.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// The file, absolute.
    pub file: String,
    pub size_gib: u64,
    /// `btrfs`, `ext4` or `xfs`.
    pub fs: String,
    /// A Btrfs subvolume to create first (absolute), when none exists.
    pub new_subvolume: Option<String>,
}

const FILE: &str = "hibernate.swap";

/// Free bytes on the filesystem holding `path`.
pub fn free_bytes(path: &std::path::Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: a valid C string and a zeroed struct for the kernel to fill.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    Some(st.f_bavail as u64 * st.f_frsize as u64)
}

/// Mounts as (mount point, type, options), from `/proc/mounts`.
fn mounts(root: &SysRoot) -> Vec<(String, String, String)> {
    std::fs::read_to_string(root.join("proc/mounts"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let w: Vec<&str> = l.split_whitespace().collect();
            (w.len() >= 4).then(|| (w[1].to_string(), w[2].to_string(), w[3].to_string()))
        })
        .collect()
}

const GIB: u64 = 1 << 30;

/// Decides the file, or says why it has to be done by hand. `free` is the
/// free space where the file would go (bytes); `mem_kb` the memory.
pub fn plan(
    root: &SysRoot,
    mem_kb: u64,
    free: impl Fn(&str) -> Option<u64>,
) -> Result<Plan, String> {
    let m = mounts(root);
    let fs_of = |mp: &str| m.iter().rev().find(|x| x.0 == mp).map(|x| x.1.clone());
    let root_fs = fs_of("/").unwrap_or_default();
    let (dir, fs, new_subvolume) = match fs_of("/swap") {
        Some(fs) if fs == "btrfs" => ("/swap".to_string(), fs, None),
        Some(fs) if fs == "ext4" || fs == "xfs" => ("/swap".to_string(), fs, None),
        Some(other) => {
            return Err(format!(
                "/swap is a {other} filesystem, which Unix Guardian does not make swap files on."
            ))
        }
        None if root_fs == "btrfs" => {
            if root.join("swap").exists() {
                return Err("/swap exists but is not a Btrfs subvolume of its own; a swap file there would stop the root from being snapshotted.".into());
            }
            ("/".to_string(), root_fs, Some("/swap".to_string()))
        }
        None if root_fs == "ext4" || root_fs == "xfs" => ("/".to_string(), root_fs, None),
        None => {
            return Err(format!(
                "The root filesystem is {}, where Unix Guardian does not make swap files.",
                if root_fs.is_empty() {
                    "unknown"
                } else {
                    &root_fs
                }
            ))
        }
    };
    let file = if dir == "/" && new_subvolume.is_none() {
        format!("/{FILE}")
    } else {
        format!("/swap/{FILE}")
    };
    if root.join(file.trim_start_matches('/')).exists() {
        return Err(format!("{file} already exists."));
    }
    // The whole memory if it fits with room to spare, else 60% of it (the
    // kernel shrinks the image to fit; 60% is the usual working minimum).
    let mem = mem_kb * 1024;
    let full = mem.div_ceil(GIB);
    let small = (mem * 6 / 10).div_ceil(GIB);
    let avail = free(&dir).unwrap_or(0);
    let room = 5 * GIB;
    let size_gib = if avail >= full * GIB + room {
        full
    } else if avail >= small * GIB + room {
        small
    } else {
        return Err(format!(
            "There is not enough free space: a hibernation swap file needs about {small}–{full} GB plus 5 GB to spare, and {} GB are free.",
            avail / GIB
        ));
    };
    Ok(Plan {
        file,
        size_gib,
        fs,
        new_subvolume,
    })
}

/// The programs that make the file, in order.
pub fn make_argv(p: &Plan) -> Vec<Vec<String>> {
    let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let mut out = Vec::new();
    if let Some(sv) = &p.new_subvolume {
        out.push(v(&["btrfs", "subvolume", "create", sv]));
    }
    if p.fs == "btrfs" {
        out.push(v(&[
            "btrfs",
            "filesystem",
            "mkswapfile",
            "--size",
            &format!("{}g", p.size_gib),
            &p.file,
        ]));
    } else {
        out.push(v(&[
            "fallocate",
            "-l",
            &format!("{}G", p.size_gib),
            &p.file,
        ]));
        out.push(v(&["chmod", "600", &p.file]));
        out.push(v(&["mkswap", &p.file]));
    }
    out
}

/// Reads the resume offset: `btrfs inspect-internal map-swapfile -r` prints
/// it alone; `filefrag -v` lists extents, the first one's physical start.
pub fn offset_argv(p: &Plan) -> Vec<String> {
    if p.fs == "btrfs" {
        ["btrfs", "inspect-internal", "map-swapfile", "-r", &p.file]
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        ["filefrag", "-v", &p.file]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }
}

pub fn parse_offset(fs: &str, out: &str) -> Option<u64> {
    if fs == "btrfs" {
        return out.split_whitespace().last()?.parse().ok();
    }
    // "   0:        0..   32767:      34816..     67583:  32768:"
    out.lines().find_map(|l| {
        let mut parts = l.split(':');
        if parts.next()?.trim() != "0" {
            return None;
        }
        parts.nth(1)?.split("..").next()?.trim().parse().ok()
    })
}

/// The fstab line for the file, at a low priority so everyday swapping
/// keeps using the swap there was.
pub fn fstab_line(p: &Plan) -> String {
    format!("{} none swap defaults,pri=-10 0 0", p.file)
}

/// The steps as commands, for doing it by hand (the guide).
pub fn guide(p: Option<&Plan>, why: &str) -> Vec<String> {
    let mut v = Vec::new();
    if !why.is_empty() {
        v.push(format!("# {why}"));
    }
    let Some(p) = p else {
        v.extend([
            "# Btrfs: a swap file needs a subvolume of its own and no copy-on-write:".to_string(),
            "sudo btrfs subvolume create /swap".into(),
            "sudo btrfs filesystem mkswapfile --size 16g /swap/hibernate.swap".into(),
            "# ext4: sudo fallocate -l 16G /hibernate.swap && sudo chmod 600 /hibernate.swap && sudo mkswap /hibernate.swap".into(),
            "sudo swapon /swap/hibernate.swap".into(),
            "echo '/swap/hibernate.swap none swap defaults,pri=-10 0 0' | sudo tee -a /etc/fstab".into(),
            "# the offset: Btrfs: sudo btrfs inspect-internal map-swapfile -r /swap/hibernate.swap; ext4: sudo filefrag -v /hibernate.swap (first physical_offset)".into(),
            "# add resume=UUID=$(findmnt -no UUID -T /swap/hibernate.swap) resume_offset=<offset> to the kernel parameters (ug driver param add …), rebuild the initramfs, restart".into(),
        ]);
        return v;
    };
    for a in make_argv(p) {
        v.push(format!("sudo {}", a.join(" ")));
    }
    v.push(format!("sudo swapon {}", p.file));
    v.push(format!("echo '{}' | sudo tee -a /etc/fstab", fstab_line(p)));
    v.push(format!("sudo {}   # the offset", offset_argv(p).join(" ")));
    v.push(format!(
        "ug driver param add resume=UUID=$(findmnt -no UUID -T {})",
        p.file
    ));
    v.push("ug driver param add resume_offset=<the offset>".into());
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    const MEM: u64 = 28_440_104; // kB, the laptop: 27.1 GiB

    #[test]
    fn the_file_goes_where_snapshots_are_safe_and_fits_the_free_space() {
        let fs = TestFs::new();
        let root = SysRoot::at(fs.root());
        // The laptop: root /@ and a /swap subvolume of its own, 40 GB free.
        fs.write("proc/mounts", "/dev/nvme0n1p2 / btrfs rw,subvol=/@ 0 0\n/dev/nvme0n1p2 /swap btrfs rw,subvol=/@swap 0 0\n");
        let p = plan(&root, MEM, |_| Some(40 * GIB)).unwrap();
        assert_eq!(
            p,
            Plan {
                file: "/swap/hibernate.swap".into(),
                size_gib: 28,
                fs: "btrfs".into(),
                new_subvolume: None
            }
        );
        assert_eq!(
            make_argv(&p),
            [[
                "btrfs",
                "filesystem",
                "mkswapfile",
                "--size",
                "28g",
                "/swap/hibernate.swap"
            ]]
        );
        assert_eq!(
            plan(&root, MEM, |_| Some(25 * GIB)).unwrap().size_gib,
            17,
            "60% when the whole memory does not fit"
        );
        assert!(plan(&root, MEM, |_| Some(15 * GIB))
            .unwrap_err()
            .contains("not enough free space"));

        let fs = TestFs::new();
        let root = SysRoot::at(fs.root());
        fs.write("proc/mounts", "/dev/sda2 / btrfs rw,subvol=/@ 0 0\n");
        let p = plan(&root, MEM, |_| Some(100 * GIB)).unwrap();
        assert_eq!(
            p.new_subvolume.as_deref(),
            Some("/swap"),
            "a subvolume of its own, so the root can still be snapshotted"
        );
        assert_eq!(make_argv(&p)[0], ["btrfs", "subvolume", "create", "/swap"]);
        fs.dir("swap");
        assert!(plan(&root, MEM, |_| Some(100 * GIB))
            .unwrap_err()
            .contains("not a Btrfs subvolume"));

        let fs = TestFs::new();
        let root = SysRoot::at(fs.root());
        fs.write("proc/mounts", "/dev/sda2 / ext4 rw 0 0\n");
        let p = plan(&root, 16_000_000, |_| Some(100 * GIB)).unwrap();
        assert_eq!((p.file.as_str(), p.size_gib), ("/hibernate.swap", 16));
        assert_eq!(make_argv(&p).len(), 3);
        fs.write("proc/mounts", "/dev/sda2 / f2fs rw 0 0\n");
        assert!(plan(&root, MEM, |_| Some(100 * GIB)).is_err());
    }

    #[test]
    fn the_resume_offset_is_read_from_the_tools() {
        assert_eq!(parse_offset("btrfs", "533760\n"), Some(533760));
        let filefrag = "Filesystem type is: ef53\nFile size of /hibernate.swap is 17179869184 (4194304 blocks of 4096 bytes)\n ext:     logical_offset:        physical_offset: length:   expected: flags:\n   0:        0..   32767:      34816..     67583:  32768:\n   1:    32768..   65535:      98304..    131071:  32768:      67584:\n";
        assert_eq!(parse_offset("ext4", filefrag), Some(34816));
        assert_eq!(parse_offset("btrfs", "error"), None);
    }
}
