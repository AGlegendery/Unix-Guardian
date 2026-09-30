//! The agent's D-Bus interfaces on the session bus.

use crate::core::AgentCore;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use ug_proto::{DayTraffic, HistoryPoint, InstalledGameInfo, PlaytimeEntry, PlaytimeSession, ProfileInfo, RunningGameInfo};
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

    #[zbus(signal)]
    pub async fn game_started(emitter: &SignalEmitter<'_>, game: RunningGameInfo) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn game_stopped(emitter: &SignalEmitter<'_>, game: RunningGameInfo, played_s: u64) -> zbus::Result<()>;
}

/// `org.unixguardian.Agent1.Playtime`
pub struct PlaytimeIface {
    pub core: Shared,
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
