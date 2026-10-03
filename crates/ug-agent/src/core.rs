//! The agent's state: detection, profiles, playtime, pinning and pausing, kept
//! behind one lock and driven by a two-second tick.
//!
//! Synchronous on purpose. A tick is about a millisecond of `/proc` reads, and
//! keeping this free of async lets it be tested by calling `tick` with a fake
//! root and a fake clock.

use crate::{affinity, apps::Paused, choices::Choices, playtime::Playtime};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use ug_core::profile::Profiles;
use ug_games::detect::{Detector, Event, RunningGame};
use ug_games::{db::GameDb, InstalledGame};
use ug_hw::{topology, SysRoot};
use ug_proto::{InstalledGameInfo, RunningGameInfo, TrackedApp};

/// The PCI address of the GPU the firmware booted the display on.
fn boot_vga(root: &SysRoot) -> String {
    std::fs::read_dir(root.join("sys/class/drm"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("card") && !e.file_name().to_string_lossy().contains('-'))
        .find(|e| std::fs::read_to_string(e.path().join("device/boot_vga")).is_ok_and(|v| v.trim() == "1"))
        .and_then(|e| std::fs::read_link(e.path().join("device")).ok())
        .and_then(|l| l.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

/// What changed on a tick, for the async layer to announce and act on.
#[derive(Debug, Clone)]
pub enum Outcome {
    Started(RunningGameInfo),
    Stopped(RunningGameInfo, u64),
}

struct Running {
    info: RunningGameInfo,
    /// The game database's profile for it, which a cleared choice returns to.
    suggested: String,
    session: i64,
    /// Present when this game's profile keeps it on preferred cores.
    pin_to: Option<Vec<u32>>,
    pinned: BTreeSet<i32>,
}

/// A tracked app running now: only its time is recorded.
struct AppSession {
    name: String,
    session: i64,
    started_ms: u64,
}

pub struct AgentCore {
    root: SysRoot,
    home: PathBuf,
    config_dir: PathBuf,
    uid: u32,
    detector: Detector,
    installed: Vec<InstalledGame>,
    playtime: Arc<Playtime>,
    profiles: Profiles,
    preferred_cpus: Option<Vec<u32>>,
    running: BTreeMap<String, Running>,
    apps: BTreeMap<String, AppSession>,
    /// Tracked apps and "not a game" decisions, kept in `playtime.json`.
    choices: Choices,
    /// False when `playtime.json` did not parse: it is then never written,
    /// so the user's file is not lost to a save of the empty defaults.
    choices_writable: bool,
    test_run: Option<String>,
    paused: Paused,
}

impl AgentCore {
    pub fn new(root: SysRoot, home: &Path, config_dir: &Path, playtime: Arc<Playtime>, uid: u32) -> Self {
        let installed = ug_games::scan_installed(home);
        let (profiles, problems) = Profiles::load(config_dir);
        for p in problems {
            tracing::warn!(problem = %p, "profile file skipped");
        }
        let topo = topology::read(&root);
        let preferred_cpus = topology::preferred_cpus(&topo).map(|(cpus, why)| {
            tracing::info!(?cpus, reason = ?why, "preferred cores available for games");
            cpus
        });
        tracing::info!(installed = installed.len(), "game library scanned");
        let (choices, choices_writable) = match Choices::load(config_dir) {
            Ok(c) => (c, true),
            Err(e) => {
                tracing::warn!(error = %e, "tracked apps and not-a-game choices unreadable; left as they are");
                (Choices::default(), false)
            }
        };
        let mut detector = Detector::new(GameDb::bundled(), installed.clone(), uid);
        detector.ignore_exes(choices.not_games.clone());
        detector.set_tracked_apps(choices.tracked_pairs());

        AgentCore {
            detector,
            root,
            home: home.to_path_buf(),
            config_dir: config_dir.to_path_buf(),
            uid,
            installed,
            playtime,
            profiles,
            preferred_cpus,
            running: BTreeMap::new(),
            apps: BTreeMap::new(),
            choices,
            choices_writable,
            test_run: None,
            paused: Paused::default(),
        }
    }

    /// Scans once and applies whatever changed.
    pub fn tick(&mut self, now: Instant, wall_ms: u64) -> Vec<Outcome> {
        let events = self.detector.scan(&self.root, now);
        let outcomes = self.apply(events, wall_ms);
        self.refresh_members();
        outcomes
    }

    /// Games become outcomes for the async layer (game mode, signals,
    /// notifications); tracked apps only start and end a playtime session.
    fn apply(&mut self, events: Vec<Event>, wall_ms: u64) -> Vec<Outcome> {
        let mut out = Vec::new();
        for e in events {
            match e {
                Event::Started(g) if g.is_app() => {
                    tracing::info!(app = %g.name, "tracked app started");
                    let session = self.playtime.start(&g.key, &g.name, wall_ms);
                    self.apps.insert(g.key.clone(), AppSession { name: g.name, session, started_ms: wall_ms });
                }
                Event::Stopped { game, played } if game.is_app() => {
                    if let Some(a) = self.apps.remove(&game.key) {
                        self.playtime.touch(a.session, a.started_ms + played.as_millis() as u64);
                        tracing::info!(app = %a.name, played_s = played.as_secs(), "tracked app stopped");
                    }
                }
                Event::Started(g) => out.push(self.on_started(g, wall_ms)),
                Event::Stopped { game, played } => out.push(self.on_stopped(&game, played.as_secs(), wall_ms)),
            }
        }
        out
    }

    fn on_started(&mut self, g: RunningGame, wall_ms: u64) -> Outcome {
        let profile = self.profiles.for_game(&g.key, &g.profile).clone();
        let info = RunningGameInfo {
            key: g.key.clone(),
            name: g.name.clone(),
            evidence: serde_plain(g.evidence),
            profile: profile.id.clone(),
            pids: g.pids.clone(),
            blocked_note: g.blocked_note.clone().unwrap_or_default(),
            started_unix_ms: wall_ms,
            gpu: String::new(),
        };
        tracing::info!(game = %info.name, key = %info.key, evidence = %info.evidence, profile = %info.profile, "game started");

        let session = self.playtime.start(&info.key, &info.name, wall_ms);
        let pin_to = profile.cpu_preferred_cores.then(|| self.preferred_cpus.clone()).flatten();

        if self.running.is_empty() && !profile.pause_apps.is_empty() {
            let protect: Vec<i32> = g.pids.clone();
            self.paused.pause_matching(&self.root, self.uid, &profile.pause_apps, &protect);
        }

        self.running.insert(
            g.key.clone(),
            Running { info: info.clone(), suggested: g.profile.clone(), session, pin_to, pinned: BTreeSet::new() },
        );
        Outcome::Started(info)
    }

    fn on_stopped(&mut self, g: &RunningGame, played_s: u64, wall_ms: u64) -> Outcome {
        let running = self.running.remove(&g.key);
        let info = match running {
            Some(r) => {
                self.playtime.touch(r.session, r.info.started_unix_ms + played_s * 1000);
                r.info
            }
            None => RunningGameInfo {
                key: g.key.clone(),
                name: g.name.clone(),
                evidence: serde_plain(g.evidence),
                profile: g.profile.clone(),
                pids: Vec::new(),
                blocked_note: String::new(),
                started_unix_ms: wall_ms.saturating_sub(played_s * 1000),
                gpu: String::new(),
            },
        };
        tracing::info!(game = %info.name, played_s, "game stopped");

        if self.running.is_empty() {
            self.paused.resume_all(&self.root);
        }
        Outcome::Stopped(info, played_s)
    }

    /// Keeps each running game's PID list current and pins processes that
    /// joined since the last tick.
    fn refresh_members(&mut self) {
        let live: BTreeMap<String, Vec<i32>> =
            self.detector.running().into_iter().map(|g| (g.key, g.pids)).collect();
        for (key, r) in self.running.iter_mut() {
            if let Some(pids) = live.get(key) {
                r.info.pids = pids.clone();
            }
            if let Some(g) = ug_hw::gpu_use::gpu_of_game(&self.root, &r.info.pids, &boot_vga(&self.root)) {
                if g != r.info.gpu {
                    tracing::info!(game = %r.info.name, gpu = %g, "renders on");
                    r.info.gpu = g;
                }
            }
            if let Some(cpus) = &r.pin_to {
                let new: Vec<i32> = r.info.pids.iter().copied().filter(|p| !r.pinned.contains(p)).collect();
                if !new.is_empty() {
                    let threads = affinity::pin(&self.root, &new, cpus);
                    tracing::debug!(game = %r.info.name, ?new, threads, "pinned to preferred cores");
                    r.pinned.extend(new);
                }
            }
        }
    }

    /// Moves every running session's end time to now, so an unclean exit loses
    /// at most one heartbeat of playtime.
    pub fn heartbeat(&self, wall_ms: u64) {
        for r in self.running.values() {
            self.playtime.touch(r.session, wall_ms);
        }
        for a in self.apps.values() {
            self.playtime.touch(a.session, wall_ms);
        }
    }

    /// Records the end of every running session, for a clean shutdown.
    pub fn shutdown(&mut self, wall_ms: u64) {
        self.heartbeat(wall_ms);
        self.paused.resume_all(&self.root);
    }

    pub fn rescan(&mut self) -> usize {
        self.installed = ug_games::scan_installed(&self.home);
        self.detector.set_installed(self.installed.clone());
        let (profiles, problems) = Profiles::load(&self.config_dir);
        for p in problems {
            tracing::warn!(problem = %p, "profile file skipped");
        }
        self.profiles = profiles;
        self.installed.len()
    }

    /// See [`ug_games::Detector::set_test_run`].
    pub fn set_test_run(&mut self, token: Option<String>) {
        self.detector.set_test_run(token.clone());
        self.test_run = token;
    }

    /// Every profile, with the games the user assigned to it.
    pub fn profiles_info(&self) -> Vec<ug_proto::ProfileInfo> {
        self.profiles
            .all()
            .iter()
            .map(|p| ug_proto::ProfileInfo {
                id: p.id.clone(),
                builtin: ug_core::profile::is_builtin(&p.id),
                cpu_performance: p.cpu_performance,
                cpu_no_idle_states: p.cpu_no_idle_states,
                cpu_preferred_cores: p.cpu_preferred_cores,
                cpu_priority: p.cpu_priority,
                memory_tuning: p.memory_tuning,
                pause_services: p.pause_services,
                network_tuning: p.network_tuning,
                power_profile: p.power_profile,
                pause_apps: p.pause_apps.clone(),
                games: self.profiles.per_game().iter().filter(|(_, id)| **id == p.id).map(|(k, _)| k.clone()).collect(),
            })
            .collect()
    }

    /// Saves the user's profile choice for a game (`None` clears it) and
    /// applies it to the game if it is running: game mode follows on the next
    /// reconcile, and pinning starts for processes not yet pinned. Threads
    /// already pinned stay where they are until the game restarts; apps are
    /// paused or not according to the profile the first game started with.
    pub fn set_game_profile(&mut self, key: &str, profile: Option<&str>) -> Result<(), String> {
        let key = key.trim();
        if key.is_empty() {
            return Err("a game key is required, e.g. steam:730 (see `ug game list`)".into());
        }
        // Re-read first, so a profile file added since start-up can be chosen.
        let (fresh, _) = Profiles::load(&self.config_dir);
        if let Some(id) = profile {
            if fresh.get(id).is_none() {
                let known: Vec<&str> = fresh.all().iter().map(|p| p.id.as_str()).collect();
                return Err(format!("no profile named {id:?}; available: {}", known.join(", ")));
            }
        }
        ug_core::profile::set_game_profile(&self.config_dir, key, profile)?;
        self.profiles = Profiles::load(&self.config_dir).0;

        if let Some(r) = self.running.get_mut(key) {
            let p = self.profiles.for_game(key, &r.suggested);
            tracing::info!(game = %r.info.name, from = %r.info.profile, to = %p.id, "profile changed while running");
            r.info.profile = p.id.clone();
            r.pin_to = p.cpu_preferred_cores.then(|| self.preferred_cpus.clone()).flatten();
        }
        Ok(())
    }

    /// Changes one setting of an existing profile. Game mode follows on the
    /// next reconcile for any game running with it.
    pub fn set_profile_option(&mut self, id: &str, option: &str, value: bool) -> Result<(), String> {
        let (fresh, _) = Profiles::load(&self.config_dir);
        if fresh.get(id).is_none() {
            let known: Vec<&str> = fresh.all().iter().map(|p| p.id.as_str()).collect();
            return Err(format!("no profile named {id:?}; available: {}", known.join(", ")));
        }
        ug_core::profile::set_profile_option(&self.config_dir, id, option, value)?;
        self.profiles = Profiles::load(&self.config_dir).0;
        Ok(())
    }

    /// A GameMode `RegisterGame` call.
    pub fn register(&mut self, pid: i32) {
        tracing::debug!(pid, "registered through GameMode");
        self.detector.register(pid);
    }

    /// "Not a game", remembered across sessions. Returns outcomes for
    /// anything this stops.
    pub fn ignore(&mut self, exe: &str, now: Instant, wall_ms: u64) -> Vec<Outcome> {
        if self.choices.not_a_game(exe) {
            self.save_choices();
        }
        let events = self.detector.ignore_exe(exe, now);
        self.apply(events, wall_ms)
    }

    fn save_choices(&self) {
        if !self.choices_writable {
            tracing::warn!("playtime.json was unreadable at start-up; this choice lasts until the agent restarts");
            return;
        }
        if let Err(e) = self.choices.save(&self.config_dir) {
            tracing::warn!(error = %e, "could not save tracked apps");
        }
    }

    pub fn tracked_apps(&self) -> Vec<TrackedApp> {
        self.choices.apps.clone()
    }

    /// Starts counting a program's time. Running already, it starts counting
    /// on the next tick.
    pub fn track_app(&mut self, exe: &str, name: &str) -> Result<(), String> {
        self.choices.track(exe, name)?;
        self.save_choices();
        self.detector.set_tracked_apps(self.choices.tracked_pairs());
        Ok(())
    }

    /// Stops counting it; its time stays. Returns whether it was tracked.
    pub fn untrack_app(&mut self, exe: &str) -> bool {
        let was = self.choices.untrack(exe);
        if was {
            self.save_choices();
            self.detector.set_tracked_apps(self.choices.tracked_pairs());
        }
        was
    }

    /// "Not a game" for a game recognised only by its behaviour: remembered
    /// as not a game, tracked as an app, its sessions so far moved along.
    /// Returns the sessions moved and outcomes for the game this stops.
    pub fn move_to_apps(&mut self, key: &str, now: Instant, wall_ms: u64) -> Result<(u32, Vec<Outcome>), String> {
        let Some(exe) = key.strip_prefix("exe:").filter(|e| !e.is_empty()) else {
            return Err(format!(
                "{key} was recognised by its launcher or the game database, not by guesswork; only guessed games can be moved"
            ));
        };
        let name = self
            .playtime
            .summary(0)
            .into_iter()
            .find(|e| e.key == key)
            .map(|e| e.name)
            .or_else(|| self.running.get(key).map(|r| r.info.name.clone()))
            .unwrap_or_else(|| exe.to_string());
        let outcomes = self.ignore(exe, now, wall_ms);
        let moved = self.playtime.move_key(key, &format!("app:{exe}"));
        self.track_app(exe, &name)?;
        Ok((moved, outcomes))
    }

    /// Steam launch options for the account used last, with a name for each
    /// game. Returns what the work needs, so files are read without the
    /// agent's lock: (launch, uid, name of `appid`).
    pub fn steam_launch(&self, appid: u32) -> Result<(ug_games::launch::Launch, u32, String), String> {
        let steam = ug_games::launch::Steam::find(&self.home)?;
        let name = self
            .installed
            .iter()
            .find(|g| g.steam_appid == Some(appid))
            .map(|g| g.name.clone())
            .or_else(|| GameDb::bundled().by_steam_appid(appid).map(|k| k.name.clone()))
            .unwrap_or_else(|| format!("Steam app {appid}"));
        let launch = ug_games::launch::Launch::new(steam, self.root.clone(), &self.home, &self.config_dir, &ug_core::paths::user_data());
        Ok((launch, self.uid, name))
    }

    /// Programs open now that could be tracked. Returns the work to do, so
    /// it runs without holding the agent's lock.
    pub fn running_programs(&self) -> impl FnOnce() -> Vec<TrackedApp> + Send + 'static {
        let skip: Vec<String> = self.choices.apps.iter().map(|a| a.exe.clone()).collect();
        let (root, uid, test_run) = (self.root.clone(), self.uid, self.test_run.clone());
        move || crate::programs::running(&root, uid, test_run.as_deref(), &crate::programs::application_dirs(), &skip)
    }

    /// Imports Steam's own playtime (see [`Playtime::import`]). Games get the
    /// library's name, the database's, or "Steam app <id>". Returns the work
    /// to do, so the file is read without holding the agent's lock.
    pub fn import_steam(&self) -> impl FnOnce() -> Result<(u32, u64), String> + Send + 'static {
        let (home, installed, playtime) = (self.home.clone(), self.installed.clone(), Arc::clone(&self.playtime));
        move || {
            let steam = ug_games::steam::playtime(&home)?;
            let db = GameDb::bundled();
            let games: Vec<(String, String, u64, u64)> = steam
                .iter()
                .map(|s| {
                    let name = installed
                        .iter()
                        .find(|g| g.steam_appid == Some(s.appid))
                        .map(|g| g.name.clone())
                        .or_else(|| db.by_steam_appid(s.appid).map(|k| k.name.clone()))
                        .unwrap_or_else(|| format!("Steam app {}", s.appid));
                    (format!("steam:{}", s.appid), name, s.minutes * 60, s.last_played_unix * 1000)
                })
                .collect();
            let r = playtime.import("steam", &games).map_err(|e| format!("could not save the import: {e}"))?;
            tracing::info!(found = games.len(), games = r.0, seconds = r.1, "Steam playtime imported");
            Ok(r)
        }
    }

    /// What game mode should be right now: `None` with no game running,
    /// otherwise every running game's profile combined — a setting is on if
    /// any running game wants it — covering every game process.
    pub fn gamemode_desired(&self) -> Option<ug_proto::GameModeRequest> {
        let mut want: Option<ug_proto::GameModeRequest> = None;
        for r in self.running.values() {
            let Some(p) = self.profiles.get(&r.info.profile) else { continue };
            let w = want.get_or_insert_with(Default::default);
            w.cpu_performance |= p.cpu_performance;
            w.cpu_no_idle_states |= p.cpu_no_idle_states;
            w.cpu_priority |= p.cpu_priority;
            w.memory_tuning |= p.memory_tuning;
            w.pause_services |= p.pause_services;
            w.network_tuning |= p.network_tuning;
            w.pids.extend(&r.info.pids);
        }
        if let Some(w) = want.as_mut() {
            w.pids.sort_unstable();
            w.pids.dedup();
        }
        want
    }

    /// Whether a running game's profile asks for the performance power
    /// profile, and that game's name.
    pub fn power_profile_wanted(&self) -> (bool, String) {
        self.running
            .values()
            .find(|r| self.profiles.get(&r.info.profile).is_some_and(|p| p.power_profile))
            .map(|r| (true, r.info.name.clone()))
            .unwrap_or_default()
    }

    pub fn is_running_pid(&self, pid: i32) -> bool {
        self.running.values().any(|r| r.info.pids.contains(&pid))
    }

    pub fn running(&self) -> Vec<RunningGameInfo> {
        self.running.values().map(|r| r.info.clone()).collect()
    }

    pub fn installed(&self) -> Vec<InstalledGameInfo> {
        self.installed
            .iter()
            .map(|g| InstalledGameInfo { id: g.id.clone(), name: g.name.clone(), launcher: serde_plain(g.launcher) })
            .collect()
    }

    pub fn playtime(&self) -> &Playtime {
        &self.playtime
    }
}

/// An enum's serde name (`steam_app_id`, `bottles`), for wire strings.
fn serde_plain<T: serde::Serialize>(v: T) -> String {
    serde_json::to_value(v).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use ug_testfs::{FakeProc, TestFs};

    fn core(fs: &TestFs, config: &Path) -> AgentCore {
        AgentCore::new(SysRoot::at(fs.root()), &fs.root().join("home"), config, Arc::new(Playtime::in_memory()), 1000)
    }

    #[test]
    fn a_game_session_is_recorded_from_start_to_stop() {
        let fs = TestFs::new();
        let cfg = tempfile::tempdir().unwrap();
        let mut c = core(&fs, cfg.path());
        let t0 = Instant::now();

        fs.process(&FakeProc { pid: 10, comm: "cs2", cmdline: &["cs2"], environ: &[("SteamAppId", "730")], ..Default::default() });
        let out = c.tick(t0, 1_000_000);
        assert!(matches!(&out[..], [Outcome::Started(g)] if g.name == "Counter-Strike 2" && g.profile == "competitive"));
        assert_eq!(c.running().len(), 1);

        fs.kill(10);
        c.tick(t0 + Duration::from_secs(90), 1_090_000);
        let out = c.tick(t0 + Duration::from_secs(94), 1_094_000);
        assert!(matches!(&out[..], [Outcome::Stopped(g, 90)] if g.key == "steam:730"));

        let s = c.playtime().summary(0);
        assert_eq!(s[0].key, "steam:730");
        assert_eq!(s[0].seconds, 90);
    }

    #[test]
    fn the_users_profile_choice_applies() {
        let fs = TestFs::new();
        let cfg = tempfile::tempdir().unwrap();
        std::fs::write(cfg.path().join("games.toml"), "[games]\n\"steam:730\" = \"quiet\"\n").unwrap();
        let mut c = core(&fs, cfg.path());
        fs.process(&FakeProc { pid: 10, comm: "cs2", cmdline: &["cs2"], environ: &[("SteamAppId", "730")], ..Default::default() });
        let out = c.tick(Instant::now(), 0);
        assert!(matches!(&out[..], [Outcome::Started(g)] if g.profile == "quiet"));
    }

    #[test]
    fn game_mode_wants_the_union_of_every_running_games_profile() {
        let fs = TestFs::new();
        let cfg = tempfile::tempdir().unwrap();
        std::fs::write(cfg.path().join("games.toml"), "[games]\n\"steam:413150\" = \"quiet\"\n").unwrap();
        let mut c = core(&fs, cfg.path());
        assert_eq!(c.gamemode_desired(), None, "no game, no game mode");

        fs.process(&FakeProc { pid: 10, comm: "sv", cmdline: &["sv"], environ: &[("SteamAppId", "413150")], ..Default::default() });
        c.tick(Instant::now(), 0);
        let quiet_only = c.gamemode_desired().unwrap();
        assert!(!quiet_only.cpu_no_idle_states && !quiet_only.cpu_performance);

        fs.process(&FakeProc { pid: 11, comm: "cs2", cmdline: &["cs2"], environ: &[("SteamAppId", "730")], ..Default::default() });
        c.tick(Instant::now(), 0);
        let both = c.gamemode_desired().unwrap();
        assert!(both.cpu_no_idle_states && both.cpu_performance, "competitive CS2 raises the bar for both");
        assert_eq!(both.pids, vec![10, 11]);
    }

    #[test]
    fn choosing_a_profile_for_a_running_game_changes_what_game_mode_wants() {
        let fs = TestFs::new();
        let cfg = tempfile::tempdir().unwrap();
        let mut c = core(&fs, cfg.path());
        fs.process(&FakeProc { pid: 10, comm: "cs2", cmdline: &["cs2"], environ: &[("SteamAppId", "730")], ..Default::default() });
        c.tick(Instant::now(), 0);
        assert!(c.gamemode_desired().unwrap().cpu_no_idle_states, "competitive by default");

        c.set_game_profile("steam:730", Some("quiet")).unwrap();
        assert_eq!(c.running()[0].profile, "quiet");
        let want = c.gamemode_desired().unwrap();
        assert!(!want.cpu_no_idle_states && !want.cpu_performance);
        let quiet = c.profiles_info().into_iter().find(|p| p.id == "quiet").unwrap();
        assert_eq!(quiet.games, vec!["steam:730"]);

        c.set_game_profile("steam:730", None).unwrap();
        assert_eq!(c.running()[0].profile, "competitive", "back to the database's suggestion");
    }

    #[test]
    fn an_unknown_profile_is_refused_and_nothing_is_written() {
        let fs = TestFs::new();
        let cfg = tempfile::tempdir().unwrap();
        let mut c = core(&fs, cfg.path());
        let err = c.set_game_profile("steam:730", Some("turbo")).unwrap_err();
        assert!(err.contains("competitive"), "the error lists what exists: {err}");
        assert!(!cfg.path().join("games.toml").exists());
    }

    #[test]
    fn a_profile_file_added_after_start_up_can_be_chosen() {
        let fs = TestFs::new();
        let cfg = tempfile::tempdir().unwrap();
        let mut c = core(&fs, cfg.path());
        std::fs::create_dir_all(cfg.path().join("profiles")).unwrap();
        std::fs::write(cfg.path().join("profiles/mine.toml"), "cpu_preferred_cores = true\n").unwrap();
        c.set_game_profile("steam:730", Some("mine")).unwrap();
        let mine = c.profiles_info().into_iter().find(|p| p.id == "mine").unwrap();
        assert!(!mine.builtin && mine.cpu_preferred_cores);
    }

    #[test]
    fn changing_a_profile_setting_reaches_a_running_games_game_mode() {
        let fs = TestFs::new();
        let cfg = tempfile::tempdir().unwrap();
        let mut c = core(&fs, cfg.path());
        fs.process(&FakeProc { pid: 10, comm: "cs2", cmdline: &["cs2"], environ: &[("SteamAppId", "730")], ..Default::default() });
        c.tick(Instant::now(), 0);
        assert!(c.gamemode_desired().unwrap().cpu_no_idle_states);
        c.set_profile_option("competitive", "cpu_no_idle_states", false).unwrap();
        assert!(!c.gamemode_desired().unwrap().cpu_no_idle_states);
        assert!(!c.profiles_info().into_iter().find(|p| p.id == "competitive").unwrap().cpu_no_idle_states);
        assert!(c.set_profile_option("turbo", "cpu_priority", true).unwrap_err().contains("available"));
    }

    fn tool(cpu: u64) -> FakeProc<'static> {
        FakeProc { pid: 20, comm: "Tool.exe", cmdline: &[r"C:\t\Tool.exe"], maps: &["/x/d3d11.dll"], cpu_ticks: cpu, ..Default::default() }
    }

    #[test]
    fn not_a_game_is_remembered_after_the_agent_restarts() {
        let fs = TestFs::new();
        let cfg = tempfile::tempdir().unwrap();
        let t0 = Instant::now();
        {
            let mut c = core(&fs, cfg.path());
            fs.process(&tool(0));
            c.tick(t0, 0);
            fs.process(&tool(100_000));
            assert_eq!(c.tick(t0 + Duration::from_secs(6), 6_000).len(), 1, "taken for a game");
            c.ignore("Tool.exe", t0 + Duration::from_secs(8), 8_000);
        }
        // The next login: a new agent, the same program still busy.
        let mut c = core(&fs, cfg.path());
        fs.process(&tool(200_000));
        c.tick(t0 + Duration::from_secs(10), 10_000);
        fs.process(&tool(400_000));
        assert!(c.tick(t0 + Duration::from_secs(16), 16_000).is_empty(), "still not a game");
    }

    #[test]
    fn a_tracked_app_records_its_time_and_never_wants_game_mode() {
        let fs = TestFs::new();
        let cfg = tempfile::tempdir().unwrap();
        let mut c = core(&fs, cfg.path());
        c.track_app("dolphin", "Dolphin").unwrap();
        let t0 = Instant::now();
        fs.process(&FakeProc { pid: 30, comm: "dolphin", exe: Some("/usr/bin/dolphin"), cmdline: &["/usr/bin/dolphin"], ..Default::default() });
        assert!(c.tick(t0, 1_000_000).is_empty(), "no game started");
        assert_eq!(c.gamemode_desired(), None);
        c.heartbeat(1_060_000);
        assert_eq!(c.playtime().summary(0)[0].seconds, 60, "counted while running");
        fs.kill(30);
        c.tick(t0 + Duration::from_secs(120), 1_120_000);
        c.tick(t0 + Duration::from_secs(124), 1_124_000);
        let s = c.playtime().summary(0);
        assert_eq!((s[0].key.as_str(), s[0].name.as_str(), s[0].seconds), ("app:dolphin", "Dolphin", 120));
        assert_eq!(c.tracked_apps().len(), 1);
        // Remembered with the other choices.
        assert_eq!(core(&fs, cfg.path()).tracked_apps()[0].exe, "dolphin");
    }

    #[test]
    fn a_guessed_game_moves_to_apps_with_its_time() {
        let fs = TestFs::new();
        let cfg = tempfile::tempdir().unwrap();
        let mut c = core(&fs, cfg.path());
        let t0 = Instant::now();
        fs.process(&tool(0));
        c.tick(t0, 0);
        fs.process(&tool(100_000));
        c.tick(t0 + Duration::from_secs(6), 6_000);
        c.heartbeat(66_000);
        assert_eq!(c.playtime().summary(0)[0].key, "exe:tool");

        let (moved, out) = c.move_to_apps("exe:tool", t0 + Duration::from_secs(70), 70_000).unwrap();
        assert_eq!(moved, 1);
        assert!(matches!(&out[..], [Outcome::Stopped(..)]), "game mode is let go at once");
        assert!(c.running().is_empty());
        // From the next tick the same process counts as the app.
        c.tick(t0 + Duration::from_secs(72), 72_000);
        c.heartbeat(132_000);
        let s = c.playtime().summary(0);
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].key.as_str(), s[0].name.as_str(), s[0].sessions), ("app:tool", "Tool", 2));
        assert!(c.move_to_apps("steam:730", t0, 0).unwrap_err().contains("only guessed games"));
    }

    #[test]
    fn steam_playtime_is_imported_with_names() {
        let fs = TestFs::new();
        fs.write("home/.local/share/Steam/steamapps/libraryfolders.vdf", "\"libraryfolders\" {}");
        fs.write(
            "home/.local/share/Steam/userdata/1/config/localconfig.vdf",
            r#""UserLocalConfigStore" { "Software" { "Valve" { "Steam" { "apps" {
                "730" { "LastPlayed" "1790000000" "Playtime" "120" }
                "999999" { "Playtime" "30" }
            } } } } }"#,
        );
        let cfg = tempfile::tempdir().unwrap();
        let c = core(&fs, cfg.path());
        assert_eq!(c.import_steam()().unwrap(), (2, 150 * 60));
        let s = c.playtime().summary(0);
        assert_eq!((s[0].name.as_str(), s[0].seconds, s[0].last_played_unix_ms), ("Counter-Strike 2", 7200, 1_790_000_000_000));
        assert_eq!(s[1].name, "Steam app 999999");
    }

    #[test]
    fn dismissing_a_heuristic_game_stops_its_session() {
        let fs = TestFs::new();
        let cfg = tempfile::tempdir().unwrap();
        let mut c = core(&fs, cfg.path());
        let t0 = Instant::now();
        let p = |cpu| FakeProc { pid: 20, comm: "Tool.exe", cmdline: &[r"C:\t\Tool.exe"], maps: &["/x/d3d11.dll"], cpu_ticks: cpu, ..Default::default() };
        fs.process(&p(0));
        c.tick(t0, 0);
        fs.process(&p(100_000));
        assert_eq!(c.tick(t0 + Duration::from_secs(6), 6_000).len(), 1);
        let out = c.ignore("Tool.exe", t0 + Duration::from_secs(8), 8_000);
        assert!(matches!(&out[..], [Outcome::Stopped(..)]));
        assert!(c.running().is_empty());
    }
}
