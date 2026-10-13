//! Where configuration and state live.
//!
//! The daemon and the agent run as different users with different lifetimes, so
//! they do not share a directory. The daemon owns machine-wide state under
//! `/var/lib`; the agent owns per-user state under the XDG directories. State
//! that must survive a crash mid-change — the revert plan, a pending GPU tune —
//! belongs to the daemon, because the agent can be restarted by the session
//! while hardware is still in a modified state.

use std::path::PathBuf;

/// System-wide configuration, installed by the package: `/etc/unix-guardian`.
pub fn system_config() -> PathBuf {
    PathBuf::from("/etc/unix-guardian")
}

/// Machine state the daemon must recover after a crash: `/var/lib/unix-guardian`.
pub fn system_state() -> PathBuf {
    PathBuf::from("/var/lib/unix-guardian")
}

/// Per-user configuration, honouring `XDG_CONFIG_HOME`.
pub fn user_config() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config")
}

/// Per-user data — the playtime and FPS database.
pub fn user_data() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share")
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(fallback))
        .join("unix-guardian")
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// Writes a file atomically: write a sibling temporary, then rename over the
/// target.
///
/// State files record what has been changed on the hardware and are read after
/// a crash to undo it. A half-written one is worse than none, because it would
/// be parsed as authoritative.
pub fn write_atomic(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or(std::path::Path::new("."));
    std::fs::create_dir_all(parent)?;
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_leaves_no_temporary_behind() {
        let dir = tempdir();
        let target = dir.join("state.json");
        write_atomic(&target, b"{\"applied\":true}").unwrap();

        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "{\"applied\":true}"
        );
        let strays: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("tmp"))
            .collect();
        assert!(strays.is_empty(), "temporary file survived the rename");
    }

    #[test]
    fn atomic_write_replaces_existing_content_completely() {
        let dir = tempdir();
        let target = dir.join("state.json");
        write_atomic(&target, b"a longer previous value").unwrap();
        write_atomic(&target, b"short").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "short");
    }

    #[test]
    fn atomic_write_creates_missing_parents() {
        let dir = tempdir();
        let target = dir.join("nested/deeper/state.json");
        write_atomic(&target, b"ok").unwrap();
        assert!(target.exists());
    }

    fn tempdir() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "ug-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
