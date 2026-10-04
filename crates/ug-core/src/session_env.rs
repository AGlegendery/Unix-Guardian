//! Files the whole desktop session reads, which Unix Guardian must never
//! write to, and the move that takes away what older versions put there.
//!
//! Two older features wrote into `~/.config/environment.d`: the card switch
//! (NVIDIA offload variables) and the overlay's "every game"
//! (`MANGOHUD=1`). systemd's user manager reads that directory and KWin,
//! plasmashell and every app it starts inherit it, so on 2026-10-04 the
//! first one put KDE's compositor on the NVIDIA card and the desktop went
//! black (`claude-project/05-findings.md`). `MANGOHUD=1` has the same reach:
//! every Vulkan program, GTK 4 apps included, loads the overlay. Both now
//! work per program only.

use std::path::{Path, PathBuf};

/// Every place that applies to the whole session, relative to the home
/// directory. Nothing of Unix Guardian's may be written under them; tests
/// check writers against this list.
pub const SESSION_WIDE: [&str; 7] = [
    ".config/environment.d",
    ".config/plasma-workspace/env",
    ".profile",
    ".pam_environment",
    ".xprofile",
    ".xsessionrc",
    ".bash_profile",
];

/// The files older versions wrote into `environment.d`.
pub const LEGACY_FILES: [&str; 2] = ["90-unix-guardian-gpu.conf", "90-unix-guardian-mangohud.conf"];

/// What [`retire`] did with one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retired {
    /// Where the file was moved to.
    pub moved_to: PathBuf,
    /// The variables it set, so the caller can drop them from the running
    /// user manager (with `Linger=yes` it outlives a logout).
    pub vars: Vec<(String, String)>,
}

/// Moves `environment.d/<name>` (under `config_home`, i.e. `~/.config`) to
/// `<ug_config>/retired/<name>.<stamp>`, if it is there and Unix Guardian
/// wrote it. Never deletes it, and never touches anything else (not
/// `environment.d.disabled`, where a repair may have put a copy).
pub fn retire(config_home: &Path, ug_config: &Path, name: &str, stamp: &str) -> std::io::Result<Option<Retired>> {
    let path = config_home.join("environment.d").join(name);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if !text.contains("Unix Guardian") && !text.contains("UG_DEFAULT_GPU=") {
        // Not ours to move.
        return Ok(None);
    }
    let vars = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let dir = ug_config.join("retired");
    std::fs::create_dir_all(&dir)?;
    let moved_to = dir.join(format!("{name}.{stamp}"));
    std::fs::rename(&path, &moved_to)?;
    Ok(Some(Retired { moved_to, vars }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_mangohud_file_is_moved_and_a_foreign_one_left() {
        let home = tempfile::tempdir().unwrap();
        let config = home.path().join(".config");
        let env = config.join("environment.d");
        std::fs::create_dir_all(&env).unwrap();
        let ours = "# Written by Unix Guardian: load MangoHud into Vulkan games.\nMANGOHUD=1\n";
        std::fs::write(env.join("90-unix-guardian-mangohud.conf"), ours).unwrap();
        std::fs::write(env.join("50-mine.conf"), "MANGOHUD=1\n").unwrap();
        let ug = config.join("unix-guardian");
        let r = retire(&config, &ug, "90-unix-guardian-mangohud.conf", "7").unwrap().unwrap();
        assert_eq!(r.vars, [("MANGOHUD".to_string(), "1".to_string())]);
        assert_eq!(std::fs::read_to_string(&r.moved_to).unwrap(), ours);
        assert!(!env.join("90-unix-guardian-mangohud.conf").exists());
        assert!(env.join("50-mine.conf").exists());
        std::fs::write(env.join("90-unix-guardian-mangohud.conf"), "MANGOHUD=1\n").unwrap();
        assert_eq!(retire(&config, &ug, "90-unix-guardian-mangohud.conf", "8").unwrap(), None, "not ours");
    }
}
