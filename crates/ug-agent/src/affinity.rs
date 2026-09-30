//! Keeping a game on the CPU's preferred cores.
//!
//! Done with `sched_setaffinity` on every thread of every game process, which
//! a user may do to their own processes — no root, no cgroup surgery, and
//! nothing to restore afterwards: the affinity dies with the game. New threads
//! inherit their creator's affinity, so a thread spawned after pinning stays
//! pinned too.

use std::collections::HashSet;
use ug_hw::SysRoot;

/// Pins every thread of `pids` to `cpus`. Returns how many threads were set.
///
/// Threads that exit mid-way are skipped silently; a game spawning and
/// retiring worker threads while we walk its thread list is normal.
pub fn pin(root: &SysRoot, pids: &[i32], cpus: &[u32]) -> usize {
    if cpus.is_empty() {
        return 0;
    }
    // SAFETY: cpu_set_t is plain data; CPU_ZERO/CPU_SET only write within it.
    let mut set: libc::cpu_set_t = unsafe { std::mem::zeroed() };
    for &c in cpus {
        if (c as usize) < libc::CPU_SETSIZE as usize {
            unsafe { libc::CPU_SET(c as usize, &mut set) };
        }
    }

    let mut done = 0;
    let mut seen = HashSet::new();
    for &pid in pids {
        for tid in threads(root, pid) {
            if !seen.insert(tid) {
                continue;
            }
            // SAFETY: `set` is a valid, initialised cpu_set_t for its full size.
            let rc = unsafe { libc::sched_setaffinity(tid, std::mem::size_of::<libc::cpu_set_t>(), &set) };
            if rc == 0 {
                done += 1;
            }
        }
    }
    done
}

/// Reads a thread's current affinity. Used to verify pinning.
#[cfg(test)]
pub fn current(tid: i32) -> Option<Vec<u32>> {
    // SAFETY: as above; the kernel writes at most size_of::<cpu_set_t>() bytes.
    let mut set: libc::cpu_set_t = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::sched_getaffinity(tid, std::mem::size_of::<libc::cpu_set_t>(), &mut set) };
    if rc != 0 {
        return None;
    }
    Some((0..libc::CPU_SETSIZE as u32).filter(|&c| unsafe { libc::CPU_ISSET(c as usize, &set) }).collect())
}

/// Thread IDs of a process, from `/proc/<pid>/task`.
fn threads(root: &SysRoot, pid: i32) -> Vec<i32> {
    std::fs::read_dir(root.join(format!("proc/{pid}/task")))
        .map(|d| d.filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok()).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_every_thread_of_a_real_process() {
        // A child with a few threads of its own, pinned from outside.
        let mut child = std::process::Command::new("sh")
            .args(["-c", "sleep 30 & sleep 30 & wait"])
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        let online = current(pid).unwrap();
        let target = vec![online[0]];

        let n = pin(&SysRoot::real(), &[pid], &target);
        assert!(n >= 1);
        assert_eq!(current(pid).unwrap(), target);

        // Children forked *after* pinning inherit the affinity.
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn a_vanished_process_is_skipped_not_an_error() {
        assert_eq!(pin(&SysRoot::real(), &[i32::MAX - 1], &[0]), 0);
    }

    #[test]
    fn an_empty_cpu_list_changes_nothing() {
        assert_eq!(pin(&SysRoot::real(), &[std::process::id() as i32], &[]), 0);
    }
}
