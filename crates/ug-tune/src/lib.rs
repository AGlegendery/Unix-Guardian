//! Reversible system tuning for game mode.
//!
//! Every change is recorded in an undo list **before** it is made, and the list
//! is written to disk after each step. If the daemon dies mid-game — crash,
//! OOM kill, power cut — the next start finds the list and undoes it, so the
//! machine never stays in game mode by accident. Undo entries carry the value
//! that was there before, never an assumed default: a user who had set
//! `swappiness` to 30 gets 30 back, not 60.
//!
//! Two changes need no undo entry because the kernel reverts them itself: the
//! idle-state hold on `/dev/cpu_dma_latency` ends when its file descriptor
//! closes, and a thread's nice value dies with the thread.

pub mod units;

use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use ug_hw::SysRoot;
use units::Units;

/// What a game session asks for. Built by the agent from the game's profile.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub cpu_performance: bool,
    pub cpu_no_idle_states: bool,
    pub cpu_priority: bool,
    pub memory_tuning: bool,
    pub pause_services: bool,
    /// Game processes, already checked to belong to the caller.
    pub pids: Vec<i32>,
}

/// What happened, in sentences the UI can list.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
    pub applied: Vec<String>,
    pub skipped: Vec<String>,
}

/// One change to undo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Undo {
    /// Restore a sysfs/procfs file's previous contents.
    File { path: PathBuf, previous: String },
    /// A cpufreq policy. Kept as one entry because the order matters:
    /// `intel_pstate` refuses EPP writes while the governor is `performance`,
    /// so the governor must be restored before the EPP.
    CpuPolicy { dir: PathBuf, governor: String, epp: Option<String> },
    /// Start a unit we stopped.
    StartUnit { unit: String },
    /// Restore an autogroup's nice value, via any process still in it.
    Autogroup { id: String, previous: i32 },
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Persisted {
    undo: Vec<Undo>,
}

/// Background units safe to hold off during a game.
///
/// Timers are stopped so they do not fire; their schedule resumes when they
/// are started again, and `Persistent=` timers catch up on anything missed.
/// Services are only listed where killing one mid-run is harmless (an index
/// rebuild just runs again). Package-manager *services* are deliberately
/// absent: stopping `apt` or `dpkg` mid-install can leave the system broken,
/// so only their timers are held.
pub const BACKGROUND_UNITS: &[&str] = &[
    // Debian / Ubuntu
    "apt-daily.timer",
    "apt-daily-upgrade.timer",
    // Fedora
    "dnf-makecache.timer",
    // Arch
    "archlinux-keyring-wkd-sync.timer",
    "pamac-cleancache.timer",
    // everywhere
    "fwupd-refresh.timer",
    "man-db.timer",
    "man-db.service",
    "plocate-updatedb.timer",
    "plocate-updatedb.service",
    "updatedb.timer",
    "logrotate.timer",
    "fstrim.timer",
    "e2scrub_all.timer",
];

pub struct Tuner<U: Units> {
    root: SysRoot,
    state_path: PathBuf,
    units: U,
    undo: Vec<Undo>,
    latency: Option<File>,
    active: bool,
}

impl<U: Units> Tuner<U> {
    /// Creates a tuner, first undoing anything a previous run left applied.
    pub fn new(root: SysRoot, state_path: PathBuf, units: U) -> (Self, Report) {
        let mut t = Tuner { root, state_path, units, undo: Vec::new(), latency: None, active: false };
        let leftovers: Persisted = std::fs::read_to_string(&t.state_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let report = if leftovers.undo.is_empty() {
            Report::default()
        } else {
            tracing::warn!(changes = leftovers.undo.len(), "game mode was left on by a previous run; restoring");
            t.undo = leftovers.undo;
            t.revert()
        };
        (t, report)
    }

    pub fn active(&self) -> bool {
        self.active
    }

    /// Turns game mode on, or — if it is already on for another game — adds
    /// these processes to it.
    pub fn enter(&mut self, req: &Request, caller_uid: u32) -> Report {
        let mut r = Report::default();
        let first = !self.active;
        self.active = true;

        if first {
            if req.cpu_performance {
                self.cpu_performance(&mut r);
            }
            if req.cpu_no_idle_states {
                self.hold_idle_states(&mut r);
            }
            if req.memory_tuning {
                self.memory(&mut r);
            }
            if req.pause_services {
                self.pause_services(&mut r);
            }
        }
        if req.cpu_priority {
            self.prioritise(&req.pids, caller_uid, &mut r);
        }
        r
    }

    /// Turns game mode off, restoring everything.
    pub fn leave(&mut self) -> Report {
        if !self.active && self.undo.is_empty() {
            return Report::default();
        }
        let r = self.revert();
        self.active = false;
        r
    }

    // ---- application --------------------------------------------------------

    /// Records an undo entry and writes it to disk before the change it undoes.
    fn record(&mut self, u: Undo) {
        self.undo.push(u);
        self.persist();
    }

    /// Records `undo`, then makes the change. If the change fails nothing was
    /// altered, so the entry is withdrawn again — otherwise the report would
    /// claim changes that never happened.
    fn change(&mut self, undo: Undo, apply: impl FnOnce() -> std::io::Result<()>) -> std::io::Result<()> {
        self.record(undo);
        apply().inspect_err(|_| {
            self.undo.pop();
            self.persist();
        })
    }

    fn persist(&self) {
        let doc = Persisted { undo: self.undo.clone() };
        let json = serde_json::to_vec_pretty(&doc).unwrap_or_default();
        if let Err(e) = ug_core::paths::write_atomic(&self.state_path, &json) {
            // Carry on: failing to record the undo is bad, but refusing to tune
            // because /var/lib is read-only would be worse for the user.
            tracing::error!(error = %e, path = %self.state_path.display(), "could not persist game-mode undo state");
        }
    }

    fn cpu_performance(&mut self, r: &mut Report) {
        let policies = policies(&self.root);
        if policies.is_empty() {
            r.skipped.push("CPU performance mode: this system exposes no cpufreq policies.".into());
            return;
        }
        let mut changed = 0;
        let mut before = None;
        let mut failure: Option<std::io::Error> = None;
        for dir in policies {
            let Some(governor) = read(&dir.join("scaling_governor")) else { continue };
            let available = read(&dir.join("scaling_available_governors")).unwrap_or_default();
            if !available.split_whitespace().any(|g| g == "performance") {
                continue;
            }
            let epp = read(&dir.join("energy_performance_preference"));
            if governor == "performance" && epp.as_deref().is_none_or(|e| e == "performance") {
                continue;
            }
            let was = format!("{governor}{}", epp.as_ref().map(|e| format!("/{e}")).unwrap_or_default());
            let gov_path = dir.join("scaling_governor");
            let epp_path = dir.join("energy_performance_preference");
            let has_epp = epp.is_some();
            let result = self.change(Undo::CpuPolicy { dir: dir.clone(), governor, epp }, || {
                write(&gov_path, "performance")?;
                if has_epp {
                    // Refused with EBUSY under intel_pstate once the governor
                    // is performance — which already implies that EPP.
                    let _ = write(&epp_path, "performance");
                }
                Ok(())
            });
            match result {
                Ok(()) => {
                    before.get_or_insert(was);
                    changed += 1;
                }
                Err(e) => {
                    failure.get_or_insert(e);
                }
            }
        }
        if let Some(b) = before {
            r.applied.push(format!("CPU set to performance on {changed} cores (was {b})"));
        } else if let Some(e) = failure {
            r.skipped.push(format!("CPU performance mode: {}", denied(&e)));
        } else {
            r.skipped.push("CPU performance mode: already in performance mode.".into());
        }
    }

    fn hold_idle_states(&mut self, r: &mut Report) {
        if self.latency.is_some() {
            return;
        }
        let path = self.root.join("dev/cpu_dma_latency");
        match std::fs::OpenOptions::new().write(true).open(&path) {
            Ok(mut f) => {
                // The request stays in force while this descriptor is open and
                // is dropped by the kernel the moment it closes — including when
                // this process dies, which is why it needs no undo entry.
                if f.write_all(&0i32.to_ne_bytes()).is_ok() {
                    self.latency = Some(f);
                    r.applied.push("Deep CPU idle states held off, so clocks do not drop between frames".into());
                } else {
                    r.skipped.push("CPU idle states: the kernel refused the latency request.".into());
                }
            }
            Err(e) => r.skipped.push(format!("CPU idle states: {} is not available ({e}).", path.display())),
        }
    }

    fn memory(&mut self, r: &mut Report) {
        // (sysctl, value, only-raise). max_map_count is only ever raised: a
        // system configured higher than our value already has what games need.
        const SETTINGS: &[(&str, u64, bool, &str)] = &[
            ("vm/swappiness", 10, false, "swapping reduced"),
            ("vm/max_map_count", 2_147_483_642, true, "memory map limit raised for games that need it"),
            ("vm/compaction_proactiveness", 0, false, "background memory compaction paused"),
        ];
        let mut done = Vec::new();
        let mut failure: Option<std::io::Error> = None;
        for &(key, value, only_raise, what) in SETTINGS {
            let path = self.root.join(format!("proc/sys/{key}"));
            let Some(prev) = read(&path) else { continue };
            let Ok(prev_n) = prev.parse::<u64>() else { continue };
            if prev_n == value || (only_raise && prev_n >= value) {
                continue;
            }
            let v = value.to_string();
            match self.change(Undo::File { path: path.clone(), previous: prev }, || write(&path, &v)) {
                Ok(()) => done.push(what),
                Err(e) => {
                    failure.get_or_insert(e);
                }
            }
        }
        if !done.is_empty() {
            r.applied.push(format!("Memory: {}", done.join(", ")));
        } else if let Some(e) = failure {
            r.skipped.push(format!("Memory tuning: {}", denied(&e)));
        } else {
            r.skipped.push("Memory tuning: already set.".into());
        }
    }

    fn pause_services(&mut self, r: &mut Report) {
        let mut paused = Vec::new();
        for unit in BACKGROUND_UNITS {
            if !self.units.is_active(unit) {
                continue;
            }
            self.record(Undo::StartUnit { unit: unit.to_string() });
            match self.units.stop(unit) {
                Ok(()) => paused.push(*unit),
                Err(e) => {
                    self.undo.pop();
                    self.persist();
                    tracing::warn!(unit, error = %e, "could not stop unit");
                }
            }
        }
        if paused.is_empty() {
            r.skipped.push("Background tasks: none were scheduled.".into());
        } else {
            r.applied.push(format!("Background tasks held until you stop playing: {}", paused.join(", ")));
        }
    }

    fn prioritise(&mut self, pids: &[i32], caller_uid: u32, r: &mut Report) {
        let mut threads = 0;
        let mut groups = Vec::new();
        for &pid in pids {
            // The request comes from an unprivileged session; the processes it
            // names must be the caller's own, or this would let any user raise
            // the priority of anything on the machine.
            if owner(&self.root, pid) != Some(caller_uid) {
                r.skipped.push(format!("Priority: process {pid} does not belong to you."));
                continue;
            }
            for tid in tasks(&self.root, pid) {
                // SAFETY: setpriority has no memory-safety preconditions.
                if unsafe { libc::setpriority(libc::PRIO_PROCESS, tid as libc::id_t, -10) } == 0 {
                    threads += 1;
                }
            }
            if let Some((id, nice)) = autogroup(&self.root, pid) {
                if !groups.contains(&id) && nice > -10 {
                    let path = self.root.join(format!("proc/{pid}/autogroup"));
                    if self.change(Undo::Autogroup { id: id.clone(), previous: nice }, || write(&path, "-10")).is_ok() {
                        groups.push(id);
                    }
                }
            }
        }
        if threads > 0 {
            // With autogroup scheduling, per-thread nice only orders threads
            // within the game's own group; the group's nice is what gives the
            // game an edge over everything else on the desktop.
            let how = if groups.is_empty() { "" } else { " (and its scheduling group)" };
            r.applied.push(format!("Game given higher CPU priority: {threads} threads{how}"));
        }
    }

    // ---- reversal -----------------------------------------------------------

    fn revert(&mut self) -> Report {
        self.latency = None; // closing the fd releases the idle-state hold
        let mut r = Report::default();
        let mut failed = Vec::new();
        while let Some(u) = self.undo.pop() {
            let ok = match &u {
                Undo::File { path, previous } => write(path, previous).is_ok(),
                Undo::CpuPolicy { dir, governor, epp } => {
                    let g = write(&dir.join("scaling_governor"), governor).is_ok();
                    if let Some(e) = epp {
                        let _ = write(&dir.join("energy_performance_preference"), e);
                    }
                    g
                }
                Undo::StartUnit { unit } => self.units.start(unit).is_ok(),
                Undo::Autogroup { id, previous } => {
                    // The game has usually exited; any surviving member of the
                    // group (the launcher, say) can carry the write. None left
                    // means the group is gone and there is nothing to restore.
                    match member_of(&self.root, id) {
                        Some(pid) => write(&self.root.join(format!("proc/{pid}/autogroup")), &previous.to_string()).is_ok(),
                        None => true,
                    }
                }
            };
            if !ok {
                failed.push(u);
            }
            self.persist();
        }
        if failed.is_empty() {
            r.applied.push("Everything game mode changed has been restored".into());
        } else {
            for u in &failed {
                r.skipped.push(format!("Could not restore {u:?}"));
            }
            // Keep them for the next attempt rather than forgetting them.
            self.undo = failed;
            self.persist();
        }
        r
    }
}

// ---- helpers ----------------------------------------------------------------

/// Explains a failed write in terms a user can act on.
fn denied(e: &std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        "not permitted — the Unix Guardian service must run as root to change this.".into()
    } else {
        format!("the kernel refused the change ({e}).")
    }
}

fn read(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

fn write(p: &Path, v: &str) -> std::io::Result<()> {
    std::fs::write(p, v)
}

fn policies(root: &SysRoot) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(root.join("sys/devices/system/cpu/cpufreq"))
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("policy")))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn owner(root: &SysRoot, pid: i32) -> Option<u32> {
    let s = std::fs::read_to_string(root.join(format!("proc/{pid}/status"))).ok()?;
    s.lines().find_map(|l| l.strip_prefix("Uid:"))?.split_whitespace().next()?.parse().ok()
}

fn tasks(root: &SysRoot, pid: i32) -> Vec<i32> {
    std::fs::read_dir(root.join(format!("proc/{pid}/task")))
        .map(|d| d.filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok()).collect())
        .unwrap_or_default()
}

/// `(group id, nice)` from `/proc/<pid>/autogroup`, which reads like
/// `/autogroup-4288 nice 0`. Absent when autogroup scheduling is off.
fn autogroup(root: &SysRoot, pid: i32) -> Option<(String, i32)> {
    let s = read(&root.join(format!("proc/{pid}/autogroup")))?;
    let mut parts = s.split_whitespace();
    let id = parts.next()?.to_string();
    let nice = parts.nth(1)?.parse().ok()?;
    Some((id, nice))
}

/// Any process currently in autogroup `id`.
fn member_of(root: &SysRoot, id: &str) -> Option<i32> {
    std::fs::read_dir(root.join("proc")).ok()?.filter_map(|e| e.ok()).find_map(|e| {
        let pid: i32 = e.file_name().to_str()?.parse().ok()?;
        (autogroup(root, pid)?.0 == id).then_some(pid)
    })
}

#[cfg(test)]
mod tests;
