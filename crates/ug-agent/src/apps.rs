//! Pausing distracting applications while a game runs.
//!
//! Pausing (`SIGSTOP`) rather than closing is the default because it loses
//! nothing: the app's memory, open connections and window stay as they were,
//! and `SIGCONT` resumes it exactly where it stopped. Its CPU and GPU use drop
//! to zero while paused, which is the point. Only the user's own processes are
//! touched, so no privilege is involved.

use std::collections::BTreeSet;
use ug_hw::SysRoot;

/// Processes this agent paused, so exactly those — and only those — are
/// resumed. A process the user had stopped themselves stays stopped.
#[derive(Debug, Default)]
pub struct Paused {
    pids: BTreeSet<(i32, u64)>,
}

impl Paused {
    /// Pauses every process of `uid` whose name matches one of `names`
    /// (compared to `comm`, case-insensitively). Returns the names paused.
    pub fn pause_matching(&mut self, root: &SysRoot, uid: u32, names: &[String], protect: &[i32]) -> Vec<String> {
        if names.is_empty() {
            return Vec::new();
        }
        let wanted: Vec<String> = names.iter().map(|n| n.to_lowercase()).collect();
        let mut hit = Vec::new();
        for pid in ug_games::proc::pids(root) {
            if protect.contains(&pid) || pid == std::process::id() as i32 {
                continue;
            }
            let Some(p) = ug_games::proc::read(root, pid) else { continue };
            if p.uid != uid || !wanted.contains(&p.comm.to_lowercase()) {
                continue;
            }
            if already_stopped(root, pid) {
                continue; // not ours to resume later
            }
            // SAFETY: kill with a valid signal number has no memory-safety preconditions.
            if unsafe { libc::kill(pid, libc::SIGSTOP) } == 0 {
                self.pids.insert((pid, p.start_ticks));
                if !hit.contains(&p.comm) {
                    hit.push(p.comm.clone());
                }
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
        // exec -a sets argv[0]; comm follows the executable, so copy `sleep`
        // under the wanted name into a temp dir and run that.
        let dir = std::env::temp_dir().join(format!("ug-apps-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(name);
        // Copy once: overwriting a copy that is already running fails with
        // "text file busy".
        if !exe.exists() {
            std::fs::copy("/bin/sleep", &exe).unwrap();
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
        assert!(wait_state(target.id(), 'T'), "matched by name, case-insensitively");
        assert_ne!(state(bystander.id()), 'T', "others untouched");

        assert_eq!(paused.resume_all(&SysRoot::real()), 1);
        assert!(wait_state(target.id(), 'S'));
        assert_eq!(state(users_own.id()), 'T', "the one the user stopped stays stopped");

        for c in [&mut target, &mut bystander, &mut users_own] {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    #[test]
    fn the_game_itself_is_never_paused() {
        let uid = unsafe { libc::getuid() };
        let mut game = spawn_named("ugfakegame");
        let mut paused = Paused::default();
        let hit = paused.pause_matching(&SysRoot::real(), uid, &["ugfakegame".into()], &[game.id() as i32]);
        assert!(hit.is_empty());
        assert_ne!(state(game.id()), 'T');
        let _ = game.kill();
        let _ = game.wait();
    }
}
