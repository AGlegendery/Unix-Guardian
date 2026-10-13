//! The queue of scheduled power actions ("restart in two hours", "shut down
//! every night at 1:00").
//!
//! Kept on disk (`/var/lib/unix-guardian/schedule.json`, written atomically)
//! so a daemon restart or a crash does not silently forget a shutdown the user
//! is counting on. The rules after a restart are deliberately conservative: a
//! one-off action whose time passed while the daemon was down is dropped and
//! logged, never run late — shutting a machine down an hour after the user
//! expected, possibly in the middle of something else, is worse than not at
//! all. A repeating action moves on to its next future time.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// What logind can be asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    PowerOff,
    Reboot,
    Suspend,
    Hibernate,
    HybridSleep,
    SuspendThenHibernate,
    /// Restart straight into the firmware (BIOS/UEFI) setup screen.
    RebootFirmware,
}

impl Action {
    pub const ALL: [Action; 7] = [
        Action::PowerOff,
        Action::Reboot,
        Action::Suspend,
        Action::Hibernate,
        Action::HybridSleep,
        Action::SuspendThenHibernate,
        Action::RebootFirmware,
    ];

    /// The wire and CLI name, e.g. `power-off`.
    pub fn name(self) -> &'static str {
        match self {
            Action::PowerOff => "power-off",
            Action::Reboot => "reboot",
            Action::Suspend => "suspend",
            Action::Hibernate => "hibernate",
            Action::HybridSleep => "hybrid-sleep",
            Action::SuspendThenHibernate => "suspend-then-hibernate",
            Action::RebootFirmware => "reboot-firmware",
        }
    }

    pub fn parse(s: &str) -> Result<Action, String> {
        Action::ALL
            .into_iter()
            .find(|a| a.name() == s)
            .ok_or_else(|| {
                let names: Vec<&str> = Action::ALL.iter().map(|a| a.name()).collect();
                format!(
                    "unknown power action {s:?}; actions are: {}",
                    names.join(", ")
                )
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    Once,
    Daily,
    Weekly,
}

impl Repeat {
    pub fn name(self) -> &'static str {
        match self {
            Repeat::Once => "once",
            Repeat::Daily => "daily",
            Repeat::Weekly => "weekly",
        }
    }

    pub fn parse(s: &str) -> Result<Repeat, String> {
        match s {
            "" | "once" => Ok(Repeat::Once),
            "daily" => Ok(Repeat::Daily),
            "weekly" => Ok(Repeat::Weekly),
            _ => Err(format!("unknown repeat {s:?}; use once, daily or weekly")),
        }
    }

    /// Fixed steps: Iran, where most users are, has had no daylight saving
    /// since 2022; elsewhere a repeating job drifts by an hour twice a year.
    fn step_ms(self) -> Option<u64> {
        match self {
            Repeat::Once => None,
            Repeat::Daily => Some(DAY_MS),
            Repeat::Weekly => Some(7 * DAY_MS),
        }
    }
}

const DAY_MS: u64 = 24 * 3_600_000;
/// Further ahead than this is almost certainly a mistake in the input.
const MAX_AHEAD_MS: u64 = 366 * DAY_MS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: u32,
    pub action: Action,
    pub at_ms: u64,
    pub repeat: Repeat,
    #[serde(default)]
    pub note: String,
}

#[derive(Default, Serialize, Deserialize)]
struct File {
    next_id: u32,
    jobs: Vec<Job>,
}

pub struct Schedule {
    path: PathBuf,
    next_id: u32,
    jobs: Vec<Job>,
}

impl Schedule {
    /// Loads the queue, dropping one-off jobs that are already past and
    /// moving repeating ones to their next time. Returns what it dropped, as
    /// sentences for the log. An unreadable file starts an empty queue and is
    /// reported, never fatal.
    pub fn load(path: &Path, now_ms: u64) -> (Schedule, Vec<String>) {
        let mut notes = Vec::new();
        let file = match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str::<File>(&s).unwrap_or_else(|e| {
                notes.push(format!(
                    "{} could not be read ({e}); starting with an empty schedule",
                    path.display()
                ));
                File::default()
            }),
            Err(_) => File::default(),
        };
        let mut s = Schedule {
            path: path.to_path_buf(),
            next_id: file.next_id.max(1),
            jobs: Vec::new(),
        };
        for mut job in file.jobs {
            if job.at_ms > now_ms {
                s.jobs.push(job);
                continue;
            }
            match job.repeat.step_ms() {
                None => notes.push(format!(
                    "missed while the service was not running, not run late: {} (job {})",
                    job.action.name(),
                    job.id
                )),
                Some(step) => {
                    let behind = (now_ms - job.at_ms) / step + 1;
                    job.at_ms += behind * step;
                    s.jobs.push(job);
                }
            }
        }
        s.sort();
        if !notes.is_empty() {
            let _ = s.save();
        }
        (s, notes)
    }

    pub fn jobs(&self) -> &[Job] {
        &self.jobs
    }

    /// Adds a job for a time in the future.
    pub fn add(
        &mut self,
        action: Action,
        at_ms: u64,
        repeat: Repeat,
        note: &str,
        now_ms: u64,
    ) -> Result<Job, String> {
        if at_ms <= now_ms {
            return Err("that time has already passed".into());
        }
        if at_ms - now_ms > MAX_AHEAD_MS {
            return Err("that is more than a year away".into());
        }
        let job = Job {
            id: self.next_id,
            action,
            at_ms,
            repeat,
            note: note.chars().take(200).collect(),
        };
        self.next_id += 1;
        self.jobs.push(job.clone());
        self.sort();
        if let Err(e) = self.save() {
            // Told it failed, so it must not run either.
            self.jobs.retain(|j| j.id != job.id);
            return Err(e);
        }
        Ok(job)
    }

    /// Removes a job; false if there was none with that id.
    pub fn cancel(&mut self, id: u32) -> Result<bool, String> {
        let before = self.jobs.len();
        self.jobs.retain(|j| j.id != id);
        if self.jobs.len() == before {
            return Ok(false);
        }
        self.save()?;
        Ok(true)
    }

    /// Takes the jobs due at `now_ms`: one-off jobs leave the queue, repeating
    /// ones move to their next time. A job more than `grace_ms` late — the
    /// machine was asleep when it was due — comes back in the second list and
    /// must not be run: powering off hours after the user expected is worse
    /// than not at all.
    pub fn take_due(&mut self, now_ms: u64, grace_ms: u64) -> (Vec<Job>, Vec<Job>) {
        let (mut due, mut missed) = (Vec::new(), Vec::new());
        for job in &mut self.jobs {
            if job.at_ms <= now_ms {
                if now_ms - job.at_ms > grace_ms {
                    missed.push(job.clone());
                } else {
                    due.push(job.clone());
                }
                if let Some(step) = job.repeat.step_ms() {
                    while job.at_ms <= now_ms {
                        job.at_ms += step;
                    }
                }
            }
        }
        if due.is_empty() && missed.is_empty() {
            return (due, missed);
        }
        self.jobs.retain(|j| j.at_ms > now_ms);
        self.sort();
        if let Err(e) = self.save() {
            tracing::warn!(error = %e, "could not save the schedule");
        }
        (due, missed)
    }

    fn sort(&mut self) {
        self.jobs.sort_by_key(|j| (j.at_ms, j.id));
    }

    fn save(&self) -> Result<(), String> {
        let file = File {
            next_id: self.next_id,
            jobs: self.jobs.clone(),
        };
        let text = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, text)
            .and_then(|()| std::fs::rename(&tmp, &self.path))
            .map_err(|e| format!("{}: {e}", self.path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: u64 = 3_600_000;

    fn path(d: &tempfile::TempDir) -> PathBuf {
        d.path().join("state/schedule.json")
    }

    #[test]
    fn a_job_that_could_not_be_saved_never_runs() {
        let d = tempfile::tempdir().unwrap();
        let p = path(&d);
        std::fs::create_dir_all(&p).unwrap(); // a directory: every save fails
        let (mut s, _) = Schedule::load(&p, 0);
        assert!(s.add(Action::PowerOff, H, Repeat::Once, "", 0).is_err());
        assert!(s.jobs().is_empty());
        assert!(s.take_due(H, H).0.is_empty(), "nothing powers off");
    }

    #[test]
    fn a_scheduled_restart_survives_a_daemon_restart() {
        let d = tempfile::tempdir().unwrap();
        let (mut s, _) = Schedule::load(&path(&d), 0);
        let job = s
            .add(Action::Reboot, 2 * H, Repeat::Once, "updates", 0)
            .unwrap();
        drop(s);
        let (s, notes) = Schedule::load(&path(&d), H);
        assert!(notes.is_empty());
        assert_eq!(s.jobs(), &[job]);
    }

    #[test]
    fn a_one_off_job_missed_while_down_is_dropped_not_run_late() {
        let d = tempfile::tempdir().unwrap();
        let (mut s, _) = Schedule::load(&path(&d), 0);
        s.add(Action::PowerOff, H, Repeat::Once, "", 0).unwrap();
        drop(s);
        let (s, notes) = Schedule::load(&path(&d), 3 * H);
        assert!(s.jobs().is_empty());
        assert!(notes[0].contains("not run late"), "{notes:?}");
        assert!(
            Schedule::load(&path(&d), 3 * H).0.jobs().is_empty(),
            "the drop was saved"
        );
    }

    #[test]
    fn a_repeating_job_missed_while_down_moves_to_its_next_time() {
        let d = tempfile::tempdir().unwrap();
        let (mut s, _) = Schedule::load(&path(&d), 0);
        s.add(Action::PowerOff, H, Repeat::Daily, "", 0).unwrap();
        drop(s);
        let (s, _) = Schedule::load(&path(&d), 50 * H);
        assert_eq!(
            s.jobs()[0].at_ms,
            H + 3 * DAY_MS,
            "the first 1:00 after hour 50"
        );
    }

    #[test]
    fn due_jobs_are_taken_once_and_repeats_roll_forward() {
        let d = tempfile::tempdir().unwrap();
        let (mut s, _) = Schedule::load(&path(&d), 0);
        s.add(Action::Suspend, H, Repeat::Once, "", 0).unwrap();
        s.add(Action::PowerOff, 2 * H, Repeat::Weekly, "", 0)
            .unwrap();
        assert_eq!(s.jobs()[0].at_ms, H, "soonest first");
        assert!(s.take_due(H - 1, 120_000).0.is_empty());
        let (due, missed) = s.take_due(2 * H, 2 * H);
        assert!(missed.is_empty());
        assert_eq!(
            due.iter().map(|j| j.action).collect::<Vec<_>>(),
            [Action::Suspend, Action::PowerOff]
        );
        assert_eq!(s.jobs().len(), 1);
        assert_eq!(s.jobs()[0].at_ms, 2 * H + 7 * DAY_MS);
        assert!(s.take_due(2 * H, 2 * H).0.is_empty(), "not taken twice");
    }

    #[test]
    fn a_job_that_fell_due_while_the_machine_slept_is_not_run_on_wake() {
        let d = tempfile::tempdir().unwrap();
        let (mut s, _) = Schedule::load(&path(&d), 0);
        s.add(Action::PowerOff, H, Repeat::Once, "", 0).unwrap();
        s.add(Action::Reboot, H, Repeat::Daily, "", 0).unwrap();
        // Suspended just before 1:00, resumed at 8:00.
        let (due, missed) = s.take_due(8 * H, 120_000);
        assert!(due.is_empty(), "nothing runs late");
        assert_eq!(missed.len(), 2);
        assert_eq!(s.jobs().len(), 1, "the one-off is gone");
        assert_eq!(
            s.jobs()[0].at_ms,
            H + DAY_MS,
            "the daily one waits for tomorrow"
        );
    }

    #[test]
    fn past_and_absurd_times_are_refused() {
        let d = tempfile::tempdir().unwrap();
        let (mut s, _) = Schedule::load(&path(&d), 10 * H);
        assert!(s
            .add(Action::Reboot, 5 * H, Repeat::Once, "", 10 * H)
            .is_err());
        assert!(s
            .add(
                Action::Reboot,
                10 * H + 400 * DAY_MS,
                Repeat::Once,
                "",
                10 * H
            )
            .is_err());
        assert!(s.jobs().is_empty());
    }

    #[test]
    fn cancelling_removes_exactly_that_job_and_ids_are_never_reused() {
        let d = tempfile::tempdir().unwrap();
        let (mut s, _) = Schedule::load(&path(&d), 0);
        let a = s.add(Action::Reboot, H, Repeat::Once, "", 0).unwrap();
        let b = s.add(Action::PowerOff, 2 * H, Repeat::Once, "", 0).unwrap();
        assert!(s.cancel(a.id).unwrap());
        assert!(!s.cancel(a.id).unwrap());
        drop(s);
        let (mut s, _) = Schedule::load(&path(&d), 0);
        assert_eq!(s.jobs(), &[b.clone()]);
        let c = s.add(Action::Reboot, H, Repeat::Once, "", 0).unwrap();
        assert!(c.id > b.id);
    }

    #[test]
    fn a_corrupt_file_starts_an_empty_schedule_and_says_so() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(path(&d).parent().unwrap()).unwrap();
        std::fs::write(path(&d), "{ not json").unwrap();
        let (s, notes) = Schedule::load(&path(&d), 0);
        assert!(s.jobs().is_empty());
        assert!(notes[0].contains("could not be read"));
    }

    #[test]
    fn action_names_round_trip_and_unknown_ones_list_the_choices() {
        for a in Action::ALL {
            assert_eq!(Action::parse(a.name()).unwrap(), a);
        }
        assert!(Action::parse("explode").unwrap_err().contains("power-off"));
    }
}
