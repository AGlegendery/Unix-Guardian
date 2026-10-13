//! What the test lab has turned on and must turn off again.
//!
//! The project's rule for anything temporary: record the undo *before* the
//! change, atomically, and keep it until the undo has worked. The journal is
//! a small JSON list in the user's data folder; a run that is killed half way
//! (the window closed, the laptop's battery ran out) leaves it behind, and the
//! next window, `ug tui` or `ug testlab` that starts puts things back first.
//!
//! Game mode is not in here: the daemon itself reverts it when the
//! connection that turned it on goes away, and calling `Leave` later could end
//! a game mode the user has turned on since.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One thing to put back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "undo", rename_all = "kebab-case")]
pub enum Undo {
    /// A profile the lab created.
    DeleteProfile { id: String },
    /// A program the lab added to Playtime's tracked apps.
    UntrackApp { exe: String },
    /// Scheduled power actions whose note is this marker.
    CancelPowerJobs { note: String },
    /// An internet rule for an app name the lab made up.
    RemoveNetRule { app: String },
    /// The wake alarm the lab set (cleared only if it is still that one).
    ClearWakeAlarm { at_unix_ms: u64 },
    /// A recording the lab made: stop the recorder if it still runs, delete
    /// the file.
    DeleteRecording { file: String },
}

pub fn path() -> PathBuf {
    ug_core::paths::user_data().join("testlab-undo.json")
}

#[derive(Debug)]
pub struct Journal {
    file: PathBuf,
    items: Vec<Undo>,
}

impl Journal {
    /// The journal at `file`, with whatever an earlier run left in it.
    pub fn open(file: &Path) -> Journal {
        let items = std::fs::read(file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Journal {
            file: file.to_path_buf(),
            items,
        }
    }

    pub fn items(&self) -> &[Undo] {
        &self.items
    }

    /// Records an undo before the change it undoes is made.
    pub fn push(&mut self, u: Undo) -> std::io::Result<()> {
        if !self.items.contains(&u) {
            self.items.push(u);
        }
        self.save()
    }

    /// The undo worked (or was not needed): forget it.
    pub fn done(&mut self, u: &Undo) -> std::io::Result<()> {
        self.items.retain(|i| i != u);
        self.save()
    }

    fn save(&self) -> std::io::Result<()> {
        if self.items.is_empty() {
            return match std::fs::remove_file(&self.file) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            };
        }
        let text = serde_json::to_vec_pretty(&self.items).map_err(std::io::Error::other)?;
        ug_core::paths::write_atomic(&self.file, &text)
    }
}

/// Only one run at a time: two would undo each other's changes. Held until
/// dropped; the kernel releases it if the process dies.
pub struct RunLock(#[allow(dead_code)] std::fs::File);

impl RunLock {
    pub fn take(dir: &Path) -> Result<RunLock, String> {
        use std::os::fd::AsRawFd;
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let p = dir.join("testlab.lock");
        let f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&p)
            .map_err(|e| format!("{}: {e}", p.display()))?;
        // SAFETY: flock on a descriptor this function owns.
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(
                "A test is already running (in the window, ug tui or another terminal).".into(),
            );
        }
        Ok(RunLock(f))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_undo_is_on_disk_before_the_change_and_gone_after_it() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("j.json");
        let mut j = Journal::open(&f);
        let u = Undo::DeleteProfile {
            id: "testlab-probe".into(),
        };
        j.push(u.clone()).unwrap();
        // What a crashed run leaves: the next run finds it.
        assert_eq!(Journal::open(&f).items(), [u.clone()]);
        j.done(&u).unwrap();
        assert!(!f.exists());
        assert!(Journal::open(&f).items().is_empty());
    }

    #[test]
    fn a_second_run_cannot_start_while_one_is_running() {
        let d = tempfile::tempdir().unwrap();
        let first = RunLock::take(d.path()).unwrap();
        assert!(RunLock::take(d.path()).is_err());
        drop(first);
        assert!(RunLock::take(d.path()).is_ok());
    }
}
