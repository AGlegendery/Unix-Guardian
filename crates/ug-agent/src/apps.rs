//! Pausing distracting applications while a game runs.
//!
//! Pausing (`SIGSTOP`) rather than closing is the default because it loses
//! nothing: the app's memory, open connections and window stay as they were,
//! and `SIGCONT` resumes it exactly where it stopped. Its CPU and GPU use drop
//! to zero while paused, which is the point. Only the user's own processes are
//! touched, so no privilege is involved.
//!
//! The list of paused processes is also kept in a file, written before each
//! `SIGSTOP`: an agent that crashes, is killed or is replaced by an update
//! while a game runs would otherwise leave those apps frozen until the user
//! found and killed them. The next agent resumes them first thing.

use std::collections::BTreeSet;
use std::path::PathBuf;
use ug_hw::SysRoot;

/// Processes this agent paused, so exactly those — and only those — are
/// resumed. A process the user had stopped themselves stays stopped.
#[derive(Debug, Default)]
pub struct Paused {
    pids: BTreeSet<(i32, u64)>,
    /// Where the list is kept across a crash; none in tests.
    file: Option<PathBuf>,
}

impl Paused {
    /// Keeps the list in `file`, and resumes what an earlier agent left
    /// paused there. Returns how many were resumed.
    pub fn keep_in(&mut self, root: &SysRoot, file: PathBuf) -> usize {
        let left: Vec<(i32, u64)> = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        self.file = Some(file);
        self.pids.extend(left);
        let n = self.resume_all(root);
        if n > 0 {
            tracing::warn!(
                count = n,
                "resumed apps an earlier agent had paused and not resumed"
            );
        }
        n
    }

    fn save(&self) {
        let Some(f) = &self.file else { return };
        let r = if self.pids.is_empty() {
            match std::fs::remove_file(f) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            }
        } else {
            let v: Vec<&(i32, u64)> = self.pids.iter().collect();
            ug_core::paths::write_atomic(
                f,
                serde_json::to_string(&v).unwrap_or_default().as_bytes(),
            )
        };
        if let Err(e) = r {
            tracing::warn!(error = %e, file = %f.display(), "could not record paused apps");
        }
    }

    /// Pauses every process of `uid` whose name matches one of `names`
    /// (compared to `comm`, case-insensitively). Returns the names paused.
    pub fn pause_matching(
        &mut self,
        root: &SysRoot,
        uid: u32,
        names: &[String],
        protect: &[i32],
    ) -> Vec<String> {
        if names.is_empty() {
            return Vec::new();
        }
        let wanted: Vec<String> = names.iter().map(|n| n.to_lowercase()).collect();
        let mut hit = Vec::new();
        for pid in ug_games::proc::pids(root) {
            if protect.contains(&pid) || pid == std::process::id() as i32 {
                continue;
            }
            let Some(p) = ug_games::proc::read(root, pid) else {
                continue;
            };
            if p.uid != uid || !wanted.contains(&p.comm.to_lowercase()) {
                continue;
            }
            if already_stopped(root, pid) {
                continue; // not ours to resume later
            }
            // On disk before the stop, so a crash right after it still
            // leaves the way back.
            self.pids.insert((pid, p.start_ticks));
            self.save();
            // SAFETY: kill with a valid signal number has no memory-safety preconditions.
            if unsafe { libc::kill(pid, libc::SIGSTOP) } == 0 {
                if !hit.contains(&p.comm) {
                    hit.push(p.comm.clone());
                }
            } else {
                self.pids.remove(&(pid, p.start_ticks));
                self.save();
            }
        }
        if !hit.is_empty() {
            tracing::info!(apps = ?hit, "paused for the game");
        }
        hit
    }

    /// Resumes everything this agent paused. A PID now belonging to a
    /// different process (same number, different start time) is left alone.
    pub fn resume_all(&mut self, root: &SysRoot) -> usize {
        let mut n = 0;
        for (pid, start) in std::mem::take(&mut self.pids) {
            match ug_games::proc::stat(root, pid) {
                Some((_, _, s)) if s == start => {
                    // SAFETY: as above.
                    if unsafe { libc::kill(pid, libc::SIGCONT) } == 0 {
                        n += 1;
                    }
                }
                _ => {}
            }
        }
        self.save();
        if n > 0 {
            tracing::info!(count = n, "resumed paused applications");
        }
        n
    }
}

/// True when a process is already in the stopped state (`T`).
fn already_stopped(root: &SysRoot, pid: i32) -> bool {
    std::fs::read_to_string(root.join(format!("proc/{pid}/stat")))
        .ok()
        .and_then(|s| s.rfind(')').map(|i| s[i + 2..].starts_with('T')))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command};

    fn state(pid: u32) -> char {
        let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        s[s.rfind(')').unwrap() + 2..].chars().next().unwrap()
    }

    fn spawn_named(name: &str) -> Child {
        // The kernel names a process (`comm`) after the path it was executed
        // through, so a symlink called `name` pointing at `sleep` gives a
        // process called `name`.
        //
        // Not a copy: writing an executable while other test threads fork
        // lets a forked child inherit the open write handle for a moment, and
        // executing the file in that window fails with "text file busy".
        let dir = std::env::temp_dir().join(format!("ug-apps-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(name);
        if !exe.exists() {
            std::os::unix::fs::symlink("/bin/sleep", &exe).unwrap();
        }
        Command::new(exe).arg("30").spawn().unwrap()
    }

    fn wait_state(pid: u32, want: char) -> bool {
        (0..100).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(10));
            state(pid) == want
        })
    }

    #[test]
    fn pauses_by_name_and_resumes_exactly_what_it_paused() {
        let uid = unsafe { libc::getuid() };
        let mut target = spawn_named("ugfakediscord");
        let mut bystander = spawn_named("ugbystander");
        let mut users_own = spawn_named("ugfakediscord");
        // The user had already stopped this one; we must not resume it.
        unsafe { libc::kill(users_own.id() as i32, libc::SIGSTOP) };
        assert!(wait_state(users_own.id(), 'T'));

        let mut paused = Paused::default();
        let names = paused.pause_matching(&SysRoot::real(), uid, &["UGFakeDiscord".into()], &[]);
        assert_eq!(names, vec!["ugfakediscord"]);
        assert!(
            wait_state(target.id(), 'T'),
            "matched by name, case-insensitively"
        );
        assert_ne!(state(bystander.id()), 'T', "others untouched");

        assert_eq!(paused.resume_all(&SysRoot::real()), 1);
        assert!(wait_state(target.id(), 'S'));
        assert_eq!(
            state(users_own.id()),
            'T',
            "the one the user stopped stays stopped"
        );

        for c in [&mut target, &mut bystander, &mut users_own] {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    #[test]
    fn apps_paused_by_an_agent_that_died_are_resumed_by_the_next_one() {
        let uid = unsafe { libc::getuid() };
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("paused.json");
        // Resumed and killed even if an assertion fails: a stopped stray
        // would outlive the test.
        struct Reap(Child);
        impl Drop for Reap {
            fn drop(&mut self) {
                unsafe { libc::kill(self.0.id() as i32, libc::SIGCONT) };
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let app = Reap(spawn_named("ugfakechatcrash"));
        let app = &app.0;
        let mut first = Paused::default();
        assert_eq!(first.keep_in(&SysRoot::real(), file.clone()), 0);
        first.pause_matching(&SysRoot::real(), uid, &["ugfakechatcrash".into()], &[]);
        assert!(wait_state(app.id(), 'T'));
        drop(first); // the agent is killed: no resume_all

        let mut next = Paused::default();
        assert_eq!(next.keep_in(&SysRoot::real(), file.clone()), 1);
        assert!(wait_state(app.id(), 'S'), "running again");
        assert!(!file.exists(), "nothing left to resume");
    }

    #[test]
    fn the_game_itself_is_never_paused() {
        let uid = unsafe { libc::getuid() };
        let mut game = spawn_named("ugfakegame");
        let mut paused = Paused::default();
        let hit = paused.pause_matching(
            &SysRoot::real(),
            uid,
            &["ugfakegame".into()],
            &[game.id() as i32],
        );
        assert!(hit.is_empty());
        assert_ne!(state(game.id()), 'T');
        let _ = game.kill();
        let _ = game.wait();
    }
}
