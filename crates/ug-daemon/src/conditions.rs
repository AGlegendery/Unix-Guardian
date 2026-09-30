//! Conditional power actions: "shut down when the download has finished",
//! "when this game ends", "when the CPU has been idle for 10 minutes", "when
//! this render finishes".
//!
//! Every condition must hold *continuously* for its window before it fires,
//! so a dip in download speed or a quiet moment between two builds does not
//! shut the machine down in the middle of work. Watches are kept on disk; after
//! a daemon restart their windows start again from zero, which errs towards
//! waiting longer rather than firing early.

use crate::schedule::Action;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Condition {
    /// Download finished: receive rate below `below_bps` for `minutes`.
    NetIdle { below_bps: u64, minutes: u32 },
    /// Nothing heavy running: CPU load below `below_pct` for `minutes`.
    CpuIdle { below_pct: f64, minutes: u32 },
    /// A named program (by process name, e.g. `blender`) has exited.
    ProcessGone { name: String },
    /// The game that is running now has ended (game mode went off).
    GameOver,
}

impl Condition {
    /// How long the condition must hold before firing. Process and game
    /// endings get a short confirmation so a relaunch or a launcher handing
    /// over between processes does not count as the end.
    fn window_ms(&self) -> u64 {
        match self {
            Condition::NetIdle { minutes, .. } | Condition::CpuIdle { minutes, .. } => *minutes as u64 * 60_000,
            Condition::ProcessGone { .. } | Condition::GameOver => 20_000,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Condition::NetIdle { minutes, .. } | Condition::CpuIdle { minutes, .. } if !(1..=24 * 60).contains(minutes) => {
                Err("the waiting time must be between 1 minute and 24 hours".into())
            }
            Condition::CpuIdle { below_pct, .. } if !(1.0..=100.0).contains(below_pct) => {
                Err("the CPU threshold must be between 1 and 100 %".into())
            }
            Condition::ProcessGone { name } if name.trim().is_empty() || name.contains('/') => {
                Err("give a process name, e.g. blender".into())
            }
            _ => Ok(()),
        }
    }
}

/// One reading of the things conditions look at.
#[derive(Debug, Clone, Default)]
pub struct Observation {
    pub rx_bps: u64,
    pub cpu_pct: Option<f64>,
    pub game_active: bool,
    /// Process names currently running.
    pub processes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Watch {
    pub id: u32,
    pub action: Action,
    pub condition: Condition,
    #[serde(default)]
    pub note: String,
    /// When the condition started holding without a break; not persisted.
    #[serde(skip)]
    holding_since: Option<u64>,
}

impl Watch {
    /// Updates the watch with a reading; true when it should fire now.
    fn observe(&mut self, o: &Observation, now_ms: u64) -> bool {
        let holds = match &self.condition {
            Condition::NetIdle { below_bps, .. } => o.rx_bps < *below_bps,
            // No reading is not "idle": never fire on missing data.
            Condition::CpuIdle { below_pct, .. } => o.cpu_pct.is_some_and(|c| c < *below_pct),
            Condition::ProcessGone { name } => !o.processes.iter().any(|p| p == name),
            Condition::GameOver => !o.game_active,
        };
        if !holds {
            self.holding_since = None;
            return false;
        }
        let since = *self.holding_since.get_or_insert(now_ms);
        now_ms - since >= self.condition.window_ms()
    }
}

#[derive(Default, Serialize, Deserialize)]
struct File {
    next_id: u32,
    watches: Vec<Watch>,
}

pub struct Watches {
    path: PathBuf,
    next_id: u32,
    watches: Vec<Watch>,
}

impl Watches {
    pub fn load(path: &Path) -> Watches {
        let f: File = std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        Watches { path: path.to_path_buf(), next_id: f.next_id.max(1), watches: f.watches }
    }

    pub fn list(&self) -> &[Watch] {
        &self.watches
    }

    pub fn is_empty(&self) -> bool {
        self.watches.is_empty()
    }

    /// Adds a watch. `now` is the current state, so a condition that cannot
    /// start meaningfully (no game to wait for, the program is not running)
    /// is refused instead of firing at once.
    pub fn add(&mut self, action: Action, condition: Condition, note: &str, now: &Observation) -> Result<Watch, String> {
        condition.validate()?;
        match &condition {
            Condition::GameOver if !now.game_active => return Err("no game is running right now".into()),
            Condition::ProcessGone { name } if !now.processes.iter().any(|p| p == name) => {
                return Err(format!("no process called {name:?} is running right now"));
            }
            _ => {}
        }
        let w = Watch { id: self.next_id, action, condition, note: note.chars().take(200).collect(), holding_since: None };
        self.next_id += 1;
        self.watches.push(w.clone());
        self.save()?;
        Ok(w)
    }

    pub fn cancel(&mut self, id: u32) -> Result<bool, String> {
        let before = self.watches.len();
        self.watches.retain(|w| w.id != id);
        if self.watches.len() == before {
            return Ok(false);
        }
        self.save()?;
        Ok(true)
    }

    /// Feeds a reading to every watch; the ones that fire leave the list.
    pub fn observe(&mut self, o: &Observation, now_ms: u64) -> Vec<Watch> {
        let mut fired = Vec::new();
        self.watches.retain_mut(|w| {
            if w.observe(o, now_ms) {
                fired.push(w.clone());
                false
            } else {
                true
            }
        });
        if !fired.is_empty() {
            if let Err(e) = self.save() {
                tracing::warn!(error = %e, "could not save conditional power actions");
            }
        }
        fired
    }

    fn save(&self) -> Result<(), String> {
        let text = serde_json::to_string_pretty(&File { next_id: self.next_id, watches: self.watches.clone() })
            .map_err(|e| e.to_string())?;
        if let Some(d) = self.path.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
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

    const MIN: u64 = 60_000;

    fn watches() -> (tempfile::TempDir, Watches) {
        let d = tempfile::tempdir().unwrap();
        let w = Watches::load(&d.path().join("conditions.json"));
        (d, w)
    }

    fn obs(rx: u64, cpu: f64) -> Observation {
        Observation { rx_bps: rx, cpu_pct: Some(cpu), ..Default::default() }
    }

    #[test]
    fn a_download_counts_as_finished_only_after_the_whole_quiet_window() {
        let (_d, mut w) = watches();
        w.add(Action::PowerOff, Condition::NetIdle { below_bps: 50_000, minutes: 5 }, "", &obs(4_000_000, 10.0)).unwrap();
        assert!(w.observe(&obs(4_000_000, 10.0), 0).is_empty(), "still downloading");
        assert!(w.observe(&obs(1_000, 10.0), MIN).is_empty(), "quiet starts");
        assert!(w.observe(&obs(3_000_000, 10.0), 3 * MIN).is_empty(), "a burst resets the window");
        assert!(w.observe(&obs(1_000, 10.0), 4 * MIN).is_empty());
        assert!(w.observe(&obs(1_000, 10.0), 8 * MIN).is_empty(), "four quiet minutes are not five");
        let fired = w.observe(&obs(1_000, 10.0), 9 * MIN);
        assert_eq!(fired.len(), 1);
        assert!(w.is_empty());
    }

    #[test]
    fn a_missing_cpu_reading_never_counts_as_idle() {
        let (_d, mut w) = watches();
        w.add(Action::Suspend, Condition::CpuIdle { below_pct: 10.0, minutes: 1 }, "", &obs(0, 50.0)).unwrap();
        let blind = Observation::default();
        assert!(w.observe(&blind, 0).is_empty());
        assert!(w.observe(&blind, 10 * MIN).is_empty());
    }

    #[test]
    fn a_program_ending_fires_after_a_short_confirmation() {
        let (_d, mut w) = watches();
        let running = Observation { processes: vec!["blender".into()], ..Default::default() };
        w.add(Action::PowerOff, Condition::ProcessGone { name: "blender".into() }, "", &running).unwrap();
        assert!(w.observe(&running, 0).is_empty());
        let gone = Observation::default();
        assert!(w.observe(&gone, 10_000).is_empty());
        assert!(w.observe(&running, 20_000).is_empty(), "came back (a relaunch): start again");
        assert!(w.observe(&gone, 30_000).is_empty());
        assert_eq!(w.observe(&gone, 50_000).len(), 1);
    }

    #[test]
    fn waiting_for_something_that_is_not_there_is_refused() {
        let (_d, mut w) = watches();
        assert!(w.add(Action::PowerOff, Condition::GameOver, "", &Observation::default()).unwrap_err().contains("no game"));
        assert!(w.add(Action::PowerOff, Condition::ProcessGone { name: "blender".into() }, "", &Observation::default()).is_err());
        assert!(w.add(Action::PowerOff, Condition::CpuIdle { below_pct: 500.0, minutes: 5 }, "", &obs(0, 0.0)).is_err());
        assert!(w.add(Action::PowerOff, Condition::NetIdle { below_bps: 1, minutes: 0 }, "", &obs(0, 0.0)).is_err());
        assert!(w.is_empty());
    }

    #[test]
    fn watches_survive_a_restart_and_their_windows_start_over() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("conditions.json");
        let mut w = Watches::load(&p);
        let game = Observation { game_active: true, ..Default::default() };
        let a = w.add(Action::PowerOff, Condition::GameOver, "after the match", &game).unwrap();
        w.observe(&Observation::default(), 0); // game ended, confirmation running
        drop(w);
        let mut w = Watches::load(&p);
        assert_eq!(w.list()[0].id, a.id);
        assert_eq!(w.list()[0].note, "after the match");
        assert!(w.observe(&Observation::default(), 30_000).is_empty(), "window restarted after the restart");
        assert_eq!(w.observe(&Observation::default(), 50_000).len(), 1);
    }
}
