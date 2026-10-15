//! The agent's D-Bus interfaces on the session bus.

use crate::core::AgentCore;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use ug_proto::{
    DayTraffic, HistoryPoint, InstalledGameInfo, PingComparison, PingReport, PlaytimeEntry,
    PlaytimeSession, ProfileInfo, RunningGameInfo,
};
use zbus::object_server::SignalEmitter;

pub type Shared = Arc<Mutex<AgentCore>>;

pub fn lock(core: &Shared) -> std::sync::MutexGuard<'_, AgentCore> {
    core.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `org.unixguardian.Agent1.Games`
pub struct GamesIface {
    pub core: Shared,
    /// Lets `IgnoreExecutable` announce the stop it causes; set once the
    /// connection exists.
    pub announce: tokio::sync::mpsc::UnboundedSender<crate::core::Outcome>,
}

#[zbus::interface(name = "org.unixguardian.Agent1.Games")]
impl GamesIface {
    async fn installed(&self) -> Vec<InstalledGameInfo> {
        lock(&self.core).installed()
    }

    async fn running(&self) -> Vec<RunningGameInfo> {
        lock(&self.core).running()
    }

    async fn rescan(&self) -> u32 {
        let core = Arc::clone(&self.core);
        tokio::task::spawn_blocking(move || lock(&core).rescan() as u32)
            .await
            .unwrap_or(0)
    }

    async fn ignore_executable(&self, name: &str) -> zbus::fdo::Result<()> {
        if name.trim().is_empty() {
            return Err(zbus::fdo::Error::InvalidArgs(
                "an executable name is required".into(),
            ));
        }
        let outcomes = lock(&self.core).ignore(name, Instant::now(), now_ms());
        for o in outcomes {
            let _ = self.announce.send(o);
        }
        Ok(())
    }

    async fn launch_options(&self, appid: u32) -> zbus::fdo::Result<ug_proto::LaunchInfo> {
        let (launch, uid, name) = lock(&self.core)
            .steam_launch(appid)
            .map_err(zbus::fdo::Error::Failed)?;
        tokio::task::spawn_blocking(move || -> Result<ug_proto::LaunchInfo, String> {
            let st = launch.state(appid)?;
            let flatpak = launch.steam.flatpak;
            let overlay_problem = launch.overlay_problem().unwrap_or_default();
            Ok(ug_proto::LaunchInfo {
                appid,
                name,
                current: st.current,
                own: st.own,
                gpu: st.additions.gpu,
                overlay: st.additions.overlay,
                changed_outside: st.changed_outside,
                flatpak,
                steam_running: launch.steam_running(uid),
                overlay_problem,
            })
        })
        .await
        .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
        .map_err(zbus::fdo::Error::Failed)
    }

    async fn preview_launch_options(
        &self,
        appid: u32,
        gpu: &str,
        overlay: bool,
    ) -> zbus::fdo::Result<String> {
        let (launch, _, _) = lock(&self.core)
            .steam_launch(appid)
            .map_err(zbus::fdo::Error::Failed)?;
        let add = ug_games::launch::Additions {
            gpu: gpu.to_string(),
            overlay,
        };
        launch
            .preview(appid, &add)
            .map_err(zbus::fdo::Error::Failed)
    }

    async fn set_launch_options(
        &self,
        appid: u32,
        gpu: &str,
        overlay: bool,
    ) -> zbus::fdo::Result<(String, String)> {
        let (launch, uid, _) = lock(&self.core)
            .steam_launch(appid)
            .map_err(zbus::fdo::Error::Failed)?;
        let add = ug_games::launch::Additions {
            gpu: gpu.to_string(),
            overlay,
        };
        let a = tokio::task::spawn_blocking(move || launch.apply(appid, &add, uid))
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
            .map_err(zbus::fdo::Error::Failed)?;
        Ok((a.options, a.backup.display().to_string()))
    }

    async fn undo_launch_options(&self, appid: u32) -> zbus::fdo::Result<(String, String)> {
        let (launch, uid, _) = lock(&self.core)
            .steam_launch(appid)
            .map_err(zbus::fdo::Error::Failed)?;
        let a = tokio::task::spawn_blocking(move || launch.undo(appid, uid))
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
            .map_err(zbus::fdo::Error::Failed)?;
        Ok((a.options, a.backup.display().to_string()))
    }

    #[zbus(signal)]
    pub async fn game_started(
        emitter: &SignalEmitter<'_>,
        game: RunningGameInfo,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn game_stopped(
        emitter: &SignalEmitter<'_>,
        game: RunningGameInfo,
        played_s: u64,
    ) -> zbus::Result<()>;
}

/// `org.unixguardian.Agent1.Playtime`
pub struct PlaytimeIface {
    pub core: Shared,
    /// Lets `MoveToApps` announce the game it stops.
    pub announce: tokio::sync::mpsc::UnboundedSender<crate::core::Outcome>,
}

#[zbus::interface(name = "org.unixguardian.Agent1.Playtime")]
impl PlaytimeIface {
    async fn summary(&self, since_unix_ms: u64) -> Vec<PlaytimeEntry> {
        let core = lock(&self.core);
        // Running sessions are only as current as the last heartbeat; bring
        // them up to now so "today" includes the game being played.
        core.heartbeat(now_ms());
        core.playtime().summary(since_unix_ms)
    }

    async fn forget(&self, key: &str) -> u32 {
        lock(&self.core).playtime().forget(key)
    }

    async fn sessions(&self, since_unix_ms: u64) -> Vec<PlaytimeSession> {
        let core = lock(&self.core);
        core.heartbeat(now_ms());
        core.playtime().sessions(since_unix_ms)
    }

    async fn tracked_apps(&self) -> Vec<ug_proto::TrackedApp> {
        lock(&self.core).tracked_apps()
    }

    async fn track_app(&self, exe: &str, name: &str) -> zbus::fdo::Result<()> {
        lock(&self.core)
            .track_app(exe, name)
            .map_err(zbus::fdo::Error::InvalidArgs)
    }

    async fn untrack_app(&self, exe: &str) -> bool {
        lock(&self.core).untrack_app(exe)
    }

    /// Reads the process table and desktop entries: off the async threads.
    async fn running_programs(&self) -> Vec<ug_proto::TrackedApp> {
        let job = lock(&self.core).running_programs();
        tokio::task::spawn_blocking(job).await.unwrap_or_default()
    }

    async fn move_to_apps(&self, key: &str) -> zbus::fdo::Result<u32> {
        let (moved, outcomes) = lock(&self.core)
            .move_to_apps(key, Instant::now(), now_ms())
            .map_err(zbus::fdo::Error::InvalidArgs)?;
        for o in outcomes {
            let _ = self.announce.send(o);
        }
        Ok(moved)
    }

    async fn import_steam(&self) -> zbus::fdo::Result<(u32, u64)> {
        let job = lock(&self.core).import_steam();
        tokio::task::spawn_blocking(job)
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
            .map_err(zbus::fdo::Error::Failed)
    }

    async fn imported(&self) -> Vec<PlaytimeEntry> {
        lock(&self.core).playtime().imported()
    }

    async fn forget_imported(&self) -> u32 {
        lock(&self.core).playtime().forget_imported()
    }
}

/// `org.unixguardian.Agent1.Profiles`
pub struct ProfilesIface {
    pub core: Shared,
}

#[zbus::interface(name = "org.unixguardian.Agent1.Profiles")]
impl ProfilesIface {
    async fn list(&self) -> Vec<ProfileInfo> {
        lock(&self.core).profiles_info()
    }

    async fn set_game_profile(&self, key: &str, profile: &str) -> zbus::fdo::Result<()> {
        let profile = Some(profile.trim()).filter(|p| !p.is_empty());
        lock(&self.core)
            .set_game_profile(key, profile)
            .map_err(zbus::fdo::Error::InvalidArgs)
    }

    async fn set_option(&self, profile: &str, option: &str, value: bool) -> zbus::fdo::Result<()> {
        lock(&self.core)
            .set_profile_option(profile, option, value)
            .map_err(zbus::fdo::Error::InvalidArgs)
    }

    /// Whether game mode was set by hand ("on", "off" or "") and the
    /// profile it uses when on by hand.
    async fn game_mode(&self) -> (String, String) {
        let c = lock(&self.core);
        (
            c.gamemode_by_hand().to_string(),
            c.by_hand_profile().to_string(),
        )
    }

    async fn set_game_mode(&self, on: bool, profile: &str) -> zbus::fdo::Result<()> {
        lock(&self.core)
            .set_gamemode(on, profile)
            .map_err(zbus::fdo::Error::InvalidArgs)
    }

    async fn set_pause_apps(&self, profile: &str, apps: Vec<String>) -> zbus::fdo::Result<()> {
        lock(&self.core)
            .set_pause_apps(profile, &apps)
            .map_err(zbus::fdo::Error::InvalidArgs)
    }

    async fn create(&self, profile: &str, from: &str) -> zbus::fdo::Result<()> {
        lock(&self.core)
            .create_profile(profile.trim(), from.trim())
            .map_err(zbus::fdo::Error::InvalidArgs)
    }

    async fn delete(&self, profile: &str) -> zbus::fdo::Result<()> {
        lock(&self.core)
            .delete_profile(profile.trim())
            .map_err(zbus::fdo::Error::InvalidArgs)
    }
}

/// `org.unixguardian.Agent1.Monitor`
pub struct MonitorIface {
    pub history: Arc<crate::history::History>,
    pub overlay: Arc<crate::overlay::Overlay>,
}

#[zbus::interface(name = "org.unixguardian.Agent1.Monitor")]
impl MonitorIface {
    async fn history(
        &self,
        since_unix_ms: u64,
        until_unix_ms: u64,
        points: u32,
    ) -> Vec<HistoryPoint> {
        let h = Arc::clone(&self.history);
        tokio::task::spawn_blocking(move || h.range(since_unix_ms, until_unix_ms, points))
            .await
            .unwrap_or_default()
    }

    async fn battery_report(&self, since_unix_ms: u64, until_unix_ms: u64) -> ug_proto::BatteryReport {
        let h = Arc::clone(&self.history);
        tokio::task::spawn_blocking(move || {
            let rows = h.battery_rows(since_unix_ms, until_unix_ms);
            // Health looks at the last 30 days whatever span is shown: one
            // morning holds too little discharge to measure.
            let month = until_unix_ms.saturating_sub(30 * 24 * 3_600_000);
            let health_rows = h.battery_rows(month, until_unix_ms);
            let info = ug_hw::battery::battery_info(&ug_hw::SysRoot::real());
            crate::battery_report::report(&rows, info.as_ref(), &health_rows)
        })
        .await
        .unwrap_or_default()
    }

    async fn traffic(&self, since_unix_ms: u64) -> Vec<DayTraffic> {
        let h = Arc::clone(&self.history);
        tokio::task::spawn_blocking(move || h.traffic(since_unix_ms, local_date))
            .await
            .unwrap_or_default()
    }

    async fn traffic_buckets(
        &self,
        since_unix_ms: u64,
        until_unix_ms: u64,
        bucket_ms: u64,
    ) -> Vec<ug_proto::TrafficBucket> {
        let h = Arc::clone(&self.history);
        tokio::task::spawn_blocking(move || {
            h.traffic_buckets(since_unix_ms, until_unix_ms, bucket_ms)
        })
        .await
        .unwrap_or_default()
    }

    async fn app_traffic(&self, since_unix_ms: u64, until_unix_ms: u64) -> Vec<ug_proto::AppUsage> {
        let h = Arc::clone(&self.history);
        tokio::task::spawn_blocking(move || h.app_traffic(since_unix_ms, until_unix_ms))
            .await
            .unwrap_or_default()
    }

    async fn reset_traffic(&self) -> bool {
        let h = Arc::clone(&self.history);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        tokio::task::spawn_blocking(move || h.reset_traffic(now))
            .await
            .unwrap_or(false)
    }

    async fn traffic_reset_at(&self) -> u64 {
        let h = Arc::clone(&self.history);
        tokio::task::spawn_blocking(move || h.reset_at())
            .await
            .unwrap_or(0)
    }

    /// The running game's FPS from MangoHud's log.
    async fn fps(&self) -> ug_proto::FpsReading {
        let o = Arc::clone(&self.overlay);
        tokio::task::spawn_blocking(move || o.fps.read(std::time::SystemTime::now()))
            .await
            .unwrap_or_default()
    }

    async fn fps_sessions(&self, since_unix_ms: u64) -> Vec<ug_proto::FpsSession> {
        let h = Arc::clone(&self.history);
        tokio::task::spawn_blocking(move || h.fps_sessions(since_unix_ms))
            .await
            .unwrap_or_default()
    }

    async fn forget_fps_sessions(&self) -> u32 {
        let h = Arc::clone(&self.history);
        tokio::task::spawn_blocking(move || h.forget_fps())
            .await
            .unwrap_or(0)
    }

    /// Adds (true) or removes (false) Unix Guardian's block in MangoHud.conf.
    async fn set_up_fps(&self, on: bool) -> zbus::fdo::Result<()> {
        let o = Arc::clone(&self.overlay);
        tokio::task::spawn_blocking(move || o.set_up(on))
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
            .map_err(zbus::fdo::Error::Failed)
    }
}

/// `org.unixguardian.Agent1.Overlay`
pub struct OverlayIface {
    pub overlay: Arc<crate::overlay::Overlay>,
}

#[zbus::interface(name = "org.unixguardian.Agent1.Overlay")]
impl OverlayIface {
    async fn state(&self) -> ug_proto::OverlayState {
        let o = Arc::clone(&self.overlay);
        tokio::task::spawn_blocking(move || o.state())
            .await
            .unwrap_or_default()
    }

    async fn set(&self, settings: ug_proto::OverlaySettings) -> zbus::fdo::Result<()> {
        let o = Arc::clone(&self.overlay);
        tokio::task::spawn_blocking(move || o.set(settings))
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
            .map_err(zbus::fdo::Error::InvalidArgs)
    }

    async fn set_up(&self, on: bool) -> zbus::fdo::Result<()> {
        let o = Arc::clone(&self.overlay);
        tokio::task::spawn_blocking(move || o.set_up(on))
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
            .map_err(zbus::fdo::Error::Failed)
    }
}

/// `org.unixguardian.Agent1.Recorder`
pub struct RecorderIface {
    pub recorder: Arc<crate::recorder::Recorder>,
}

impl RecorderIface {
    /// Runs a recorder call off the async threads (they wait for processes).
    async fn blocking<T: Send + 'static>(
        &self,
        f: impl FnOnce(&crate::recorder::Recorder) -> Result<T, String> + Send + 'static,
    ) -> zbus::fdo::Result<T> {
        let r = Arc::clone(&self.recorder);
        tokio::task::spawn_blocking(move || f(&r))
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
            .map_err(zbus::fdo::Error::Failed)
    }
}

#[zbus::interface(name = "org.unixguardian.Agent1.Recorder")]
impl RecorderIface {
    async fn state(&self) -> ug_proto::RecorderState {
        self.blocking(|r| Ok(r.state())).await.unwrap_or_default()
    }

    async fn set(&self, settings: ug_proto::RecorderSettings) -> zbus::fdo::Result<()> {
        self.recorder
            .set(settings)
            .map_err(zbus::fdo::Error::InvalidArgs)
    }

    async fn start(&self) -> zbus::fdo::Result<String> {
        self.blocking(|r| r.start()).await
    }

    async fn stop(&self) -> zbus::fdo::Result<String> {
        self.blocking(|r| r.stop()).await
    }

    async fn start_replay(&self) -> zbus::fdo::Result<()> {
        self.blocking(|r| r.start_replay()).await
    }

    async fn save_replay(&self) -> zbus::fdo::Result<String> {
        self.blocking(|r| r.save_replay()).await
    }

    async fn set_up(&self, on: bool) -> zbus::fdo::Result<()> {
        self.recorder
            .begin_set_up(on)
            .map_err(zbus::fdo::Error::Failed)
    }
}

/// `org.unixguardian.Agent1.Ping`
pub struct PingIface {
    pub pinger: crate::ping::Pinger,
}

#[zbus::interface(name = "org.unixguardian.Agent1.Ping")]
impl PingIface {
    /// Asking keeps the measurement running for another minute.
    async fn recent(&self, since_unix_ms: u64) -> PingReport {
        self.pinger.touch();
        self.pinger.report(since_unix_ms)
    }

    async fn compare(&self, target: &str, mark_unix_ms: u64, window_s: u32) -> PingComparison {
        self.pinger.compare(target, mark_unix_ms, window_s)
    }
}

/// `YYYY-MM-DD` of a Unix-ms time, in local time.
pub fn local_date(ms: u64) -> String {
    // SAFETY: localtime_r writes only into the struct we pass.
    let tm = unsafe {
        let t = (ms / 1000) as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    };
    format!(
        "{:04}-{:02}-{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday
    )
}

/// `org.unixguardian.Agent1.Settings`
pub struct SettingsIface {
    pub settings: Arc<crate::settings::Settings>,
}

#[zbus::interface(name = "org.unixguardian.Agent1.Settings")]
impl SettingsIface {
    async fn state(&self) -> ug_proto::SettingsState {
        ug_proto::SettingsState {
            settings: self.settings.get(),
            agent_version: env!("CARGO_PKG_VERSION").into(),
            config_dir: ug_core::paths::user_config().display().to_string(),
            data_dir: ug_core::paths::user_data().display().to_string(),
            scan_interval_fixed: self.settings.scan_fixed(),
        }
    }

    async fn set(&self, settings: ug_proto::AgentSettings) -> zbus::fdo::Result<()> {
        self.settings
            .set(settings)
            .map_err(zbus::fdo::Error::InvalidArgs)
    }
}

/// `org.unixguardian.Agent1.Update`
pub struct UpdateIface {
    pub updater: Arc<crate::update::Updater>,
    pub daemon_session: bool,
}

impl UpdateIface {
    async fn daemon(&self) -> zbus::fdo::Result<ug_proto::daemon::UpdateProxy<'static>> {
        let conn = if self.daemon_session {
            zbus::Connection::session().await
        } else {
            zbus::Connection::system().await
        }
        .map_err(|e| {
            zbus::fdo::Error::Failed(format!("The Unix Guardian service is not reachable ({e})."))
        })?;
        ug_proto::daemon::UpdateProxy::new(&conn)
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }
}

#[zbus::interface(name = "org.unixguardian.Agent1.Update")]
impl UpdateIface {
    async fn state(&self) -> ug_proto::UpdateState {
        self.updater.state()
    }

    async fn install(&self) -> zbus::fdo::Result<()> {
        self.updater
            .begin_install(self.daemon_session)
            .map_err(zbus::fdo::Error::Failed)
    }

    async fn rollback(&self) -> zbus::fdo::Result<String> {
        self.daemon().await?.rollback().await.map_err(|e| match e {
            zbus::Error::MethodError(_, Some(m), _) => zbus::fdo::Error::Failed(m),
            e => zbus::fdo::Error::Failed(e.to_string()),
        })
    }

    async fn check(&self) -> zbus::fdo::Result<ug_proto::UpdateState> {
        let u = Arc::clone(&self.updater);
        tokio::task::spawn_blocking(move || u.check(now_ms()))
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        // Whether there is a version to go back to, from the service.
        if let Ok(d) = self.daemon().await {
            if let Ok(p) = d.previous().await {
                self.updater.set_previous(p);
            }
        }
        Ok(self.updater.state())
    }
}

/// `org.unixguardian.Agent1.Shortcuts`
pub struct ShortcutsIface {
    pub shortcuts: Arc<crate::shortcuts::Shortcuts>,
    pub core: Shared,
    pub overlay: Arc<crate::overlay::Overlay>,
    pub recorder: Arc<crate::recorder::Recorder>,
}

#[zbus::interface(name = "org.unixguardian.Agent1.Shortcuts")]
impl ShortcutsIface {
    async fn state(&self) -> ug_proto::ShortcutsState {
        let mut s = self.shortcuts.state();
        s.gamemode_by_hand = lock(&self.core).gamemode_by_hand().to_string();
        s
    }

    async fn set_enabled(&self, on: bool) -> zbus::fdo::Result<()> {
        self.shortcuts
            .set_enabled(on)
            .await
            .map_err(zbus::fdo::Error::Failed)
    }

    async fn run(&self, id: &str) -> zbus::fdo::Result<String> {
        crate::shortcuts::perform(id, &self.core, &self.overlay, &self.recorder)
            .await
            .map_err(zbus::fdo::Error::Failed)
    }

    async fn set_keys(&self, id: &str, keys: &str) -> zbus::fdo::Result<String> {
        self.shortcuts
            .set_keys(id, keys)
            .await
            .map_err(zbus::fdo::Error::InvalidArgs)
    }

    async fn desktop_keys(&self) -> ug_proto::DesktopKeys {
        self.shortcuts.desktop_keys()
    }

    async fn set_up_desktop_keys(&self, on: bool) -> zbus::fdo::Result<String> {
        self.shortcuts
            .set_up_desktop_keys(on)
            .await
            .map_err(zbus::fdo::Error::Failed)
    }
}
