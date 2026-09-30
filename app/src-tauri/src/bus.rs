//! The window's only way to the machine: D-Bus calls to the daemon and agent.
//!
//! The front end holds no logic and never touches sysfs; every command here is
//! a thin wrapper over the same interfaces `ug` uses, so the UI can never do
//! something the CLI cannot. Errors cross to the front end as plain sentences
//! it can show as they are.

use futures_lite::StreamExt;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::{Mutex, OnceCell};
use ug_proto::agent::{GamesProxy, MonitorProxy, PingProxy, PlaytimeProxy, ProfilesProxy};
use ug_proto::daemon::{DaemonProxy, GameModeProxy, GpuProxy, NetProxy, PowerProxy};
use ug_proto::{BatteryCare, DayTraffic, HistoryPoint, Inventory, InstalledGameInfo, PlaytimeEntry, PowerCapability, PowerJob, PowerUsage, PowerWatch, ProfileInfo, RunningGameInfo, Snapshot};

pub struct Bus {
    /// Reach the daemon on the session bus (`UG_SESSION=1`), for a development
    /// daemon started with `ug-daemon --session`.
    daemon_on_session: bool,
    daemon: OnceCell<zbus::Connection>,
    agent: OnceCell<zbus::Connection>,
    monitor: Mutex<Option<Monitor>>,
}

/// A live subscription to the daemon's samples. It owns its own connection:
/// the daemon keys subscribers by unique bus name, so dropping the connection
/// is a guaranteed release even if the unsubscribe call never gets through.
struct Monitor {
    _conn: zbus::Connection,
    task: tauri::async_runtime::JoinHandle<()>,
}

type Res<T> = Result<T, String>;

const DAEMON_DOWN: &str = "The Unix Guardian service is not running. Install or start it: \
     sudo systemctl start unix-guardiand";
const AGENT_DOWN: &str = "The Unix Guardian agent is not running in this session. Start it: \
     systemctl --user start ug-agent";

impl Bus {
    pub fn new(daemon_on_session: bool) -> Self {
        Bus {
            daemon_on_session,
            daemon: OnceCell::new(),
            agent: OnceCell::new(),
            monitor: Mutex::new(None),
        }
    }

    async fn daemon_bus(&self) -> zbus::Result<zbus::Connection> {
        if self.daemon_on_session {
            zbus::Connection::session().await
        } else {
            zbus::Connection::system().await
        }
    }

    async fn daemon(&self) -> Res<&zbus::Connection> {
        self.daemon.get_or_try_init(|| self.daemon_bus()).await.map_err(|_| DAEMON_DOWN.to_string())
    }

    async fn agent(&self) -> Res<&zbus::Connection> {
        self.agent
            .get_or_try_init(zbus::Connection::session)
            .await
            .map_err(|_| AGENT_DOWN.to_string())
    }

    /// Ends the sample stream (the page hid, or asked). Closing the window ends
    /// the process, and the daemon drops a subscriber whose connection closes.
    pub async fn stop_monitor(&self) {
        if let Some(m) = self.monitor.lock().await.take() {
            m.task.abort();
            if let Ok(p) = DaemonProxy::new(&m._conn).await {
                let _ = p.unsubscribe().await;
            }
        }
    }
}

/// A method error from the daemon or agent is already a sentence meant for
/// people; a missing service gets the hint for starting it.
fn daemon_err(e: zbus::Error) -> String {
    match e {
        zbus::Error::MethodError(name, _, _) if name.as_str().ends_with("ServiceUnknown") => DAEMON_DOWN.into(),
        zbus::Error::MethodError(_, Some(msg), _) => msg,
        other => other.to_string(),
    }
}

fn agent_err(e: zbus::Error) -> String {
    match e {
        zbus::Error::MethodError(name, _, _) if name.as_str().ends_with("ServiceUnknown") => AGENT_DOWN.into(),
        zbus::Error::MethodError(_, Some(msg), _) => msg,
        other => other.to_string(),
    }
}

#[derive(Serialize)]
pub struct Overview {
    inventory: Option<Inventory>,
    daemon_error: Option<String>,
    daemon_version: Option<String>,
    agent_error: Option<String>,
    gamemode_active: Option<bool>,
    active_gpu: Option<String>,
}

/// Everything the shell needs once, when the window opens: what the machine
/// is, and which of the two services answer.
#[tauri::command]
pub async fn overview(bus: State<'_, Bus>) -> Res<Overview> {
    let mut o = Overview {
        inventory: None,
        daemon_error: None,
        daemon_version: None,
        agent_error: None,
        gamemode_active: None,
        active_gpu: None,
    };
    match bus.daemon().await {
        Ok(conn) => {
            let d = DaemonProxy::new(conn).await.map_err(daemon_err)?;
            match d.inventory().await {
                Ok(inv) => {
                    o.inventory = Some(inv);
                    o.daemon_version = d.version().await.ok();
                    if let Ok(g) = GameModeProxy::new(conn).await {
                        o.gamemode_active = g.active().await.ok();
                    }
                    if let Ok(g) = GpuProxy::new(conn).await {
                        o.active_gpu = g.active_gpu().await.ok();
                    }
                }
                Err(e) => o.daemon_error = Some(daemon_err(e)),
            }
        }
        Err(e) => o.daemon_error = Some(e),
    }
    match bus.agent().await {
        Ok(conn) => {
            let g = GamesProxy::new(conn).await.map_err(agent_err)?;
            if let Err(e) = g.running().await {
                o.agent_error = Some(agent_err(e));
            }
        }
        Err(e) => o.agent_error = Some(e),
    }
    Ok(o)
}

#[tauri::command]
pub async fn snapshot(bus: State<'_, Bus>) -> Res<Snapshot> {
    let d = DaemonProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    d.snapshot().await.map_err(daemon_err)
}

/// Starts streaming `sample` events at `interval_ms`. Idempotent: a second
/// call replaces the first subscription.
#[tauri::command]
pub async fn monitor_start(app: AppHandle, bus: State<'_, Bus>, interval_ms: u32) -> Res<()> {
    bus.stop_monitor().await;
    let conn = bus.daemon_bus().await.map_err(|_| DAEMON_DOWN.to_string())?;
    let d = DaemonProxy::new(&conn).await.map_err(daemon_err)?;
    // Listen before subscribing, or the first sample arrives before the match
    // rule exists and is lost (found in the CLI, 05-findings.md).
    let mut samples = d.receive_sample().await.map_err(daemon_err)?;
    d.subscribe(interval_ms.clamp(250, 10_000)).await.map_err(daemon_err)?;
    let task = tauri::async_runtime::spawn(async move {
        while let Some(sig) = samples.next().await {
            if let Ok(args) = sig.args() {
                if app.emit("sample", &args.snapshot).is_err() {
                    break;
                }
            }
        }
    });
    *bus.monitor.lock().await = Some(Monitor { _conn: conn, task });
    Ok(())
}

#[tauri::command]
pub async fn monitor_stop(bus: State<'_, Bus>) -> Res<()> {
    bus.stop_monitor().await;
    Ok(())
}

#[tauri::command]
pub async fn gamemode_active(bus: State<'_, Bus>) -> Res<bool> {
    let g = GameModeProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    g.active().await.map_err(daemon_err)
}

#[tauri::command]
pub async fn games_running(bus: State<'_, Bus>) -> Res<Vec<RunningGameInfo>> {
    let g = GamesProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    g.running().await.map_err(agent_err)
}

#[tauri::command]
pub async fn games_installed(bus: State<'_, Bus>) -> Res<Vec<InstalledGameInfo>> {
    let g = GamesProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    g.installed().await.map_err(agent_err)
}

#[tauri::command]
pub async fn profiles(bus: State<'_, Bus>) -> Res<Vec<ProfileInfo>> {
    let p = ProfilesProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    p.list().await.map_err(agent_err)
}

/// `profile` empty returns the game to the database's suggestion.
#[tauri::command]
pub async fn set_game_profile(bus: State<'_, Bus>, key: String, profile: String) -> Res<()> {
    let p = ProfilesProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    p.set_game_profile(&key, &profile).await.map_err(agent_err)
}

/// Turns one setting of a profile on or off (`ug profile option`).
#[tauri::command]
pub async fn set_profile_option(bus: State<'_, Bus>, profile: String, option: String, value: bool) -> Res<()> {
    let p = ProfilesProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    p.set_option(&profile, &option, value).await.map_err(agent_err)
}

#[tauri::command]
pub async fn playtime(bus: State<'_, Bus>, since_unix_ms: u64) -> Res<Vec<PlaytimeEntry>> {
    let p = PlaytimeProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    p.summary(since_unix_ms).await.map_err(agent_err)
}

#[tauri::command]
pub async fn power_caps(bus: State<'_, Bus>) -> Res<Vec<PowerCapability>> {
    let p = PowerProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    p.capabilities().await.map_err(daemon_err)
}

#[tauri::command]
pub async fn power_now(bus: State<'_, Bus>, action: String) -> Res<()> {
    let p = PowerProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    p.now(&action).await.map_err(daemon_err)
}

#[tauri::command]
pub async fn power_schedule(bus: State<'_, Bus>, action: String, at_unix_ms: u64, repeat: String) -> Res<PowerJob> {
    let p = PowerProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    p.schedule(&action, at_unix_ms, &repeat, "").await.map_err(daemon_err)
}

#[tauri::command]
pub async fn power_jobs(bus: State<'_, Bus>) -> Res<Vec<PowerJob>> {
    let p = PowerProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    p.jobs().await.map_err(daemon_err)
}

#[tauri::command]
pub async fn power_cancel(bus: State<'_, Bus>, id: u32) -> Res<bool> {
    let p = PowerProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    p.cancel(id).await.map_err(daemon_err)
}

/// The armed wake alarm (Unix ms, 0 = none) and whether the clock supports one.
#[tauri::command]
pub async fn wake_alarm(bus: State<'_, Bus>) -> Res<(u64, bool)> {
    let p = PowerProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    Ok((p.wake_alarm().await.unwrap_or(0), p.wake_alarm_supported().await.unwrap_or(false)))
}

#[tauri::command]
pub async fn set_wake_alarm(bus: State<'_, Bus>, at_unix_ms: u64) -> Res<()> {
    let p = PowerProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    p.set_wake_alarm(at_unix_ms).await.map_err(daemon_err)
}

#[tauri::command]
pub async fn clear_wake_alarm(bus: State<'_, Bus>) -> Res<()> {
    let p = PowerProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    p.clear_wake_alarm().await.map_err(daemon_err)
}

#[tauri::command]
pub async fn power_when(
    bus: State<'_, Bus>,
    action: String,
    kind: String,
    threshold: f64,
    minutes: u32,
    name: String,
) -> Res<PowerWatch> {
    let p = PowerProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    p.when(&action, &kind, threshold, minutes, &name, "").await.map_err(daemon_err)
}

#[tauri::command]
pub async fn power_watches(bus: State<'_, Bus>) -> Res<Vec<PowerWatch>> {
    let p = PowerProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    p.watches().await.map_err(daemon_err)
}

#[tauri::command]
pub async fn power_unwatch(bus: State<'_, Bus>, id: u32) -> Res<bool> {
    let p = PowerProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    p.cancel_watch(id).await.map_err(daemon_err)
}

#[tauri::command]
pub async fn power_usage(bus: State<'_, Bus>) -> Res<PowerUsage> {
    let p = PowerProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    p.usage().await.map_err(daemon_err)
}

#[tauri::command]
pub async fn battery(bus: State<'_, Bus>) -> Res<BatteryCare> {
    let p = PowerProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    p.battery().await.map_err(daemon_err)
}

#[tauri::command]
pub async fn set_battery_care(bus: State<'_, Bus>, mode: String, limit: u8) -> Res<()> {
    let p = PowerProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    p.set_battery_care(&mode, limit).await.map_err(daemon_err)
}

#[tauri::command]
pub async fn history(bus: State<'_, Bus>, since_unix_ms: u64, until_unix_ms: u64, points: u32) -> Res<Vec<HistoryPoint>> {
    let m = MonitorProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    m.history(since_unix_ms, until_unix_ms, points).await.map_err(agent_err)
}

#[tauri::command]
pub async fn extra(bus: State<'_, Bus>) -> Res<ug_proto::ExtraSample> {
    let d = DaemonProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    d.extra().await.map_err(daemon_err)
}

#[tauri::command]
pub async fn fps(bus: State<'_, Bus>) -> Res<ug_proto::FpsReading> {
    let m = MonitorProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    m.fps().await.map_err(agent_err)
}

#[tauri::command]
pub async fn set_up_fps(bus: State<'_, Bus>, on: bool) -> Res<()> {
    let m = MonitorProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    m.set_up_fps(on).await.map_err(agent_err)
}

#[tauri::command]
pub async fn ping_recent(bus: State<'_, Bus>, since_unix_ms: u64) -> Res<ug_proto::PingReport> {
    let p = PingProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    p.recent(since_unix_ms).await.map_err(agent_err)
}

#[tauri::command]
pub async fn ping_compare(bus: State<'_, Bus>, target: String, mark_unix_ms: u64, window_s: u32) -> Res<ug_proto::PingComparison> {
    let p = PingProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    p.compare(&target, mark_unix_ms, window_s).await.map_err(agent_err)
}

#[tauri::command]
pub async fn net_status(bus: State<'_, Bus>) -> Res<ug_proto::NetControlStatus> {
    let n = NetProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    n.status().await.map_err(daemon_err)
}

#[tauri::command]
pub async fn net_apps(bus: State<'_, Bus>) -> Res<Vec<ug_proto::AppTraffic>> {
    let n = NetProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    n.apps().await.map_err(daemon_err)
}

#[tauri::command]
pub async fn net_rules(bus: State<'_, Bus>) -> Res<Vec<ug_proto::NetRule>> {
    let n = NetProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    n.rules().await.map_err(daemon_err)
}

#[tauri::command]
pub async fn net_set_rule(bus: State<'_, Bus>, app: String, block: bool, up_kbps: u32, down_kbps: u32, when: String) -> Res<()> {
    let n = NetProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    n.set_rule(&app, block, up_kbps, down_kbps, &when).await.map_err(daemon_err)
}

#[tauri::command]
pub async fn net_remove_rule(bus: State<'_, Bus>, app: String) -> Res<bool> {
    let n = NetProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    n.remove_rule(&app).await.map_err(daemon_err)
}

#[tauri::command]
pub async fn traffic(bus: State<'_, Bus>, since_unix_ms: u64) -> Res<Vec<DayTraffic>> {
    let m = MonitorProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    m.traffic(since_unix_ms).await.map_err(agent_err)
}
