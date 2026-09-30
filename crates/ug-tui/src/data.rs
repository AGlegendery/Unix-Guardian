//! What the terminal interface shows, fetched over the same D-Bus interfaces
//! as the window and the CLI. Each piece has its own refresh rate, and only
//! the page on screen refreshes: an idle TUI on an unchanging page does no
//! work.

use std::time::{Duration, Instant};
use ug_proto::agent::{GamesProxy, MonitorProxy, PingProxy, ProfilesProxy};
use ug_proto::daemon::{DaemonProxy, GameModeProxy, NetProxy, PowerProxy};
use ug_proto::{
    AppTraffic, BatteryCare, DayTraffic, ExtraSample, NetControlStatus, NetRule, PingComparison, PingReport, FpsReading, HistoryPoint, InstalledGameInfo, Inventory, PowerCapability, PowerJob, PowerWatch,
    ProfileInfo, RunningGameInfo, Snapshot,
};

pub struct Bus {
    rt: tokio::runtime::Runtime,
    daemon: Option<zbus::Connection>,
    agent: Option<zbus::Connection>,
}

/// The daemon's or agent's sentence when it refused; a hint when it is gone.
fn msg(e: zbus::Error, who: &str) -> String {
    match e {
        zbus::Error::MethodError(name, _, _) if name.as_str().ends_with("ServiceUnknown") => {
            format!("The Unix Guardian {who} is not running.")
        }
        zbus::Error::MethodError(_, Some(m), _) => m,
        other => other.to_string(),
    }
}

impl Bus {
    pub fn connect(daemon_session: bool) -> anyhow::Result<Bus> {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
        let (daemon, agent) = rt.block_on(async {
            let d = if daemon_session { zbus::Connection::session().await } else { zbus::Connection::system().await };
            (d.ok(), zbus::Connection::session().await.ok())
        });
        Ok(Bus { rt, daemon, agent })
    }

    /// No buses at all: every call reports that it cannot reach them.
    #[cfg(test)]
    pub fn offline() -> Bus {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        Bus { rt, daemon: None, agent: None }
    }

    fn daemon(&self) -> Result<&zbus::Connection, String> {
        self.daemon.as_ref().ok_or_else(|| "Cannot reach the system bus.".into())
    }

    fn agent(&self) -> Result<&zbus::Connection, String> {
        self.agent.as_ref().ok_or_else(|| "Cannot reach the session bus.".into())
    }

    pub fn inventory(&self) -> Result<Inventory, String> {
        let c = self.daemon()?;
        self.rt.block_on(async { DaemonProxy::new(c).await?.inventory().await }).map_err(|e| msg(e, "service"))
    }

    pub fn snapshot(&self) -> Result<Snapshot, String> {
        let c = self.daemon()?;
        self.rt.block_on(async { DaemonProxy::new(c).await?.snapshot().await }).map_err(|e| msg(e, "service"))
    }

    pub fn extra(&self) -> Result<ExtraSample, String> {
        let c = self.daemon()?;
        self.rt.block_on(async { DaemonProxy::new(c).await?.extra().await }).map_err(|e| msg(e, "service"))
    }

    pub fn fps(&self) -> Result<FpsReading, String> {
        let c = self.agent()?;
        self.rt.block_on(async { MonitorProxy::new(c).await?.fps().await }).map_err(|e| msg(e, "agent"))
    }

    pub fn set_up_fps(&self, on: bool) -> Result<(), String> {
        let c = self.agent()?;
        self.rt.block_on(async { MonitorProxy::new(c).await?.set_up_fps(on).await }).map_err(|e| msg(e, "agent"))
    }

    pub fn ping(&self, since_ms: u64) -> Result<PingReport, String> {
        let c = self.agent()?;
        self.rt.block_on(async { PingProxy::new(c).await?.recent(since_ms).await }).map_err(|e| msg(e, "agent"))
    }

    pub fn ping_compare(&self, target: &str, mark_ms: u64) -> Result<PingComparison, String> {
        let c = self.agent()?;
        self.rt.block_on(async { PingProxy::new(c).await?.compare(target, mark_ms, 60).await }).map_err(|e| msg(e, "agent"))
    }

    /// Apps with their traffic, the rules, and whether control works here.
    pub fn net(&self) -> Result<(Vec<AppTraffic>, Vec<NetRule>, NetControlStatus), String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async {
                let n = NetProxy::new(c).await?;
                Ok::<_, zbus::Error>((n.apps().await?, n.rules().await?, n.status().await?))
            })
            .map_err(|e| msg(e, "service"))
    }

    pub fn net_set_rule(&self, app: &str, block: bool, up: u32, down: u32, when: &str) -> Result<(), String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async { NetProxy::new(c).await?.set_rule(app, block, up, down, when).await })
            .map_err(|e| msg(e, "service"))
    }

    pub fn net_remove_rule(&self, app: &str) -> Result<bool, String> {
        let c = self.daemon()?;
        self.rt.block_on(async { NetProxy::new(c).await?.remove_rule(app).await }).map_err(|e| msg(e, "service"))
    }

    pub fn gamemode_active(&self) -> Result<bool, String> {
        let c = self.daemon()?;
        self.rt.block_on(async { GameModeProxy::new(c).await?.active().await }).map_err(|e| msg(e, "service"))
    }

    pub fn running(&self) -> Result<Vec<RunningGameInfo>, String> {
        let c = self.agent()?;
        self.rt.block_on(async { GamesProxy::new(c).await?.running().await }).map_err(|e| msg(e, "agent"))
    }

    pub fn installed(&self) -> Result<Vec<InstalledGameInfo>, String> {
        let c = self.agent()?;
        self.rt.block_on(async { GamesProxy::new(c).await?.installed().await }).map_err(|e| msg(e, "agent"))
    }

    pub fn profiles(&self) -> Result<Vec<ProfileInfo>, String> {
        let c = self.agent()?;
        self.rt.block_on(async { ProfilesProxy::new(c).await?.list().await }).map_err(|e| msg(e, "agent"))
    }

    pub fn set_option(&self, profile: &str, option: &str, value: bool) -> Result<(), String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { ProfilesProxy::new(c).await?.set_option(profile, option, value).await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn set_game_profile(&self, key: &str, profile: &str) -> Result<(), String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { ProfilesProxy::new(c).await?.set_game_profile(key, profile).await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn history(&self, since_ms: u64, until_ms: u64, points: u32) -> Result<Vec<HistoryPoint>, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { MonitorProxy::new(c).await?.history(since_ms, until_ms, points).await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn traffic(&self, since_ms: u64) -> Result<Vec<DayTraffic>, String> {
        let c = self.agent()?;
        self.rt.block_on(async { MonitorProxy::new(c).await?.traffic(since_ms).await }).map_err(|e| msg(e, "agent"))
    }

    pub fn power_caps(&self) -> Result<Vec<PowerCapability>, String> {
        let c = self.daemon()?;
        self.rt.block_on(async { PowerProxy::new(c).await?.capabilities().await }).map_err(|e| msg(e, "service"))
    }

    pub fn power_jobs(&self) -> Result<(Vec<PowerJob>, Vec<PowerWatch>), String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async {
                let p = PowerProxy::new(c).await?;
                Ok::<_, zbus::Error>((p.jobs().await?, p.watches().await?))
            })
            .map_err(|e| msg(e, "service"))
    }

    pub fn power_now(&self, action: &str) -> Result<(), String> {
        let c = self.daemon()?;
        self.rt.block_on(async { PowerProxy::new(c).await?.now(action).await }).map_err(|e| msg(e, "service"))
    }

    pub fn power_schedule(&self, action: &str, at_ms: u64, repeat: &str) -> Result<PowerJob, String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async { PowerProxy::new(c).await?.schedule(action, at_ms, repeat, "").await })
            .map_err(|e| msg(e, "service"))
    }

    pub fn power_cancel(&self, id: u32) -> Result<bool, String> {
        let c = self.daemon()?;
        self.rt.block_on(async { PowerProxy::new(c).await?.cancel(id).await }).map_err(|e| msg(e, "service"))
    }

    pub fn power_unwatch(&self, id: u32) -> Result<bool, String> {
        let c = self.daemon()?;
        self.rt.block_on(async { PowerProxy::new(c).await?.cancel_watch(id).await }).map_err(|e| msg(e, "service"))
    }

    pub fn battery(&self) -> Result<BatteryCare, String> {
        let c = self.daemon()?;
        self.rt.block_on(async { PowerProxy::new(c).await?.battery().await }).map_err(|e| msg(e, "service"))
    }

    pub fn set_battery(&self, mode: &str, limit: u8) -> Result<(), String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async { PowerProxy::new(c).await?.set_battery_care(mode, limit).await })
            .map_err(|e| msg(e, "service"))
    }
}

/// A value refreshed on its own schedule.
pub struct Cached<T> {
    pub value: Option<T>,
    pub error: Option<String>,
    at: Option<Instant>,
    every: Duration,
}

impl<T> Cached<T> {
    pub fn new(every: Duration) -> Self {
        Cached { value: None, error: None, at: None, every }
    }

    pub fn due(&self) -> bool {
        self.at.is_none_or(|t| t.elapsed() >= self.every)
    }

    pub fn store(&mut self, r: Result<T, String>) {
        self.at = Some(Instant::now());
        match r {
            Ok(v) => {
                self.value = Some(v);
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Forces the next `due()`, after a change the user made.
    pub fn invalidate(&mut self) {
        self.at = None;
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `Wed 21:30` in local time.
pub fn local_time(ms: u64) -> String {
    // SAFETY: localtime_r and strftime only touch the buffers we pass.
    unsafe {
        let t = (ms / 1000) as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        let mut buf = [0u8; 32];
        let n = libc::strftime(buf.as_mut_ptr() as *mut libc::c_char, buf.len(), c"%a %H:%M".as_ptr(), &tm);
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }
}
