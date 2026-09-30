//! Which cgroups hold processes, and what to call them.
//!
//! systemd already gives every desktop app its own cgroup
//! (`app-firefox-1234.scope`, `app-org.kde.konsole@….service`) and every
//! service its own `.service`, so an app's traffic is its cgroup's traffic:
//! nothing has to be moved to be counted.

use std::path::Path;
use ug_hw::SysRoot;

/// Deepest cgroup level counted. systemd's user apps sit at level 5
/// (`user.slice/user-1000.slice/user@1000.service/app.slice/app-x.scope`);
/// three more levels leave room for apps that make their own sub-groups.
pub const MAX_LEVEL: u8 = 8;
pub const MIN_LEVEL: u8 = 2;

/// A cgroup with processes in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// Relative to the cgroup root, no leading slash.
    pub path: String,
    pub level: u8,
    pub pids: Vec<i32>,
}

/// Every cgroup between [`MIN_LEVEL`] and [`MAX_LEVEL`] that holds processes.
pub fn scan(root: &SysRoot) -> Vec<Group> {
    let base = root.join("sys/fs/cgroup");
    let mut out = Vec::new();
    walk(&base, &base, 0, &mut out);
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn walk(base: &Path, dir: &Path, level: u8, out: &mut Vec<Group>) {
    if level >= MIN_LEVEL {
        let pids: Vec<i32> = std::fs::read_to_string(dir.join("cgroup.procs"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.trim().parse().ok())
            .collect();
        if !pids.is_empty() {
            if let Ok(rel) = dir.strip_prefix(base) {
                out.push(Group { path: rel.to_string_lossy().into_owned(), level, pids });
            }
        }
    }
    if level >= MAX_LEVEL {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            walk(base, &e.path(), level + 1, out);
        }
    }
}

/// How the interfaces present a cgroup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    pub name: String,
    /// `app`, `service` or `session`.
    pub kind: &'static str,
    pub uid: u32,
}

/// Names a cgroup from its path: `app-com.google.Chrome-6072.scope` is
/// `Chrome`, `app-firefox\x2desr@….service` is `firefox-esr`,
/// `system.slice/packagekit.service` is `packagekit`. `exe` (the first
/// process's command) names what systemd's name does not.
pub fn label(path: &str, exe: &str) -> Label {
    let parts: Vec<&str> = path.split('/').collect();
    let uid = parts
        .iter()
        .find_map(|p| p.strip_prefix("user-")?.strip_suffix(".slice")?.parse().ok())
        .unwrap_or(0);
    // The innermost systemd unit names the app; deeper groups are the app's own.
    let unit = parts
        .iter()
        .rev()
        .find(|p| p.ends_with(".scope") || p.ends_with(".service"))
        .copied()
        .unwrap_or_else(|| parts.last().copied().unwrap_or(""));
    let in_apps = parts.contains(&"app.slice");
    let session = unit.starts_with("session-") && unit.ends_with(".scope");
    let kind = if session {
        "session"
    } else if in_apps || unit.starts_with("app-") {
        "app"
    } else {
        "service"
    };
    if session {
        return Label { name: format!("Terminal and session ({})", if exe.is_empty() { "login" } else { exe }), kind, uid };
    }
    let mut name = unit.trim_end_matches(".scope").trim_end_matches(".service").to_string();
    if let Some(n) = name.strip_prefix("app-") {
        name = n.to_string();
    }
    if let Some((n, _instance)) = name.split_once('@') {
        name = n.to_string();
    }
    // `app-<name>-<pid>.scope`
    if let Some((n, tail)) = name.rsplit_once('-') {
        if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) {
            name = n.to_string();
        }
    }
    let mut name = unescape(&name);
    // Reverse-DNS app ids read best by their last part.
    if name.matches('.').count() >= 2 {
        if let Some(last) = name.rsplit('.').next() {
            name = last.to_string();
        }
    }
    if name.is_empty() {
        name = if exe.is_empty() { path.to_string() } else { exe.to_string() };
    }
    Label { name, kind, uid }
}

/// systemd's `\xNN` escapes in unit names.
fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("\\x") {
        out.push_str(&rest[..i]);
        let hex = rest.get(i + 2..i + 4).and_then(|h| u8::from_str_radix(h, 16).ok());
        match hex {
            Some(b) if b.is_ascii() => {
                out.push(b as char);
                rest = &rest[i + 4..];
            }
            _ => {
                out.push_str("\\x");
                rest = &rest[i + 2..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Programs known to use the network in the background, by the words in
/// their app or command name.
const CATALOG: &[(&str, &[&str])] = &[
    ("torrent", &["qbittorrent", "transmission", "deluge", "ktorrent", "fragments", "rtorrent", "aria2c", "tixati", "biglybt"]),
    ("sync", &["dropbox", "syncthing", "nextcloud", "owncloud", "megasync", "onedrive", "insync", "rclone", "seafile", "pcloud", "maestral", "rsync"]),
    ("updates", &["packagekit", "flatpak-system-helper", "snapd", "apt", "dnf", "zypper", "pamac", "discover", "fwupd", "unattended-upgr", "gnome-software", "mintupdate"]),
    ("launcher", &["steam", "epicgameslauncher", "heroic", "lutris", "battle.net", "legendary", "itch", "gogdl", "bottles"]),
    ("backup", &["backblaze", "duplicati", "deja-dup", "restic", "borg", "timeshift", "kopia", "vorta"]),
    ("streaming", &["obs"]),
];

/// The background-task kind of an app, or `""`.
pub fn category(name: &str, exe: &str) -> &'static str {
    let (n, e) = (name.to_lowercase(), exe.to_lowercase());
    for (cat, words) in CATALOG {
        for w in *words {
            // Whole-word match for short names ("apt", "obs") so "captain"
            // or "jobs" do not count; substring for the distinctive ones.
            let hit = |s: &str| {
                if w.len() <= 4 {
                    s.split(|c: char| !c.is_ascii_alphanumeric()).any(|part| part == *w)
                } else {
                    s.contains(w)
                }
            };
            if hit(&n) || hit(&e) {
                return cat;
            }
        }
    }
    ""
}

/// The command name of a process, from `/proc/<pid>/comm`.
pub fn comm(root: &SysRoot, pid: i32) -> String {
    std::fs::read_to_string(root.join(format!("proc/{pid}/comm"))).map(|s| s.trim().to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    const APPS: &str = "user.slice/user-1000.slice/user@1000.service/app.slice";

    #[test]
    fn scan_finds_groups_with_processes_only() {
        let fs = TestFs::new();
        fs.write(format!("sys/fs/cgroup/{APPS}/app-com.google.Chrome-6072.scope/cgroup.procs"), "6072\n6100");
        fs.write(format!("sys/fs/cgroup/{APPS}/cgroup.procs"), "");
        fs.write("sys/fs/cgroup/system.slice/packagekit.service/cgroup.procs", "812");
        fs.write("sys/fs/cgroup/system.slice/cgroup.procs", "");
        fs.write("sys/fs/cgroup/init.scope/cgroup.procs", "1"); // level 1: not counted
        let got = scan(&SysRoot::at(fs.root()));
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].path, "system.slice/packagekit.service");
        assert_eq!(got[0].level, 2);
        assert_eq!(got[1].level, 5);
        assert_eq!(got[1].pids, vec![6072, 6100]);
    }

    #[test]
    fn known_background_tasks_are_recognised_without_false_friends() {
        assert_eq!(category("qBittorrent", "qbittorrent"), "torrent");
        assert_eq!(category("packagekit", "packagekitd"), "updates");
        assert_eq!(category("steam", "steamwebhelper"), "launcher");
        assert_eq!(category("com.obsproject.Studio", "obs"), "streaming");
        assert_eq!(category("syncthing", ""), "sync");
        assert_eq!(category("jobs-app", "captain"), "", "short words match whole words only");
        assert_eq!(category("firefox-esr", "firefox-esr"), "");
    }

    #[test]
    fn labels_read_like_app_names() {
        let l = label(&format!("{APPS}/app-com.google.Chrome-6072.scope"), "chrome");
        assert_eq!((l.name.as_str(), l.kind, l.uid), ("Chrome", "app", 1000));
        let l = label(&format!("{APPS}/app-firefox\\x2desr@7fb6f6ecbac84ba091c548cd422013d2.service"), "firefox-esr");
        assert_eq!(l.name, "firefox-esr");
        let l = label(&format!("{APPS}/app-org.kde.konsole-344352.scope"), "konsole");
        assert_eq!(l.name, "konsole");
        let l = label("system.slice/packagekit.service", "packagekitd");
        assert_eq!((l.name.as_str(), l.kind, l.uid), ("packagekit", "service", 0));
        let l = label("user.slice/user-1000.slice/session-2.scope", "sddm-helper");
        assert_eq!(l.kind, "session");
        let l = label("user.slice/user-1000.slice/user@1000.service/session.slice/pipewire.service", "pipewire");
        assert_eq!((l.name.as_str(), l.kind), ("pipewire", "service"));
        // An app's own sub-group is still the app.
        let l = label(&format!("{APPS}/app-steam@abc.service/steamwebhelper"), "steamwebhelper");
        assert_eq!(l.name, "steam");
    }
}
