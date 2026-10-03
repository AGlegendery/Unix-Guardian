//! Runs automatic fixes, step by step, and undoes them.
//!
//! The way back is recorded before each step does anything: a file's old
//! contents are copied to `backups/driver-<id>/` (or "there was no file" is
//! noted), and only then is the new file written; units are enabled only
//! after the ones that were off are written down. Everything is kept in
//! `driver-ops.json` in the daemon's state directory, so an undo works after
//! a restart — from a text console with `ug driver rollback`, which is where
//! someone whose graphics broke will be. `driver-recovery.txt` next to it
//! has the same undo as plain commands, for when even that is not possible.
//!
//! A step that fails undoes the steps before it at once.
//!
//! Programs (the initramfs rebuild, `systemctl`) run through a [`Runner`]:
//! the daemon's runs them in a transient systemd unit, outside the daemon's
//! own sandbox; development runs and tests record them instead.

use crate::fixes::{AutoFix, Step};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use ug_hw::SysRoot;
use ug_proto::DriverOp;

pub trait Runner: Send + Sync {
    /// Runs a program to the end; its output, or why it failed.
    fn run(&self, argv: &[String]) -> Result<String, String>;
}

/// Runs each program as a transient systemd unit and waits for it. The
/// daemon's unit restricts capabilities (an initramfs rebuild needs
/// chown/mknod), and `systemd-run` asks PID 1 instead.
pub struct SystemdRun;

impl Runner for SystemdRun {
    fn run(&self, argv: &[String]) -> Result<String, String> {
        let unit = format!("unix-guardian-driver-{}", now_ms());
        let out = std::process::Command::new("systemd-run")
            .args(["--wait", "--pipe", "--collect", "--quiet", "--service-type=exec", "--unit", &unit, "--"])
            .args(argv)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("could not start systemd-run: {e}"))?;
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        if out.status.success() {
            Ok(text)
        } else {
            Err(format!("`{}` failed ({}): {}", argv.join(" "), out.status, text.trim()))
        }
    }
}

/// Records programs instead of running them; can be told to fail one.
#[derive(Default)]
pub struct Recorder {
    pub ran: Mutex<Vec<Vec<String>>>,
    pub fail: Mutex<Option<String>>,
}

impl Runner for Recorder {
    fn run(&self, argv: &[String]) -> Result<String, String> {
        self.ran.lock().unwrap_or_else(|e| e.into_inner()).push(argv.to_vec());
        if let Some(f) = self.fail.lock().unwrap_or_else(|e| e.into_inner()).as_deref() {
            if argv.first().is_some_and(|a| a == f) {
                return Err(format!("`{}` failed (simulated)", argv.join(" ")));
            }
        }
        Ok(format!("(recorded, not run) {}", argv.join(" ")))
    }
}

/// How one step is undone.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
enum Undo {
    /// Put `backup` back at `path`, or remove `path` when there was no file.
    Restore { path: String, backup: Option<String> },
    Disable {
        units: Vec<String>,
        /// Also stop them (they were started with `--now`).
        #[serde(default)]
        now: bool,
    },
    Initramfs { argv: Vec<String> },
    /// Write `value` back to a sysfs switch.
    Switch { path: String, value: String },
    /// Unload a module that was not loaded before.
    Unload { module: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    id: String,
    fix_id: String,
    title: String,
    started_unix_ms: u64,
    state: String,
    log: Vec<String>,
    needs_reboot: bool,
    undo: Vec<Undo>,
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub struct Executor {
    root: SysRoot,
    dir: PathBuf,
    runner: Box<dyn Runner>,
    records: Mutex<Vec<Record>>,
}

impl Executor {
    /// Loads earlier operations. One that was running when the daemon
    /// stopped is marked interrupted: its undo list is still exact, because
    /// each undo was saved before its step ran.
    pub fn new(root: SysRoot, state_dir: &Path, runner: Box<dyn Runner>) -> Executor {
        let mut records: Vec<Record> = std::fs::read_to_string(state_dir.join("driver-ops.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        for r in &mut records {
            if r.state == "running" || r.state == "undoing" {
                r.state = "interrupted".into();
                r.log.push("Interrupted (the service stopped mid-way). Undo puts back what was done.".into());
            }
        }
        Executor { root, dir: state_dir.to_path_buf(), runner, records: Mutex::new(records) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Record>> {
        self.records.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn real(&self, abs: &str) -> PathBuf {
        self.root.join(abs.trim_start_matches('/'))
    }

    fn save(&self, records: &[Record]) {
        let text = serde_json::to_string_pretty(records).unwrap_or_default();
        if let Err(e) = ug_core::paths::write_atomic(&self.dir.join("driver-ops.json"), text.as_bytes()) {
            tracing::error!(error = %e, "could not save the driver operations");
        }
        let _ = ug_core::paths::write_atomic(&self.dir.join("driver-recovery.txt"), recovery_text(records, &self.dir).as_bytes());
    }

    fn update(&self, id: &str, f: impl FnOnce(&mut Record)) {
        let mut all = self.lock();
        if let Some(r) = all.iter_mut().find(|r| r.id == id) {
            f(r);
        }
        self.save(&all);
    }

    fn log(&self, id: &str, line: impl Into<String>) {
        let line = line.into();
        tracing::info!(op = id, "{line}");
        self.update(id, |r| r.log.push(line));
    }

    pub fn ops(&self) -> Vec<DriverOp> {
        let mut v: Vec<DriverOp> = self
            .lock()
            .iter()
            .map(|r| DriverOp {
                id: r.id.clone(),
                fix_id: r.fix_id.clone(),
                title: r.title.clone(),
                started_unix_ms: r.started_unix_ms,
                state: r.state.clone(),
                log: r.log.clone(),
                needs_reboot: r.needs_reboot,
            })
            .collect();
        v.sort_by(|a, b| b.started_unix_ms.cmp(&a.started_unix_ms));
        v
    }

    /// Records a new operation; `run` then does it. Refused while another
    /// one is running.
    pub fn start(&self, fix: &AutoFix) -> Result<String, String> {
        let mut all = self.lock();
        if all.iter().any(|r| r.state == "running" || r.state == "undoing") {
            return Err("Another driver change is still running; wait for it to finish.".into());
        }
        // Two changes in the same millisecond must not share an id.
        let mut id = format!("d{}", now_ms());
        while all.iter().any(|r| r.id == id) {
            id.push('x');
        }
        all.push(Record {
            id: id.clone(),
            fix_id: fix.fix_id.clone(),
            title: fix.title.clone(),
            started_unix_ms: now_ms(),
            state: "running".into(),
            log: vec![format!("Started: {}", fix.title)],
            needs_reboot: fix.needs_reboot,
            undo: Vec::new(),
        });
        self.save(&all);
        Ok(id)
    }

    fn push_undo(&self, id: &str, u: Undo) {
        self.update(id, |r| r.undo.push(u));
    }

    fn do_step(&self, id: &str, n: usize, step: &Step) -> Result<(), String> {
        match step {
            Step::Write { path, content } => {
                let target = self.real(path);
                let backup = if target.exists() {
                    let b = self.dir.join(format!("backups/driver-{id}/{n}-{}", path.rsplit('/').next().unwrap_or("file")));
                    std::fs::create_dir_all(b.parent().unwrap_or(&self.dir)).map_err(|e| e.to_string())?;
                    std::fs::copy(&target, &b).map_err(|e| format!("backing up {path}: {e}"))?;
                    Some(b.display().to_string())
                } else {
                    None
                };
                self.push_undo(id, Undo::Restore { path: path.clone(), backup: backup.clone() });
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                ug_core::paths::write_atomic(&target, content.as_bytes()).map_err(|e| format!("writing {path}: {e}"))?;
                self.log(id, match backup {
                    Some(b) => format!("Wrote {path} (the previous file is saved as {b})"),
                    None => format!("Wrote {path}"),
                });
            }
            Step::EnableUnits { units, now } => {
                let off: Vec<String> = units
                    .iter()
                    .filter(|u| crate::inventory::unit_exists(&self.root, u) && !crate::inventory::unit_enabled(&self.root, u))
                    .cloned()
                    .collect();
                if off.is_empty() {
                    self.log(id, format!("Already enabled: {}", units.join(", ")));
                    return Ok(());
                }
                self.push_undo(id, Undo::Disable { units: off.clone(), now: *now });
                let flags: &[&str] = if *now { &["systemctl", "enable", "--now"] } else { &["systemctl", "enable"] };
                let argv: Vec<String> = flags.iter().map(|s| s.to_string()).chain(off.iter().cloned()).collect();
                let out = self.runner.run(&argv)?;
                self.log(id, format!("Enabled {}{}", off.join(", "), trail(&out)));
            }
            Step::SysWrite { path, value } => {
                let target = self.real(path);
                let old = std::fs::read_to_string(&target).map_err(|e| format!("reading {path}: {e}"))?.trim().to_string();
                if old == *value {
                    self.log(id, format!("{path} is already {value}"));
                    return Ok(());
                }
                self.push_undo(id, Undo::Switch { path: path.clone(), value: old.clone() });
                std::fs::write(&target, value).map_err(|e| format!("writing {path}: {e}"))?;
                self.log(id, format!("Set {path} to {value} (was {old})"));
            }
            Step::Modprobe { module } => {
                let loaded = std::fs::read_to_string(self.root.join("proc/modules"))
                    .unwrap_or_default()
                    .lines()
                    .any(|l| l.split_whitespace().next() == Some(module.as_str()));
                if loaded {
                    self.log(id, format!("{module} is already loaded"));
                    return Ok(());
                }
                self.push_undo(id, Undo::Unload { module: module.clone() });
                let out = self.runner.run(&["modprobe".to_string(), module.clone()])?;
                self.log(id, format!("Loaded {module}{}", trail(&out)));
            }
            Step::Initramfs { argv } => {
                self.log(id, format!("Running {} (up to a minute)…", argv.join(" ")));
                self.push_undo(id, Undo::Initramfs { argv: argv.clone() });
                let out = self.runner.run(argv)?;
                self.log(id, format!("Initramfs rebuilt{}", trail(&out)));
            }
        }
        Ok(())
    }

    /// Does an operation's steps; on a failure, undoes what was done.
    pub fn run(&self, id: &str, fix: &AutoFix) {
        for (n, step) in fix.steps.iter().enumerate() {
            if let Err(e) = self.do_step(id, n, step) {
                self.log(id, format!("Failed: {e}"));
                self.log(id, "Putting back what was changed…");
                self.undo_all(id);
                self.update(id, |r| r.state = "failed".into());
                return;
            }
        }
        self.update(id, |r| r.state = "done".into());
        if fix.needs_reboot {
            self.log(id, "Done. Restart for it to take effect. If the screen stays black after the restart, press Ctrl+Alt+F3, log in, and run: ug driver rollback");
        } else {
            self.log(id, "Done.");
        }
    }

    /// Undoes the newest operation that is not undone yet, or the given one.
    pub fn rollback(&self, id: Option<&str>) -> Result<String, String> {
        let id = {
            let all = self.lock();
            let pick = match id {
                Some(i) => all.iter().find(|r| r.id == i),
                None => all.iter().filter(|r| matches!(r.state.as_str(), "done" | "interrupted")).max_by_key(|r| r.started_unix_ms),
            };
            let r = pick.ok_or_else(|| match id {
                Some(i) => format!("No driver change `{i}`."),
                None => "There is no driver change to undo.".to_string(),
            })?;
            if !matches!(r.state.as_str(), "done" | "interrupted") {
                return Err(format!("`{}` is {}, so there is nothing to undo.", r.id, r.state));
            }
            r.id.clone()
        };
        self.update(&id, |r| r.state = "undoing".into());
        let ok = self.undo_all(&id);
        self.update(&id, |r| r.state = if ok { "undone".into() } else { "interrupted".into() });
        Ok(id)
    }

    /// Runs the undo list backwards, the initramfs last. True if all of it
    /// worked.
    fn undo_all(&self, id: &str) -> bool {
        let undo: Vec<Undo> = self.lock().iter().find(|r| r.id == id).map(|r| r.undo.clone()).unwrap_or_default();
        let mut ok = true;
        let mut initramfs = None;
        for u in undo.iter().rev() {
            let r: Result<String, String> = match u {
                Undo::Restore { path, backup: Some(b) } => std::fs::read(b)
                    .and_then(|data| ug_core::paths::write_atomic(&self.real(path), &data))
                    .map(|_| format!("Put back {path}"))
                    .map_err(|e| format!("putting back {path}: {e}")),
                Undo::Restore { path, backup: None } => match std::fs::remove_file(self.real(path)) {
                    Ok(()) => Ok(format!("Removed {path}")),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(format!("{path} was not there")),
                    Err(e) => Err(format!("removing {path}: {e}")),
                },
                Undo::Disable { units, now } => {
                    let flags: &[&str] = if *now { &["systemctl", "disable", "--now"] } else { &["systemctl", "disable"] };
                    let argv: Vec<String> = flags.iter().map(|s| s.to_string()).chain(units.iter().cloned()).collect();
                    self.runner.run(&argv).map(|_| format!("Disabled {}", units.join(", ")))
                }
                Undo::Switch { path, value } => std::fs::write(self.real(path), value)
                    .map(|_| format!("Set {path} back to {value}"))
                    .map_err(|e| format!("writing {path}: {e}")),
                Undo::Unload { module } => {
                    self.runner.run(&["modprobe".to_string(), "-r".to_string(), module.clone()]).map(|_| format!("Unloaded {module}"))
                }
                Undo::Initramfs { argv } => {
                    initramfs = Some(argv.clone());
                    continue;
                }
            };
            match r {
                Ok(m) => self.log(id, m),
                Err(e) => {
                    ok = false;
                    self.log(id, format!("Could not undo: {e}"));
                }
            }
        }
        if let Some(argv) = initramfs {
            self.log(id, format!("Running {}…", argv.join(" ")));
            match self.runner.run(&argv) {
                Ok(out) => self.log(id, format!("Initramfs rebuilt{}", trail(&out))),
                Err(e) => {
                    ok = false;
                    self.log(id, format!("Could not rebuild the initramfs: {e}"));
                }
            }
        }
        self.update(id, |r| {
            if ok {
                r.undo.clear();
            }
        });
        ok
    }
}

/// The last line of a program's output, for the log.
fn trail(out: &str) -> String {
    match out.trim().lines().last() {
        Some(l) if !l.is_empty() => format!(": {l}"),
        _ => String::new(),
    }
}

/// Plain commands that undo every change still in place.
fn recovery_text(records: &[Record], _dir: &Path) -> String {
    let mut t = String::from(
        "Unix Guardian driver changes, and how to undo them by hand.\n\
         Easiest, from a text console (Ctrl+Alt+F3): ug driver rollback\n\n",
    );
    let open: Vec<&Record> = records.iter().filter(|r| !r.undo.is_empty()).collect();
    if open.is_empty() {
        t.push_str("Nothing to undo.\n");
    }
    for r in open {
        t.push_str(&format!("# {} ({}, {})\n", r.title, r.id, r.state));
        let mut initramfs = None;
        for u in r.undo.iter().rev() {
            match u {
                Undo::Restore { path, backup: Some(b) } => t.push_str(&format!("sudo cp '{b}' '{path}'\n")),
                Undo::Restore { path, backup: None } => t.push_str(&format!("sudo rm -f '{path}'\n")),
                Undo::Disable { units, now } => {
                    let flag = if *now { " --now" } else { "" };
                    t.push_str(&format!("sudo systemctl disable{flag} {}\n", units.join(" ")))
                }
                Undo::Switch { path, value } => t.push_str(&format!("echo {value} | sudo tee '{path}'\n")),
                Undo::Unload { module } => t.push_str(&format!("sudo modprobe -r {module}\n")),
                Undo::Initramfs { argv } => initramfs = Some(argv.join(" ")),
            }
        }
        if let Some(i) = initramfs {
            t.push_str(&format!("sudo {i}\n"));
        }
        t.push('\n');
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixes::{auto, FILE_POWER};
    use std::sync::Arc;
    use ug_testfs::TestFs;

    struct Shared(Arc<Recorder>);
    impl Runner for Shared {
        fn run(&self, argv: &[String]) -> Result<String, String> {
            self.0.run(argv)
        }
    }

    fn machine() -> (TestFs, tempfile::TempDir, Arc<Recorder>, Executor) {
        let fs = TestFs::new();
        fs.write("etc/os-release", "ID=debian\nPRETTY_NAME=Debian\n")
            .write("usr/sbin/update-initramfs", "")
            .write("etc/modprobe.d/nvidia-options.conf", "options nvidia NVreg_UsePageAttributeTable=1\n")
            .write("usr/lib/systemd/system/nvidia-suspend.service", "")
            .write("usr/lib/systemd/system/nvidia-resume.service", "")
            .write("usr/lib/systemd/system/nvidia-hibernate.service", "")
            .write("etc/systemd/system/systemd-hibernate.service.wants/nvidia-hibernate.service", "");
        let state = tempfile::tempdir().unwrap();
        let rec = Arc::new(Recorder::default());
        let ex = Executor::new(SysRoot::at(fs.root()), state.path(), Box::new(Shared(Arc::clone(&rec))));
        (fs, state, rec, ex)
    }

    fn snapshot(fs: &TestFs) -> Vec<(String, Vec<u8>)> {
        let mut v = Vec::new();
        fn walk(d: &Path, base: &Path, v: &mut Vec<(String, Vec<u8>)>) {
            for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                if e.path().is_dir() {
                    walk(&e.path(), base, v);
                } else {
                    v.push((e.path().strip_prefix(base).unwrap().display().to_string(), std::fs::read(e.path()).unwrap()));
                }
            }
        }
        walk(&fs.root().join("etc"), fs.root(), &mut v);
        v.sort();
        v
    }

    #[test]
    fn a_fix_is_applied_then_undone_back_to_the_byte() {
        let (fs, state, rec, ex) = machine();
        let root = SysRoot::at(fs.root());
        let facts = crate::inventory::facts(&root, &Default::default());
        let before = snapshot(&fs);
        let fix = auto("nvidia-suspend", &root, &facts).unwrap();
        let id = ex.start(&fix).unwrap();
        ex.run(&id, &fix);
        assert_eq!(ex.ops()[0].state, "done");
        assert!(std::fs::read_to_string(fs.root().join(FILE_POWER.trim_start_matches('/'))).unwrap().contains("PreserveVideoMemoryAllocations=1"));
        let ran = rec.ran.lock().unwrap().clone();
        assert_eq!(ran[0], ["systemctl", "enable", "nvidia-suspend.service", "nvidia-resume.service"], "only the units that were off");
        assert_eq!(ran[1][0], "update-initramfs");
        let recovery = std::fs::read_to_string(state.path().join("driver-recovery.txt")).unwrap();
        assert!(recovery.contains(&format!("sudo rm -f '{FILE_POWER}'")) && recovery.contains("sudo systemctl disable nvidia-suspend.service nvidia-resume.service"));

        // Undone after a "restart": a new executor reads the saved list.
        let ex = Executor::new(root.clone(), state.path(), Box::new(Shared(Arc::clone(&rec))));
        ex.rollback(None).unwrap();
        assert_eq!(ex.ops()[0].state, "undone");
        assert_eq!(snapshot(&fs), before, "every file as it was");
        let ran = rec.ran.lock().unwrap().clone();
        assert_eq!(ran[2], ["systemctl", "disable", "nvidia-suspend.service", "nvidia-resume.service"]);
        assert_eq!(ran[3][0], "update-initramfs", "the initramfs is rebuilt without our file");
        assert!(ex.rollback(None).is_err(), "nothing left to undo");
        assert!(std::fs::read_to_string(state.path().join("driver-recovery.txt")).unwrap().contains("Nothing to undo"));
    }

    #[test]
    fn an_existing_file_is_backed_up_and_put_back() {
        let (fs, _state, _rec, ex) = machine();
        fs.write("etc/modprobe.d/unix-guardian-nvidia-drm.conf", "options nvidia-drm modeset=0\n");
        let root = SysRoot::at(fs.root());
        let before = snapshot(&fs);
        let fix = auto("nvidia-drm-modeset", &root, &crate::inventory::facts(&root, &Default::default())).unwrap();
        let id = ex.start(&fix).unwrap();
        ex.run(&id, &fix);
        assert!(ex.ops()[0].log.iter().any(|l| l.contains("previous file is saved")));
        ex.rollback(Some(&id)).unwrap();
        assert_eq!(snapshot(&fs), before);
    }

    #[test]
    fn a_failing_step_undoes_what_was_done_and_nothing_runs_twice_at_once() {
        let (fs, _state, rec, ex) = machine();
        let root = SysRoot::at(fs.root());
        let before = snapshot(&fs);
        *rec.fail.lock().unwrap() = Some("update-initramfs".into());
        let fix = auto("nouveau-blacklist", &root, &crate::inventory::facts(&root, &Default::default())).unwrap();
        let id = ex.start(&fix).unwrap();
        assert!(ex.start(&fix).unwrap_err().contains("still running"));
        ex.run(&id, &fix);
        let op = &ex.ops()[0];
        assert_eq!(op.state, "failed");
        assert!(op.log.iter().any(|l| l.contains("Removed /etc/modprobe.d/unix-guardian-blacklist-nouveau.conf")));
        assert_eq!(snapshot(&fs), before, "the blacklist file is gone again");
        assert!(ex.rollback(Some(&id)).is_err(), "a failed change has nothing left to undo");
    }

    #[test]
    fn an_operation_cut_off_mid_way_can_still_be_undone() {
        let (fs, state, rec, ex) = machine();
        let root = SysRoot::at(fs.root());
        let before = snapshot(&fs);
        let fix = auto("nouveau-blacklist", &root, &crate::inventory::facts(&root, &Default::default())).unwrap();
        let id = ex.start(&fix).unwrap();
        // Power cut after the file was written, before the initramfs step.
        ex.do_step(&id, 0, &fix.steps[0]).unwrap();
        drop(ex);
        let ex = Executor::new(root, state.path(), Box::new(Shared(Arc::clone(&rec))));
        assert_eq!(ex.ops()[0].state, "interrupted");
        ex.rollback(None).unwrap();
        assert_eq!(snapshot(&fs), before);
    }

    #[test]
    fn live_fixes_flip_switches_start_units_and_load_modules_and_undo_them() {
        let (fs, state, rec, ex) = machine();
        fs.write("sys/class/rfkill/rfkill0/type", "wlan\n").write("sys/class/rfkill/rfkill0/soft", "1\n").write("sys/class/rfkill/rfkill0/hard", "0\n");
        fs.write("usr/lib/systemd/system/cups.socket", "").write("usr/lib/systemd/system/cups.service", "");
        fs.write("proc/modules", "i915 1 0 - Live 0x0\n");
        let root = SysRoot::at(fs.root());
        let facts = crate::inventory::facts(&root, &Default::default());
        let before = snapshot(&fs);
        for id in ["rfkill-unblock-wifi", "enable-cups", "load-uvcvideo"] {
            let fix = auto(id, &root, &facts).unwrap();
            let op = ex.start(&fix).unwrap();
            ex.run(&op, &fix);
            assert_eq!(ex.ops()[0].state, "done", "{id}: {:?}", ex.ops()[0].log);
        }
        assert_eq!(std::fs::read_to_string(fs.root().join("sys/class/rfkill/rfkill0/soft")).unwrap(), "0");
        let ran = rec.ran.lock().unwrap().clone();
        assert_eq!(ran[0], ["systemctl", "enable", "--now", "cups.socket", "cups.service"]);
        assert_eq!(ran[1], ["modprobe", "uvcvideo"]);
        let recovery = std::fs::read_to_string(state.path().join("driver-recovery.txt")).unwrap();
        assert!(recovery.contains("echo 1 | sudo tee '/sys/class/rfkill/rfkill0/soft'") && recovery.contains("sudo modprobe -r uvcvideo"));
        for _ in 0..3 {
            ex.rollback(None).unwrap();
        }
        let ran = rec.ran.lock().unwrap().clone();
        assert_eq!(ran[2], ["modprobe", "-r", "uvcvideo"]);
        assert_eq!(ran[3], ["systemctl", "disable", "--now", "cups.socket", "cups.service"]);
        assert_eq!(std::fs::read_to_string(fs.root().join("sys/class/rfkill/rfkill0/soft")).unwrap(), "1");
        assert_eq!(snapshot(&fs), before, "the modules-load.d file is gone again");
        assert!(ex.rollback(None).is_err());
    }
}
