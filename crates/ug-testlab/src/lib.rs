//! The test lab: tries every feature on the user's own machine, measures a
//! few things, puts everything back and writes a plain-text report for the
//! user to email to the bug address.
//!
//! Why it exists: Unix Guardian is meant for every Linux system, but it has
//! been run for real on one laptop. Each user who runs the lab and sends the
//! report shows which sensors, interfaces and tools their distribution and
//! hardware really offer, without having to know what to look for.
//!
//! One engine for all front ends (window, `ug tui`, `ug testlab`), talking
//! to the daemon and agent through the same D-Bus interfaces they use, so the
//! lab tests what users use. Its rules:
//! - Consent first: the front ends explain what is turned on and ask.
//! - Everything turned on is verified, then turned off; the undo is on disk
//!   before the change ([`journal`]), and the next start puts back whatever an
//!   interrupted run left ([`recover`]).
//! - Nothing that touches the whole session or the compositor
//!   (`environment.d`, `/etc/environment`), no log-out or restart, no package
//!   installs, nothing behind a password prompt: those are listed as
//!   "needs a manual test" (the 2026-10-04 incident, see 05-findings).
//! - Personal identifiers are removed from the whole text ([`scrub`]), and
//!   nothing is ever sent: the user copies or saves it.

pub mod bench;
pub mod journal;
pub mod scrub;
mod steps;

use serde::Serialize;
use std::path::PathBuf;

/// Bumped when the report's layout changes, so old reports can be read.
pub const REPORT_FORMAT: u32 = 1;

/// The made-up names the lab creates things under; recognisable in any
/// list if something is ever left behind.
pub const PROBE: &str = "ug-testlab-probe";

/// What a run turns on for a moment, for the consent question.
pub const TEMPORARY_EN: &[&str] = &[
    "game mode (CPU and memory tuning, paused services) for a few seconds",
    "a full CPU load for about 6 seconds (a benchmark)",
    "a test profile, a tracked app and an internet rule for a made-up program",
    "a power action and a wake alarm scheduled 300 days ahead, cancelled at once",
    "a 3-second screen recording (if the recorder is set up), deleted at once",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pass,
    Warn,
    Fail,
    Skip,
    Info,
}

impl Status {
    fn tag(self) -> &'static str {
        match self {
            Status::Pass => "PASS",
            Status::Warn => "WARN",
            Status::Fail => "FAIL",
            Status::Skip => "SKIP",
            Status::Info => "INFO",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub area: String,
    pub name: String,
    pub status: Status,
    pub detail: String,
}

/// Sent before each step.
#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    /// Steps finished so far.
    pub done: usize,
    pub total: usize,
    /// Stable id the front ends translate (e.g. `gamemode`).
    pub id: &'static str,
    /// The English label.
    pub label: &'static str,
}

/// The steps in order: (id, label).
pub const STEPS: &[(&str, &str)] = &[
    ("recover", "Putting back anything an earlier test left"),
    ("env", "Collecting system details"),
    ("services", "Reaching the service and the agent"),
    ("sensors", "Reading sensors"),
    ("speed", "Measuring response times"),
    ("gpu", "Checking graphics cards"),
    ("cpu-bench", "CPU benchmark"),
    ("gamemode", "Turning game mode on and off"),
    ("profiles", "Profiles"),
    ("games", "Games and launchers"),
    ("playtime", "Playtime and tracked apps"),
    ("history", "Monitoring history, internet use and FPS"),
    ("ping", "Ping"),
    ("power", "Power, battery and wake alarm"),
    ("fans", "Fans"),
    ("net", "Internet control"),
    ("overlay", "Overlay"),
    ("recorder", "Recorder"),
    ("misc", "Shortcuts, settings and updates"),
    ("drivers", "Driver check"),
    ("cleanup", "Putting everything back"),
];

#[derive(Debug, Clone, Serialize, Default)]
pub struct Report {
    /// The whole report, already scrubbed: what the user copies.
    pub text: String,
    pub passed: u32,
    pub warned: u32,
    pub failed: u32,
    pub skipped: u32,
    /// Changes that could not be put back, in sentences (normally empty).
    pub leftovers: Vec<String>,
    pub checks: Vec<Check>,
}

#[derive(Debug, Clone)]
pub struct Options {
    /// Reach the daemon on the session bus (`ug-daemon --session`).
    pub daemon_on_session: bool,
    /// "window", "terminal UI" or "command line", for the report.
    pub front: &'static str,
    /// The undo journal; [`journal::path`] normally.
    pub journal: PathBuf,
    /// Short benchmark and recording, for tests.
    pub quick: bool,
}

impl Options {
    pub fn new(front: &'static str, daemon_on_session: bool) -> Options {
        Options { daemon_on_session, front, journal: journal::path(), quick: false }
    }
}

/// Runs every step and returns the scrubbed report. `progress` is called
/// before each step and once at the end (`done == total`).
pub async fn run<F: FnMut(Progress) + Send>(opts: &Options, mut progress: F) -> Result<Report, String> {
    let dir = opts.journal.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
    let _lock = journal::RunLock::take(&dir)?;
    let mut lab = steps::Lab::new(opts).await;
    let total = STEPS.len();
    for (i, (id, label)) in STEPS.iter().enumerate() {
        progress(Progress { done: i, total, id, label });
        lab.step(id).await;
    }
    progress(Progress { done: total, total, id: "done", label: "Done" });
    Ok(lab.finish())
}

/// Puts back whatever an interrupted run left; returns what was done, in
/// sentences. Quietly does nothing while a run is going (it owns the
/// journal) or when there is nothing to do.
pub async fn recover(opts: &Options) -> Vec<String> {
    if journal::Journal::open(&opts.journal).items().is_empty() {
        return Vec::new();
    }
    let dir = opts.journal.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
    let Ok(_lock) = journal::RunLock::take(&dir) else { return Vec::new() };
    let mut lab = steps::Lab::new(opts).await;
    lab.put_back().await
}

/// `2026-10-04 15:40 UTC`, without a date library.
pub(crate) fn utc_stamp(unix_ms: u64) -> String {
    let secs = unix_ms / 1000;
    let days = (secs / 86_400) as i64;
    let (h, m) = ((secs % 86_400) / 3600, (secs % 3600) / 60);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(mo <= 2);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02} UTC")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_written_without_a_date_library() {
        assert_eq!(utc_stamp(0), "1970-01-01 00:00 UTC");
        assert_eq!(utc_stamp(1_791_128_400_000), "2026-10-04 15:40 UTC");
        assert_eq!(utc_stamp(951_782_400_000), "2000-02-29 00:00 UTC");
    }

    /// The window runs the lab on Tauri's multi-threaded runtime, which
    /// needs a future that can move between threads.
    #[test]
    fn a_run_can_be_spawned_on_a_multi_threaded_runtime() {
        fn is_send<T: Send>(_: &T) {}
        let opts = Options::new("test", true);
        let run = run(&opts, |_| {});
        is_send(&run);
        let rec = recover(&opts);
        is_send(&rec);
    }

    #[test]
    fn every_step_has_a_unique_id() {
        let mut ids: Vec<&str> = STEPS.iter().map(|s| s.0).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), STEPS.len());
    }
}
