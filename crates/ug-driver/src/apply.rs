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
            .args([
                "--wait",
                "--pipe",
                "--collect",
                "--quiet",
                "--service-type=exec",
                "--unit",
                &unit,
                "--",
            ])
            .args(argv)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("could not start systemd-run: {e}"))?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if out.status.success() {
            Ok(text)
        } else {
            Err(format!(
                "`{}` failed ({}): {}",
                argv.join(" "),
                out.status,
                text.trim()
            ))
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
        self.ran
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(argv.to_vec());
        if let Some(f) = self
            .fail
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_deref()
        {
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
    Restore {
        path: String,
        backup: Option<String>,
    },
    Disable {
        units: Vec<String>,
        /// Also stop them (they were started with `--now`).
        #[serde(default)]
        now: bool,
    },
    Initramfs {
        argv: Vec<String>,
    },
    /// Write `value` back to a sysfs switch.
    Switch {
        path: String,
        value: String,
    },
    /// Unload a module that was not loaded before.
    Unload {
        module: String,
    },
    /// Remove the packages an install added.
    Remove {
        family: String,
        packages: Vec<String>,
    },
    /// Run again once the files are back (a boot menu generator).
    Rerun {
        argv: Vec<String>,
    },
    /// Run this program (the opposite of one that ran).
    Command {
        argv: Vec<String>,
    },
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
    /// How to go back to the snapshot taken before it, by hand.
    #[serde(default)]
    restore: Vec<String>,
    /// The boot it finished in, for the check after the restart.
    #[serde(default)]
    done_boot: String,
    /// `restart`, `check`, `kept` or "" (see DriverOp::verify).
    #[serde(default)]
    verify: String,
}

/// This boot's id; it changes at every start.
pub fn boot_id(root: &SysRoot) -> String {
    std::fs::read_to_string(root.join("proc/sys/kernel/random/boot_id"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// Whether someone is logged in to a desktop (a logind session of type
/// wayland or x11).
pub fn desktop_session(root: &SysRoot) -> bool {
    std::fs::read_dir(root.join("run/systemd/sessions"))
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| {
            std::fs::read_to_string(e.path())
                .is_ok_and(|t| t.lines().any(|l| l == "TYPE=wayland" || l == "TYPE=x11"))
        })
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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
        let boot = boot_id(&root);
        for r in &mut records {
            if r.state == "running" || r.state == "undoing" {
                r.state = "interrupted".into();
                r.log.push(
                    "Interrupted (the service stopped mid-way). Undo puts back what was done."
                        .into(),
                );
            }
            if r.verify == "restart" && r.state == "done" && !boot.is_empty() && r.done_boot != boot
            {
                r.verify = "check".into();
                r.log.push(
                    "Restarted. Check that everything works, then keep the change or undo it."
                        .into(),
                );
            }
        }
        let ex = Executor {
            root,
            dir: state_dir.to_path_buf(),
            runner,
            records: Mutex::new(records),
        };
        ex.save(&ex.lock());
        ex
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Record>> {
        self.records.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn real(&self, abs: &str) -> PathBuf {
        self.root.join(abs.trim_start_matches('/'))
    }

    fn save(&self, records: &[Record]) {
        let text = serde_json::to_string_pretty(records).unwrap_or_default();
        if let Err(e) =
            ug_core::paths::write_atomic(&self.dir.join("driver-ops.json"), text.as_bytes())
        {
            tracing::error!(error = %e, "could not save the driver operations");
        }
        let _ = ug_core::paths::write_atomic(
            &self.dir.join("driver-recovery.txt"),
            recovery_text(records, &self.dir).as_bytes(),
        );
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
                verify: r.verify.clone(),
            })
            .collect();
        v.sort_by(|a, b| b.started_unix_ms.cmp(&a.started_unix_ms));
        v
    }

    /// Records a new operation; `run` then does it. Refused while another
    /// one is running.
    pub fn start(&self, fix: &AutoFix) -> Result<String, String> {
        let mut all = self.lock();
        if all
            .iter()
            .any(|r| r.state == "running" || r.state == "undoing")
        {
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
            restore: Vec::new(),
            done_boot: String::new(),
            verify: String::new(),
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
                    let b = self.dir.join(format!(
                        "backups/driver-{id}/{n}-{}",
                        path.rsplit('/').next().unwrap_or("file")
                    ));
                    std::fs::create_dir_all(b.parent().unwrap_or(&self.dir))
                        .map_err(|e| e.to_string())?;
                    std::fs::copy(&target, &b).map_err(|e| format!("backing up {path}: {e}"))?;
                    Some(b.display().to_string())
                } else {
                    None
                };
                self.push_undo(
                    id,
                    Undo::Restore {
                        path: path.clone(),
                        backup: backup.clone(),
                    },
                );
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                ug_core::paths::write_atomic(&target, content.as_bytes())
                    .map_err(|e| format!("writing {path}: {e}"))?;
                self.log(
                    id,
                    match backup {
                        Some(b) => format!("Wrote {path} (the previous file is saved as {b})"),
                        None => format!("Wrote {path}"),
                    },
                );
            }
            Step::EnableUnits { units, now } => {
                let off: Vec<String> = units
                    .iter()
                    .filter(|u| {
                        crate::inventory::unit_exists(&self.root, u)
                            && !crate::inventory::unit_enabled(&self.root, u)
                    })
                    .cloned()
                    .collect();
                if off.is_empty() {
                    self.log(id, format!("Already enabled: {}", units.join(", ")));
                    return Ok(());
                }
                self.push_undo(
                    id,
                    Undo::Disable {
                        units: off.clone(),
                        now: *now,
                    },
                );
                let flags: &[&str] = if *now {
                    &["systemctl", "enable", "--now"]
                } else {
                    &["systemctl", "enable"]
                };
                let argv: Vec<String> = flags
                    .iter()
                    .map(|s| s.to_string())
                    .chain(off.iter().cloned())
                    .collect();
                let out = self.runner.run(&argv)?;
                self.log(id, format!("Enabled {}{}", off.join(", "), trail(&out)));
            }
            Step::SysWrite { path, value } => {
                let target = self.real(path);
                let old = std::fs::read_to_string(&target)
                    .map_err(|e| format!("reading {path}: {e}"))?
                    .trim()
                    .to_string();
                if old == *value {
                    self.log(id, format!("{path} is already {value}"));
                    return Ok(());
                }
                self.push_undo(
                    id,
                    Undo::Switch {
                        path: path.clone(),
                        value: old.clone(),
                    },
                );
                std::fs::write(&target, value).map_err(|e| format!("writing {path}: {e}"))?;
                self.log(id, format!("Set {path} to {value} (was {old})"));
            }
            Step::Install { family, packages } => {
                let before = crate::snapshot::installed_packages(&self.root);
                // Written down before anything runs: the requested packages
                // that are new. After the install it becomes the real list
                // (with what they pulled in).
                let new: Vec<String> = packages
                    .iter()
                    .filter(|p| !before.contains(p))
                    .cloned()
                    .collect();
                if new.is_empty() {
                    self.log(id, format!("Already installed: {}", packages.join(", ")));
                    return Ok(());
                }
                self.push_undo(
                    id,
                    Undo::Remove {
                        family: family.clone(),
                        packages: new.clone(),
                    },
                );
                if let Some(r) = crate::packages::refresh_argv(family) {
                    self.log(id, format!("Running {}…", r.join(" ")));
                    let out = self.runner.run(&r)?;
                    self.log(id, format!("Package lists refreshed{}", trail(&out)));
                }
                let argv = crate::packages::install_argv(family, packages);
                self.log(id, format!("Running {} (a few minutes)…", argv.join(" ")));
                let out = self.runner.run(&argv)?;
                let mut added = crate::packages::added(
                    &before,
                    &crate::snapshot::installed_packages(&self.root),
                );
                if added.is_empty() {
                    added = new;
                }
                let list = added.clone();
                self.update(id, |r| {
                    if let Some(Undo::Remove { packages, .. }) = r.undo.last_mut() {
                        *packages = list;
                    }
                });
                self.log(id, format!("Installed {}{}", added.join(", "), trail(&out)));
            }
            Step::Regen { argv } => {
                self.push_undo(id, Undo::Rerun { argv: argv.clone() });
                let out = self.runner.run(argv)?;
                self.log(id, format!("Ran {}{}", argv.join(" "), trail(&out)));
            }
            Step::HibernateSwap { plan } => {
                let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<String>>();
                let mut make = crate::swap::make_argv(plan).into_iter();
                if let Some(sv) = &plan.new_subvolume {
                    // Written ahead, removed last (after the file in it).
                    self.push_undo(
                        id,
                        Undo::Command {
                            argv: v(&["btrfs", "subvolume", "delete", sv]),
                        },
                    );
                    let argv = make.next().expect("the subvolume first");
                    self.runner.run(&argv)?;
                    self.log(id, format!("Made the subvolume {sv}"));
                }
                self.push_undo(
                    id,
                    Undo::Restore {
                        path: plan.file.clone(),
                        backup: None,
                    },
                );
                self.log(id, format!("Making {} ({} GB)…", plan.file, plan.size_gib));
                for argv in make {
                    self.runner.run(&argv)?;
                }
                let fstab = std::fs::read_to_string(self.real("/etc/fstab")).unwrap_or_default();
                let sep = if fstab.is_empty() || fstab.ends_with('\n') {
                    ""
                } else {
                    "\n"
                };
                self.do_step(
                    id,
                    n * 100 + 1,
                    &Step::Write {
                        path: "/etc/fstab".into(),
                        content: format!("{fstab}{sep}{}\n", crate::swap::fstab_line(plan)),
                    },
                )?;
                self.push_undo(
                    id,
                    Undo::Command {
                        argv: v(&["swapoff", &plan.file]),
                    },
                );
                self.runner.run(&v(&["swapon", &plan.file]))?;
                self.log(id, format!("Swapped on {}", plan.file));
                let out = self.runner.run(&crate::swap::offset_argv(plan))?;
                let offset = crate::swap::parse_offset(&plan.fs, &out).ok_or_else(|| {
                    format!("could not read the resume offset from: {}", out.trim())
                })?;
                let uuid = self
                    .runner
                    .run(&v(&["findmnt", "-no", "UUID", "-T", &plan.file]))?
                    .trim()
                    .to_string();
                if uuid.is_empty() || uuid.contains(char::is_whitespace) {
                    return Err(format!(
                        "could not read the filesystem's UUID (got `{uuid}`)"
                    ));
                }
                let resume = format!("resume=UUID={uuid}");
                let offset = format!("resume_offset={offset}");
                self.log(id, format!("Resume from {resume} {offset}"));
                let steps = crate::fixes::boot_steps(&self.root, &[&resume, &offset], &[])?;
                for (k, st) in steps.iter().enumerate() {
                    self.do_step(id, n * 100 + 10 + k, st)?;
                }
            }
            Step::Command { argv, undo } => {
                self.push_undo(id, Undo::Command { argv: undo.clone() });
                let out = self.runner.run(argv)?;
                self.log(id, format!("Ran {}{}", argv.join(" "), trail(&out)));
            }
            Step::Backup => {
                let dir = self.dir.join(format!("backups/driver-{id}/config"));
                let n = crate::snapshot::backup_config(&self.root, &dir)
                    .map_err(|e| format!("backing up the configuration: {e}"))?;
                self.log(
                    id,
                    format!(
                        "Copied {n} configuration files and the package list to {}",
                        dir.display()
                    ),
                );
            }
            Step::Snapshot { tool } => {
                let title = self
                    .lock()
                    .iter()
                    .find(|r| r.id == id)
                    .map(|r| r.title.clone())
                    .unwrap_or_default();
                let argv = crate::snapshot::create_argv(tool, id, &title);
                if let crate::snapshot::Tool::Btrfs { .. } = tool {
                    let dir = self.real(crate::snapshot::BTRFS_DIR);
                    std::fs::create_dir_all(&dir)
                        .map_err(|e| format!("creating {}: {e}", dir.display()))?;
                }
                self.log(id, format!("Taking a snapshot: {}", argv.join(" ")));
                let out = self.runner.run(&argv)?;
                let name = crate::snapshot::created_name(tool, id, &out);
                let back = crate::snapshot::restore_commands(tool, &name);
                self.update(id, |r| r.restore = back);
                self.log(
                    id,
                    format!(
                        "Snapshot taken: {}",
                        if name.is_empty() { "(unnamed)" } else { &name }
                    ),
                );
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
                self.push_undo(
                    id,
                    Undo::Unload {
                        module: module.clone(),
                    },
                );
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
        let boot = boot_id(&self.root);
        self.update(id, |r| {
            r.state = "done".into();
            if r.needs_reboot {
                r.verify = "restart".into();
                r.done_boot = boot;
            }
        });
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
                None => all
                    .iter()
                    .filter(|r| matches!(r.state.as_str(), "done" | "interrupted"))
                    .max_by_key(|r| r.started_unix_ms),
            };
            let r = pick.ok_or_else(|| match id {
                Some(i) => format!("No driver change `{i}`."),
                None => "There is no driver change to undo.".to_string(),
            })?;
            if !matches!(r.state.as_str(), "done" | "interrupted") {
                return Err(format!(
                    "`{}` is {}, so there is nothing to undo.",
                    r.id, r.state
                ));
            }
            r.id.clone()
        };
        self.update(&id, |r| r.state = "undoing".into());
        let ok = self.undo_all(&id);
        self.update(&id, |r| {
            r.state = if ok {
                "undone".into()
            } else {
                "interrupted".into()
            };
            if ok {
                r.verify.clear();
            }
        });
        Ok(id)
    }

    /// The user checked a change after the restart and keeps it.
    pub fn keep(&self, id: &str) -> Result<(), String> {
        let mut found = false;
        self.update(id, |r| {
            if r.verify == "check" || r.verify == "restart" {
                r.verify = "kept".into();
                r.log.push("Kept.".into());
                found = true;
            }
        });
        if found {
            Ok(())
        } else {
            Err(format!("No change `{id}` is waiting to be checked."))
        }
    }

    /// Changes waiting for the check after their restart.
    pub fn waiting(&self) -> Vec<(String, String)> {
        self.lock()
            .iter()
            .filter(|r| r.verify == "check")
            .map(|r| (r.id.clone(), r.fix_id.clone()))
            .collect()
    }

    /// Undoes a change that is still waiting for its check, saying why in
    /// its log (no desktop came up after the restart).
    pub fn undo_unchecked(&self, id: &str, why: &str) -> Result<String, String> {
        if !self.waiting().iter().any(|(i, _)| i == id) {
            return Err(format!("`{id}` is not waiting for a check."));
        }
        self.log(id, why);
        self.rollback(Some(id))
    }

    /// Runs the undo list backwards, the initramfs last. True if all of it
    /// worked.
    fn undo_all(&self, id: &str) -> bool {
        let undo: Vec<Undo> = self
            .lock()
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.undo.clone())
            .unwrap_or_default();
        let mut ok = true;
        let mut rerun: Vec<Vec<String>> = Vec::new();
        for u in undo.iter().rev() {
            let r: Result<String, String> = match u {
                Undo::Restore {
                    path,
                    backup: Some(b),
                } => std::fs::read(b)
                    .and_then(|data| ug_core::paths::write_atomic(&self.real(path), &data))
                    .map(|_| format!("Put back {path}"))
                    .map_err(|e| format!("putting back {path}: {e}")),
                Undo::Restore { path, backup: None } => match std::fs::remove_file(self.real(path))
                {
                    Ok(()) => Ok(format!("Removed {path}")),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        Ok(format!("{path} was not there"))
                    }
                    Err(e) => Err(format!("removing {path}: {e}")),
                },
                Undo::Disable { units, now } => {
                    let flags: &[&str] = if *now {
                        &["systemctl", "disable", "--now"]
                    } else {
                        &["systemctl", "disable"]
                    };
                    let argv: Vec<String> = flags
                        .iter()
                        .map(|s| s.to_string())
                        .chain(units.iter().cloned())
                        .collect();
                    self.runner
                        .run(&argv)
                        .map(|_| format!("Disabled {}", units.join(", ")))
                }
                Undo::Switch { path, value } => std::fs::write(self.real(path), value)
                    .map(|_| format!("Set {path} back to {value}"))
                    .map_err(|e| format!("writing {path}: {e}")),
                Undo::Unload { module } => self
                    .runner
                    .run(&["modprobe".to_string(), "-r".to_string(), module.clone()])
                    .map(|_| format!("Unloaded {module}")),
                Undo::Remove { family, packages } => self
                    .runner
                    .run(&crate::packages::remove_argv(family, packages))
                    .map(|_| format!("Removed {}", packages.join(", "))),
                Undo::Initramfs { argv } | Undo::Rerun { argv } => {
                    rerun.insert(0, argv.clone());
                    continue;
                }
                Undo::Command { argv } => self
                    .runner
                    .run(argv)
                    .map(|_| format!("Ran {}", argv.join(" "))),
            };
            match r {
                Ok(m) => self.log(id, m),
                Err(e) => {
                    ok = false;
                    self.log(id, format!("Could not undo: {e}"));
                }
            }
        }
        // Generators run last, once every file is back, in their order.
        for argv in rerun {
            self.log(id, format!("Running {}…", argv.join(" ")));
            match self.runner.run(&argv) {
                Ok(out)
                    if argv
                        .iter()
                        .any(|a| a.contains("initramfs") || a == "mkinitcpio" || a == "dracut") =>
                {
                    self.log(id, format!("Initramfs rebuilt{}", trail(&out)))
                }
                Ok(out) => self.log(id, format!("Ran {}{}", argv.join(" "), trail(&out))),
                Err(e) => {
                    ok = false;
                    self.log(id, format!("Could not run {}: {e}", argv.join(" ")));
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
    let open: Vec<&Record> = records
        .iter()
        .filter(|r| !r.undo.is_empty() || (!r.restore.is_empty() && r.state != "undone"))
        .collect();
    if open.is_empty() {
        t.push_str("Nothing to undo.\n");
    }
    for r in open {
        t.push_str(&format!("# {} ({}, {})\n", r.title, r.id, r.state));
        let mut rerun: Vec<String> = Vec::new();
        for u in r.undo.iter().rev() {
            match u {
                Undo::Restore {
                    path,
                    backup: Some(b),
                } => t.push_str(&format!("sudo cp '{b}' '{path}'\n")),
                Undo::Restore { path, backup: None } => {
                    t.push_str(&format!("sudo rm -f '{path}'\n"))
                }
                Undo::Disable { units, now } => {
                    let flag = if *now { " --now" } else { "" };
                    t.push_str(&format!(
                        "sudo systemctl disable{flag} {}\n",
                        units.join(" ")
                    ))
                }
                Undo::Switch { path, value } => {
                    t.push_str(&format!("echo {value} | sudo tee '{path}'\n"))
                }
                Undo::Unload { module } => t.push_str(&format!("sudo modprobe -r {module}\n")),
                Undo::Remove { family, packages } => {
                    let argv = crate::packages::remove_argv(family, packages);
                    let argv: Vec<&str> = argv
                        .iter()
                        .map(String::as_str)
                        .filter(|a| *a != "env" && !a.starts_with("DEBIAN_FRONTEND"))
                        .collect();
                    t.push_str(&format!("sudo {}\n", argv.join(" ")))
                }
                Undo::Initramfs { argv } | Undo::Rerun { argv } => rerun.insert(0, argv.join(" ")),
                Undo::Command { argv } => t.push_str(&format!("sudo {}\n", argv.join(" "))),
            }
        }
        for i in rerun {
            t.push_str(&format!("sudo {i}\n"));
        }
        if !r.restore.is_empty() {
            t.push_str(
                "# If the system is still broken, go back to the snapshot taken before it:\n",
            );
            for c in &r.restore {
                t.push_str(c);
                t.push('\n');
            }
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
            .write(
                "etc/modprobe.d/nvidia-options.conf",
                "options nvidia NVreg_UsePageAttributeTable=1\n",
            )
            .write("usr/lib/systemd/system/nvidia-suspend.service", "")
            .write("usr/lib/systemd/system/nvidia-resume.service", "")
            .write("usr/lib/systemd/system/nvidia-hibernate.service", "")
            .write(
                "etc/systemd/system/systemd-hibernate.service.wants/nvidia-hibernate.service",
                "",
            );
        let state = tempfile::tempdir().unwrap();
        let rec = Arc::new(Recorder::default());
        let ex = Executor::new(
            SysRoot::at(fs.root()),
            state.path(),
            Box::new(Shared(Arc::clone(&rec))),
        );
        (fs, state, rec, ex)
    }

    fn snapshot(fs: &TestFs) -> Vec<(String, Vec<u8>)> {
        let mut v = Vec::new();
        fn walk(d: &Path, base: &Path, v: &mut Vec<(String, Vec<u8>)>) {
            for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                if e.path().is_dir() {
                    walk(&e.path(), base, v);
                } else {
                    v.push((
                        e.path().strip_prefix(base).unwrap().display().to_string(),
                        std::fs::read(e.path()).unwrap(),
                    ));
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
        assert!(
            std::fs::read_to_string(fs.root().join(FILE_POWER.trim_start_matches('/')))
                .unwrap()
                .contains("PreserveVideoMemoryAllocations=1")
        );
        let ran = rec.ran.lock().unwrap().clone();
        assert_eq!(
            ran[0],
            [
                "systemctl",
                "enable",
                "nvidia-suspend.service",
                "nvidia-resume.service"
            ],
            "only the units that were off"
        );
        assert_eq!(ran[1][0], "update-initramfs");
        let recovery = std::fs::read_to_string(state.path().join("driver-recovery.txt")).unwrap();
        assert!(
            recovery.contains(&format!("sudo rm -f '{FILE_POWER}'"))
                && recovery.contains(
                    "sudo systemctl disable nvidia-suspend.service nvidia-resume.service"
                )
        );

        // Undone after a "restart": a new executor reads the saved list.
        let ex = Executor::new(
            root.clone(),
            state.path(),
            Box::new(Shared(Arc::clone(&rec))),
        );
        ex.rollback(None).unwrap();
        assert_eq!(ex.ops()[0].state, "undone");
        assert_eq!(snapshot(&fs), before, "every file as it was");
        let ran = rec.ran.lock().unwrap().clone();
        assert_eq!(
            ran[2],
            [
                "systemctl",
                "disable",
                "nvidia-suspend.service",
                "nvidia-resume.service"
            ]
        );
        assert_eq!(
            ran[3][0], "update-initramfs",
            "the initramfs is rebuilt without our file"
        );
        assert!(ex.rollback(None).is_err(), "nothing left to undo");
        assert!(
            std::fs::read_to_string(state.path().join("driver-recovery.txt"))
                .unwrap()
                .contains("Nothing to undo")
        );
    }

    #[test]
    fn an_existing_file_is_backed_up_and_put_back() {
        let (fs, _state, _rec, ex) = machine();
        fs.write(
            "etc/modprobe.d/unix-guardian-nvidia-drm.conf",
            "options nvidia-drm modeset=0\n",
        );
        let root = SysRoot::at(fs.root());
        let before = snapshot(&fs);
        let fix = auto(
            "nvidia-drm-modeset",
            &root,
            &crate::inventory::facts(&root, &Default::default()),
        )
        .unwrap();
        let id = ex.start(&fix).unwrap();
        ex.run(&id, &fix);
        assert!(ex.ops()[0]
            .log
            .iter()
            .any(|l| l.contains("previous file is saved")));
        ex.rollback(Some(&id)).unwrap();
        assert_eq!(snapshot(&fs), before);
    }

    #[test]
    fn a_failing_step_undoes_what_was_done_and_nothing_runs_twice_at_once() {
        let (fs, _state, rec, ex) = machine();
        let root = SysRoot::at(fs.root());
        let before = snapshot(&fs);
        *rec.fail.lock().unwrap() = Some("update-initramfs".into());
        let fix = auto(
            "nouveau-blacklist",
            &root,
            &crate::inventory::facts(&root, &Default::default()),
        )
        .unwrap();
        let id = ex.start(&fix).unwrap();
        assert!(ex.start(&fix).unwrap_err().contains("still running"));
        ex.run(&id, &fix);
        let op = &ex.ops()[0];
        assert_eq!(op.state, "failed");
        assert!(op
            .log
            .iter()
            .any(|l| l.contains("Removed /etc/modprobe.d/unix-guardian-blacklist-nouveau.conf")));
        assert_eq!(snapshot(&fs), before, "the blacklist file is gone again");
        assert!(
            ex.rollback(Some(&id)).is_err(),
            "a failed change has nothing left to undo"
        );
    }

    #[test]
    fn an_operation_cut_off_mid_way_can_still_be_undone() {
        let (fs, state, rec, ex) = machine();
        let root = SysRoot::at(fs.root());
        let before = snapshot(&fs);
        let fix = auto(
            "nouveau-blacklist",
            &root,
            &crate::inventory::facts(&root, &Default::default()),
        )
        .unwrap();
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
        fs.write("sys/class/rfkill/rfkill0/type", "wlan\n")
            .write("sys/class/rfkill/rfkill0/soft", "1\n")
            .write("sys/class/rfkill/rfkill0/hard", "0\n");
        fs.write("usr/lib/systemd/system/cups.socket", "")
            .write("usr/lib/systemd/system/cups.service", "");
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
        assert_eq!(
            std::fs::read_to_string(fs.root().join("sys/class/rfkill/rfkill0/soft")).unwrap(),
            "0"
        );
        let ran = rec.ran.lock().unwrap().clone();
        assert_eq!(
            ran[0],
            [
                "systemctl",
                "enable",
                "--now",
                "cups.socket",
                "cups.service"
            ]
        );
        assert_eq!(ran[1], ["modprobe", "uvcvideo"]);
        let recovery = std::fs::read_to_string(state.path().join("driver-recovery.txt")).unwrap();
        assert!(
            recovery.contains("echo 1 | sudo tee '/sys/class/rfkill/rfkill0/soft'")
                && recovery.contains("sudo modprobe -r uvcvideo")
        );
        for _ in 0..3 {
            ex.rollback(None).unwrap();
        }
        let ran = rec.ran.lock().unwrap().clone();
        assert_eq!(ran[2], ["modprobe", "-r", "uvcvideo"]);
        assert_eq!(
            ran[3],
            [
                "systemctl",
                "disable",
                "--now",
                "cups.socket",
                "cups.service"
            ]
        );
        assert_eq!(
            std::fs::read_to_string(fs.root().join("sys/class/rfkill/rfkill0/soft")).unwrap(),
            "1"
        );
        assert_eq!(
            snapshot(&fs),
            before,
            "the modules-load.d file is gone again"
        );
        assert!(ex.rollback(None).is_err());
    }

    /// A package manager that really adds to the dpkg database, so the
    /// executor can see what an install added.
    struct FakeApt {
        status: PathBuf,
        ran: Arc<Mutex<Vec<Vec<String>>>>,
    }
    impl Runner for FakeApt {
        fn run(&self, argv: &[String]) -> Result<String, String> {
            self.ran.lock().unwrap().push(argv.to_vec());
            if argv.iter().any(|a| a == "install") {
                let mut t = std::fs::read_to_string(&self.status).unwrap_or_default();
                for p in argv
                    .iter()
                    .skip(5)
                    .chain(["libbluetooth3".to_string()].iter())
                {
                    t.push_str(&format!("\nPackage: {p}\nStatus: install ok installed\n"));
                }
                std::fs::write(&self.status, t).unwrap();
            }
            if argv[0] == "timeshift" {
                return Ok("Tagged snapshot '2026-10-03_18-00-01': ondemand".into());
            }
            Ok(String::new())
        }
    }

    #[test]
    fn a_package_is_installed_only_under_a_snapshot_and_undo_removes_what_it_added() {
        let fs = TestFs::new();
        fs.write("etc/os-release", "ID=debian\nPRETTY_NAME=Debian\n")
            .write(
                "var/lib/dpkg/status",
                "Package: bash\nStatus: install ok installed\n",
            )
            .write("etc/modprobe.d/x.conf", "options x y=1\n");
        let root = SysRoot::at(fs.root());
        let facts = crate::inventory::facts(&root, &Default::default());
        let refused = auto("install-bluez", &root, &facts).unwrap_err();
        assert!(
            refused.contains("only when it can take a snapshot")
                && refused.contains("apt install timeshift"),
            "{refused}"
        );
        assert!(!crate::fixes::can_run("install-bluez", &facts));

        fs.write("usr/bin/timeshift", "")
            .write("etc/timeshift/timeshift.json", "{}");
        let facts = crate::inventory::facts(&root, &Default::default());
        assert!(crate::fixes::can_run("install-bluez", &facts));
        let fix = auto("install-bluez", &root, &facts).unwrap();
        let kinds: Vec<String> = fix.steps.iter().map(|s| s.describe().kind).collect();
        assert_eq!(
            kinds,
            ["backup", "snapshot", "install", "enable-units"],
            "safety net first"
        );

        let state = tempfile::tempdir().unwrap();
        let ran = Arc::new(Mutex::new(Vec::new()));
        let runner = FakeApt {
            status: fs.root().join("var/lib/dpkg/status"),
            ran: Arc::clone(&ran),
        };
        let ex = Executor::new(root.clone(), state.path(), Box::new(runner));
        let id = ex.start(&fix).unwrap();
        ex.run(&id, &fix);
        assert_eq!(ex.ops()[0].state, "done", "{:?}", ex.ops()[0].log);
        let r = ran.lock().unwrap().clone();
        assert_eq!(r[0][..2], ["timeshift".to_string(), "--create".to_string()]);
        assert_eq!(r[1], ["apt-get", "update"]);
        assert_eq!(
            r[2],
            [
                "env",
                "DEBIAN_FRONTEND=noninteractive",
                "apt-get",
                "install",
                "-y",
                "bluez"
            ]
        );
        assert!(state
            .path()
            .join(format!("backups/driver-{id}/config/etc/modprobe.d/x.conf"))
            .exists());
        assert!(std::fs::read_to_string(
            state
                .path()
                .join(format!("backups/driver-{id}/config/packages.txt"))
        )
        .unwrap()
        .contains("bash"));
        let recovery = std::fs::read_to_string(state.path().join("driver-recovery.txt")).unwrap();
        assert!(
            recovery.contains("sudo apt-get remove -y bluez libbluetooth3"),
            "{recovery}"
        );
        assert!(
            recovery.contains("sudo timeshift --restore --snapshot '2026-10-03_18-00-01'"),
            "{recovery}"
        );

        ex.rollback(None).unwrap();
        let r = ran.lock().unwrap().clone();
        assert_eq!(
            r.last().unwrap(),
            &[
                "env",
                "DEBIAN_FRONTEND=noninteractive",
                "apt-get",
                "remove",
                "-y",
                "bluez",
                "libbluetooth3"
            ],
            "what it added, bash stays"
        );
    }

    #[test]
    fn a_boot_parameter_is_added_through_grub_and_undone_to_the_byte() {
        let (fs, state, rec, ex) = machine();
        let grub = "GRUB_DEFAULT=0\nGRUB_CMDLINE_LINUX_DEFAULT=\"quiet splash\"\n";
        fs.write("etc/default/grub", grub)
            .write("usr/sbin/update-grub", "");
        let root = SysRoot::at(fs.root());
        let facts = crate::inventory::facts(&root, &Default::default());
        assert!(crate::fixes::can_run("mem-sleep-deep", &facts));
        let before = snapshot(&fs);
        let fix = auto("mem-sleep-deep", &root, &facts).unwrap();
        let id = ex.start(&fix).unwrap();
        ex.run(&id, &fix);
        assert_eq!(ex.ops()[0].state, "done", "{:?}", ex.ops()[0].log);
        assert!(std::fs::read_to_string(fs.root().join("etc/default/grub"))
            .unwrap()
            .contains("\"quiet splash mem_sleep_default=deep\""));
        assert_eq!(rec.ran.lock().unwrap().clone(), [["update-grub"]]);
        let again = crate::inventory::facts(&root, &Default::default());
        assert!(
            auto("mem-sleep-deep", &root, &again)
                .unwrap_err()
                .contains("already that way"),
            "nothing to add twice"
        );
        let recovery = std::fs::read_to_string(state.path().join("driver-recovery.txt")).unwrap();
        assert!(
            recovery.contains("sudo cp ") && recovery.trim_end().ends_with("sudo update-grub"),
            "{recovery}"
        );
        ex.rollback(None).unwrap();
        assert_eq!(snapshot(&fs), before, "the file is back byte for byte");
        assert_eq!(
            rec.ran.lock().unwrap().len(),
            2,
            "the menu is written again without the parameter"
        );
    }

    #[test]
    fn a_change_that_needs_a_restart_is_checked_after_it() {
        let (fs, state, rec, ex) = machine();
        fs.write("proc/sys/kernel/random/boot_id", "boot-1\n");
        let root = SysRoot::at(fs.root());
        let facts = crate::inventory::facts(&root, &Default::default());
        let fix = auto("nouveau-blacklist", &root, &facts).unwrap();
        let id = ex.start(&fix).unwrap();
        ex.run(&id, &fix);
        assert_eq!(ex.ops()[0].verify, "restart");
        // The service restarting in the same boot asks nothing yet.
        let ex = Executor::new(
            root.clone(),
            state.path(),
            Box::new(Shared(Arc::clone(&rec))),
        );
        assert_eq!(ex.ops()[0].verify, "restart");
        assert!(ex.waiting().is_empty());
        fs.write("proc/sys/kernel/random/boot_id", "boot-2\n");
        let ex = Executor::new(
            root.clone(),
            state.path(),
            Box::new(Shared(Arc::clone(&rec))),
        );
        assert_eq!(ex.ops()[0].verify, "check");
        assert_eq!(
            ex.waiting(),
            [(id.clone(), "nouveau-blacklist".to_string())]
        );
        ex.keep(&id).unwrap();
        assert_eq!(ex.ops()[0].verify, "kept");
        assert!(ex.keep(&id).is_err(), "only once");
        assert!(
            ex.undo_unchecked(&id, "x").is_err(),
            "a kept change is not undone by itself"
        );

        // Another change, after which no desktop came up.
        let fix = auto("nvidia-drm-modeset", &root, &facts).unwrap();
        let id2 = ex.start(&fix).unwrap();
        ex.run(&id2, &fix);
        fs.write("proc/sys/kernel/random/boot_id", "boot-3\n");
        let ex = Executor::new(
            root.clone(),
            state.path(),
            Box::new(Shared(Arc::clone(&rec))),
        );
        assert!(!desktop_session(&root));
        ex.undo_unchecked(
            &id2,
            "No desktop came up after the restart; undone automatically.",
        )
        .unwrap();
        let op = ex.ops().into_iter().find(|o| o.id == id2).unwrap();
        assert_eq!((op.state.as_str(), op.verify.as_str()), ("undone", ""));
        assert!(op.log.iter().any(|l| l.contains("No desktop came up")));
        fs.write(
            "run/systemd/sessions/2",
            "UID=1000\nTYPE=wayland\nACTIVE=1\n",
        );
        assert!(desktop_session(&root));
    }

    /// Plays the swap tools: makes the file, answers the offset and UUID.
    struct FakeSwap {
        root: PathBuf,
        ran: Arc<Mutex<Vec<Vec<String>>>>,
    }
    impl Runner for FakeSwap {
        fn run(&self, argv: &[String]) -> Result<String, String> {
            self.ran.lock().unwrap().push(argv.to_vec());
            Ok(
                match argv
                    .iter()
                    .take(2)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .as_str()
                {
                    "btrfs filesystem" => {
                        std::fs::write(self.root.join(argv[5].trim_start_matches('/')), "swap")
                            .unwrap();
                        String::new()
                    }
                    "btrfs inspect-internal" => "533760\n".into(),
                    "findmnt -no" => "0f0e0d0c-0b0a-4908-8706-050403020100\n".into(),
                    _ => String::new(),
                },
            )
        }
    }

    #[test]
    fn hibernation_swap_is_made_and_undone_completely() {
        let fs = TestFs::new();
        let grub = "GRUB_CMDLINE_LINUX_DEFAULT=\"quiet splash\"\n";
        // TestFs adds the final newline.
        let fstab =
            "UUID=x /swap btrfs subvol=/@swap,defaults 0 0\n/swap/swapfile swap swap defaults 0 0";
        fs.write("etc/os-release", "ID=parrot\nID_LIKE=debian\nPRETTY_NAME=Parrot\n")
            .write("usr/sbin/update-initramfs", "")
            .write("usr/sbin/update-grub", "")
            .write("etc/default/grub", grub)
            .write("etc/fstab", fstab)
            .write("proc/meminfo", "MemTotal:       28440104 kB\n")
            .write("proc/mounts", "/dev/nvme0n1p2 / btrfs rw,subvol=/@ 0 0\n/dev/nvme0n1p2 /swap btrfs rw,subvol=/@swap 0 0\n")
            .write("sys/kernel/security/lockdown", "[none] integrity confidentiality\n")
            .dir("swap");
        let root = SysRoot::at(fs.root());
        let facts = crate::inventory::facts(&root, &Default::default());
        assert!(crate::fixes::can_run("hibernate-swap", &facts));
        let fix = crate::fixes::hibernate_fix_with(&root, &facts, |_| Some(40 << 30)).unwrap();
        let Step::HibernateSwap { plan } = &fix.steps[1] else {
            panic!("{:?}", fix.steps)
        };
        assert_eq!(
            (plan.file.as_str(), plan.size_gib),
            ("/swap/hibernate.swap", 28)
        );
        let before = snapshot(&fs);
        let state = tempfile::tempdir().unwrap();
        let ran = Arc::new(Mutex::new(Vec::new()));
        let ex = Executor::new(
            root.clone(),
            state.path(),
            Box::new(FakeSwap {
                root: fs.root().to_path_buf(),
                ran: Arc::clone(&ran),
            }),
        );
        let id = ex.start(&fix).unwrap();
        ex.run(&id, &fix);
        assert_eq!(ex.ops()[0].state, "done", "{:?}", ex.ops()[0].log);
        assert!(fs.root().join("swap/hibernate.swap").exists());
        let new_fstab = std::fs::read_to_string(fs.root().join("etc/fstab")).unwrap();
        assert_eq!(
            new_fstab,
            format!("{fstab}\n/swap/hibernate.swap none swap defaults,pri=-10 0 0\n")
        );
        let g = std::fs::read_to_string(fs.root().join("etc/default/grub")).unwrap();
        assert!(g.contains("quiet splash resume=UUID=0f0e0d0c-0b0a-4908-8706-050403020100 resume_offset=533760"), "{g}");
        let r = ran.lock().unwrap().clone();
        let firsts: Vec<String> = r
            .iter()
            .map(|a| a.iter().take(2).cloned().collect::<Vec<_>>().join(" "))
            .collect();
        assert_eq!(
            firsts,
            [
                "btrfs filesystem",
                "swapon /swap/hibernate.swap",
                "btrfs inspect-internal",
                "findmnt -no",
                "update-grub",
                "update-initramfs -u"
            ]
        );

        ex.rollback(None).unwrap();
        let r = ran.lock().unwrap().clone();
        let undo: Vec<String> = r[6..].iter().map(|a| a.join(" ")).collect();
        assert_eq!(
            undo,
            [
                "swapoff /swap/hibernate.swap",
                "update-grub",
                "update-initramfs -u -k all"
            ],
            "swap off before the file goes"
        );
        assert_eq!(snapshot(&fs), before, "fstab and grub back to the byte");
        assert!(!fs.root().join("swap/hibernate.swap").exists());
    }
}
