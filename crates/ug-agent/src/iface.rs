//! The agent's D-Bus interfaces on the session bus.

use crate::core::AgentCore;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use ug_proto::{
    DayTraffic, HistoryPoint, InstalledGameInfo, PingComparison, PingReport, PlaytimeEntry, PlaytimeSession, ProfileInfo,
    RunningGameInfo,
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
        tokio::task::spawn_blocking(move || lock(&core).rescan() as u32).await.unwrap_or(0)
    }

    async fn ignore_executable(&self, name: &str) -> zbus::fdo::Result<()> {
        if name.trim().is_empty() {
            return Err(zbus::fdo::Error::InvalidArgs("an executable name is required".into()));
        }
        let outcomes = lock(&self.core).ignore(name, Instant::now(), now_ms());
        for o in outcomes {
            let _ = self.announce.send(o);
        }
        Ok(())
    }

    async fn launch_options(&self, appid: u32) -> zbus::fdo::Result<ug_proto::LaunchInfo> {
        let (launch, uid, name) = lock(&self.core).steam_launch(appid).map_err(zbus::fdo::Error::Failed)?;
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

    async fn preview_launch_options(&self, appid: u32, gpu: &str, overlay: bool) -> zbus::fdo::Result<String> {
        let (launch, _, _) = lock(&self.core).steam_launch(appid).map_err(zbus::fdo::Error::Failed)?;
        let add = ug_games::launch::Additions { gpu: gpu.to_string(), overlay };
        launch.preview(appid, &add).map_err(zbus::fdo::Error::Failed)
    }

    async fn set_launch_options(&self, appid: u32, gpu: &str, overlay: bool) -> zbus::fdo::Result<(String, String)> {
        let (launch, uid, _) = lock(&self.core).steam_launch(appid).map_err(zbus::fdo::Error::Failed)?;
        let add = ug_games::launch::Additions { gpu: gpu.to_string(), overlay };
        let a = tokio::task::spawn_blocking(move || launch.apply(appid, &add, uid))
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
            .map_err(zbus::fdo::Error::Failed)?;
        Ok((a.options, a.backup.display().to_string()))
    }

    async fn undo_launch_options(&self, appid: u32) -> zbus::fdo::Result<(String, String)> {
        let (launch, uid, _) = lock(&self.core).steam_launch(appid).map_err(zbus::fdo::Error::Failed)?;
        let a = tokio::task::spawn_blocking(move || launch.undo(appid, uid))
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
            .map_err(zbus::fdo::Error::Failed)?;
        Ok((a.options, a.backup.display().to_string()))
    }

    #[zbus(signal)]
    pub async fn game_started(emitter: &SignalEmitter<'_>, game: RunningGameInfo) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn game_stopped(emitter: &SignalEmitter<'_>, game: RunningGameInfo, played_s: u64) -> zbus::Result<()>;
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
        lock(&self.core).track_app(exe, name).map_err(zbus::fdo::Error::InvalidArgs)
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
        let (moved, outcomes) =
            lock(&self.core).move_to_apps(key, Instant::now(), now_ms()).map_err(zbus::fdo::Error::InvalidArgs)?;
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
        lock(&self.core).set_game_profile(key, profile).map_err(zbus::fdo::Error::InvalidArgs)
    }

    async fn set_option(&self, profile: &str, option: &str, value: bool) -> zbus::fdo::Result<()> {
        lock(&self.core).set_profile_option(profile, option, value).map_err(zbus::fdo::Error::InvalidArgs)
    }
}

/// `org.unixguardian.Agent1.Monitor`
pub struct MonitorIface {
    pub history: Arc<crate::history::History>,
    pub overlay: Arc<crate::overlay::Overlay>,
}

#[zbus::interface(name = "org.unixguardian.Agent1.Monitor")]
impl MonitorIface {
    async fn history(&self, since_unix_ms: u64, until_unix_ms: u64, points: u32) -> Vec<HistoryPoint> {
        let h = Arc::clone(&self.history);
        tokio::task::spawn_blocking(move || h.range(since_unix_ms, until_unix_ms, points)).await.unwrap_or_default()
    }

    async fn traffic(&self, since_unix_ms: u64) -> Vec<DayTraffic> {
        let h = Arc::clone(&self.history);
        tokio::task::spawn_blocking(move || h.traffic(since_unix_ms, local_date)).await.unwrap_or_default()
    }

    /// The running game's FPS from MangoHud's log.
    async fn fps(&self) -> ug_proto::FpsReading {
        let o = Arc::clone(&self.overlay);
        tokio::task::spawn_blocking(move || o.fps.read(std::time::SystemTime::now())).await.unwrap_or_default()
    }

    async fn fps_sessions(&self, since_unix_ms: u64) -> Vec<ug_proto::FpsSession> {
        let h = Arc::clone(&self.history);
        tokio::task::spawn_blocking(move || h.fps_sessions(since_unix_ms)).await.unwrap_or_default()
    }

    async fn forget_fps_sessions(&self) -> u32 {
        let h = Arc::clone(&self.history);
        tokio::task::spawn_blocking(move || h.forget_fps()).await.unwrap_or(0)
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
        tokio::task::spawn_blocking(move || o.state()).await.unwrap_or_default()
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
    format!("{:04}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday)
}
