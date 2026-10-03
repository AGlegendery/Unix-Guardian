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
use ug_proto::agent::{GamesProxy, MonitorProxy, OverlayProxy, PingProxy, PlaytimeProxy, ProfilesProxy, SettingsProxy, ShortcutsProxy};
use ug_proto::daemon::{CoolingProxy, DaemonProxy, GameModeProxy, GpuProxy, NetProxy, PowerProxy};
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
pub async fn playtime_sessions(bus: State<'_, Bus>, since_unix_ms: u64) -> Res<Vec<ug_proto::PlaytimeSession>> {
    let p = PlaytimeProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    p.sessions(since_unix_ms).await.map_err(agent_err)
}

/// Deletes every recorded session of one game; returns how many.
#[tauri::command]
pub async fn playtime_forget(bus: State<'_, Bus>, key: String) -> Res<u32> {
    let p = PlaytimeProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    p.forget(&key).await.map_err(agent_err)
}

#[tauri::command]
pub async fn steam_launch_options(bus: State<'_, Bus>, appid: u32) -> Res<ug_proto::LaunchInfo> {
    let g = GamesProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    g.launch_options(appid).await.map_err(agent_err)
}

#[tauri::command]
pub async fn steam_preview_launch_options(bus: State<'_, Bus>, appid: u32, gpu: String, overlay: bool) -> Res<String> {
    let g = GamesProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    g.preview_launch_options(appid, &gpu, overlay).await.map_err(agent_err)
}

/// Returns (options, backup path).
#[tauri::command]
pub async fn steam_set_launch_options(bus: State<'_, Bus>, appid: u32, gpu: String, overlay: bool) -> Res<(String, String)> {
    let g = GamesProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    g.set_launch_options(appid, &gpu, overlay).await.map_err(agent_err)
}

#[tauri::command]
pub async fn steam_undo_launch_options(bus: State<'_, Bus>, appid: u32) -> Res<(String, String)> {
    let g = GamesProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    g.undo_launch_options(appid).await.map_err(agent_err)
}

#[tauri::command]
pub async fn playtime_tracked_apps(bus: State<'_, Bus>) -> Res<Vec<ug_proto::TrackedApp>> {
    let p = PlaytimeProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    p.tracked_apps().await.map_err(agent_err)
}

#[tauri::command]
pub async fn playtime_track_app(bus: State<'_, Bus>, exe: String, name: String) -> Res<()> {
    let p = PlaytimeProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    p.track_app(&exe, &name).await.map_err(agent_err)
}

#[tauri::command]
pub async fn playtime_untrack_app(bus: State<'_, Bus>, exe: String) -> Res<bool> {
    let p = PlaytimeProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    p.untrack_app(&exe).await.map_err(agent_err)
}

#[tauri::command]
pub async fn playtime_running_programs(bus: State<'_, Bus>) -> Res<Vec<ug_proto::TrackedApp>> {
    let p = PlaytimeProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    p.running_programs().await.map_err(agent_err)
}

#[tauri::command]
pub async fn playtime_move_to_apps(bus: State<'_, Bus>, key: String) -> Res<u32> {
    let p = PlaytimeProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    p.move_to_apps(&key).await.map_err(agent_err)
}

/// Returns (games, seconds added).
#[tauri::command]
pub async fn playtime_import_steam(bus: State<'_, Bus>) -> Res<(u32, u64)> {
    let p = PlaytimeProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    p.import_steam().await.map_err(agent_err)
}

#[tauri::command]
pub async fn playtime_imported(bus: State<'_, Bus>) -> Res<Vec<PlaytimeEntry>> {
    let p = PlaytimeProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    p.imported().await.map_err(agent_err)
}

#[tauri::command]
pub async fn playtime_forget_imported(bus: State<'_, Bus>) -> Res<u32> {
    let p = PlaytimeProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    p.forget_imported().await.map_err(agent_err)
}

/// Writes the sessions since `since_unix_ms` as CSV into the user's
/// Downloads folder and returns the file's path. A file, not a save dialog:
/// the window has no dialog plugin, and the path is shown to the user.
#[tauri::command]
pub async fn playtime_export(bus: State<'_, Bus>, since_unix_ms: u64) -> Res<String> {
    let p = PlaytimeProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    let sessions = p.sessions(since_unix_ms).await.map_err(agent_err)?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let stamp = ug_core::playtime::local_time(now).replace([' ', ':'], "-");
    let path = ug_core::playtime::downloads_dir().join(format!("unix-guardian-playtime-{stamp}.csv"));
    std::fs::write(&path, ug_core::playtime::csv(&sessions)).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path.display().to_string())
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
pub async fn overlay_state(bus: State<'_, Bus>) -> Res<ug_proto::OverlayState> {
    let o = OverlayProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    o.state().await.map_err(agent_err)
}

#[tauri::command]
pub async fn overlay_set(bus: State<'_, Bus>, settings: ug_proto::OverlaySettings) -> Res<()> {
    let o = OverlayProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    o.set(&settings).await.map_err(agent_err)
}

#[tauri::command]
pub async fn overlay_set_up(bus: State<'_, Bus>, on: bool) -> Res<()> {
    let o = OverlayProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    o.set_up(on).await.map_err(agent_err)
}

#[tauri::command]
pub async fn shortcuts_state(bus: State<'_, Bus>) -> Res<ug_proto::ShortcutsState> {
    let s = ShortcutsProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    s.state().await.map_err(agent_err)
}

#[tauri::command]
pub async fn shortcuts_set_enabled(bus: State<'_, Bus>, on: bool) -> Res<()> {
    let s = ShortcutsProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    s.set_enabled(on).await.map_err(agent_err)
}

#[tauri::command]
pub async fn shortcuts_run(bus: State<'_, Bus>, id: String) -> Res<String> {
    let s = ShortcutsProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    s.run(&id).await.map_err(agent_err)
}

#[tauri::command]
pub async fn shortcuts_set_keys(bus: State<'_, Bus>, id: String, keys: String) -> Res<String> {
    let s = ShortcutsProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    s.set_keys(&id, &keys).await.map_err(agent_err)
}

#[tauri::command]
pub async fn fps_sessions(bus: State<'_, Bus>, since_unix_ms: u64) -> Res<Vec<ug_proto::FpsSession>> {
    let m = MonitorProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    m.fps_sessions(since_unix_ms).await.map_err(agent_err)
}

#[tauri::command]
pub async fn forget_fps_sessions(bus: State<'_, Bus>) -> Res<u32> {
    let m = MonitorProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    m.forget_fps_sessions().await.map_err(agent_err)
}

#[tauri::command]
pub async fn gpu_tune_state(bus: State<'_, Bus>, pci: String) -> Res<ug_proto::GpuTuneState> {
    let g = GpuProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    g.tune_state(&pci).await.map_err(daemon_err)
}

#[tauri::command]
pub async fn gpu_try_tune(bus: State<'_, Bus>, pci: String, tune: ug_proto::GpuTune) -> Res<()> {
    let g = GpuProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    g.try_tune(&pci, &tune).await.map_err(daemon_err)
}

#[tauri::command]
pub async fn gpu_test_tune(bus: State<'_, Bus>, pci: String, seconds: u32) -> Res<(bool, String)> {
    let g = GpuProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    g.test_tune(&pci, seconds).await.map_err(daemon_err)
}

#[tauri::command]
pub async fn gpu_auto_tune(bus: State<'_, Bus>, pci: String, goal: String) -> Res<()> {
    let g = GpuProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    g.auto_tune(&pci, &goal).await.map_err(daemon_err)
}

#[tauri::command]
pub async fn gpu_auto_tune_status(bus: State<'_, Bus>) -> Res<ug_proto::AutoTuneStatus> {
    let g = GpuProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    g.auto_tune_status().await.map_err(daemon_err)
}

#[tauri::command]
pub async fn gpu_stop_auto_tune(bus: State<'_, Bus>) -> Res<()> {
    let g = GpuProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    g.stop_auto_tune().await.map_err(daemon_err)
}

#[tauri::command]
pub async fn gpu_keep_tune(bus: State<'_, Bus>, pci: String) -> Res<()> {
    let g = GpuProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    g.keep_tune(&pci).await.map_err(daemon_err)
}

#[tauri::command]
pub async fn gpu_cancel_tune(bus: State<'_, Bus>, pci: String) -> Res<()> {
    let g = GpuProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    g.cancel_tune(&pci).await.map_err(daemon_err)
}

#[tauri::command]
pub async fn gpu_reset_tune(bus: State<'_, Bus>, pci: String) -> Res<()> {
    let g = GpuProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    g.reset_tune(&pci).await.map_err(daemon_err)
}

#[tauri::command]
pub async fn cooling_state(bus: State<'_, Bus>) -> Res<ug_proto::CoolingState> {
    let c = CoolingProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    c.state().await.map_err(daemon_err)
}

#[tauri::command]
pub async fn set_fan_curve(bus: State<'_, Bus>, id: String, points: Vec<(f64, f64)>) -> Res<()> {
    let c = CoolingProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    c.set_curve(&id, &points).await.map_err(daemon_err)
}

#[tauri::command]
pub async fn set_fan_auto(bus: State<'_, Bus>, id: String) -> Res<()> {
    let c = CoolingProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    c.set_auto(&id).await.map_err(daemon_err)
}

#[tauri::command]
pub async fn set_fan_mode(bus: State<'_, Bus>, mode: i32) -> Res<()> {
    let c = CoolingProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    c.set_fan_mode(mode).await.map_err(daemon_err)
}

#[zbus::proxy(
    interface = "org.freedesktop.UPower.PowerProfiles",
    default_service = "org.freedesktop.UPower.PowerProfiles",
    default_path = "/org/freedesktop/UPower/PowerProfiles"
)]
trait PowerProfiles {
    #[zbus(property)]
    fn active_profile(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_active_profile(&self, p: &str) -> zbus::Result<()>;
}

/// The system's power profile (power-profiles-daemon), "" when it is not
/// running.
#[tauri::command]
pub async fn power_profile() -> Res<String> {
    let Ok(c) = zbus::Connection::system().await else { return Ok(String::new()) };
    let Ok(p) = PowerProfilesProxy::new(&c).await else { return Ok(String::new()) };
    Ok(p.active_profile().await.unwrap_or_default())
}

#[tauri::command]
pub async fn set_power_profile(profile: String) -> Res<()> {
    let c = zbus::Connection::system().await.map_err(|e| e.to_string())?;
    let p = PowerProfilesProxy::new(&c).await.map_err(|e| e.to_string())?;
    p.set_active_profile(&profile).await.map_err(|e| e.to_string())
}

/// How to run a program on each GPU: the Steam launch options and the
/// `ug run` form. Read from sysfs here; nothing asks the GPUs anything.
#[derive(Serialize)]
pub struct GpuOffload {
    pci: String,
    vendor: String,
    boot_vga: bool,
    launch_options: String,
}

#[tauri::command]
pub fn gpu_offload() -> Vec<GpuOffload> {
    let root = ug_hw::SysRoot::real();
    ug_hw::gpu_use::gpus(&root)
        .iter()
        .map(|g| GpuOffload {
            pci: g.pci.clone(),
            vendor: g.vendor_name().into(),
            boot_vga: g.boot_vga,
            launch_options: ug_hw::gpu_use::launch_options(&root, g),
        })
        .collect()
}

/// The dashboard's card switch: the GPU programs start on (see
/// `ug_core::gpu_default`). Per-user files only, so neither the agent nor
/// the daemon is involved.
#[derive(Serialize)]
pub struct GpuDefault {
    chosen: String,
    active: String,
    pending: bool,
}

#[tauri::command]
pub fn gpu_default() -> GpuDefault {
    let gpus = ug_hw::gpu_use::gpus(&ug_hw::SysRoot::real());
    let st = ug_core::gpu_default::state(&ug_core::gpu_default::config_home(), &gpus);
    GpuDefault { chosen: st.chosen, active: st.active, pending: st.pending }
}

#[tauri::command]
pub fn set_gpu_default(pci: String) -> Res<GpuDefault> {
    let gpus = ug_hw::gpu_use::gpus(&ug_hw::SysRoot::real());
    let gpu = gpus.iter().find(|g| g.pci == pci).ok_or_else(|| format!("no GPU at {pci}"))?;
    ug_core::gpu_default::set(&ug_core::gpu_default::config_home(), gpu).map_err(|e| e.to_string())?;
    Ok(gpu_default())
}

/// Whether the agent shows its tray icon; `None` when there is no agent
/// or it runs without a tray.
#[tauri::command]
pub async fn tray_visible(bus: State<'_, Bus>) -> Res<Option<bool>> {
    let Ok(conn) = bus.agent().await else { return Ok(None) };
    let Ok(t) = ug_proto::agent::TrayProxy::new(conn).await else { return Ok(None) };
    Ok(t.visible().await.ok())
}

#[tauri::command]
pub async fn tray_show(bus: State<'_, Bus>) -> Res<()> {
    let t = ug_proto::agent::TrayProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    t.show().await.map_err(agent_err)
}

/// Reads files only, in this process, so it works with the service down.
#[tauri::command]
pub async fn driver_diagnose() -> Res<ug_proto::DriverReport> {
    tokio::task::spawn_blocking(|| ug_driver::diagnose(&ug_hw::SysRoot::real(), &ug_driver::Env::from_process()))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn driver_plan(bus: State<'_, Bus>, fix_id: String) -> Res<ug_proto::DriverPlan> {
    let d = ug_proto::daemon::DriverProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    d.plan(&fix_id).await.map_err(daemon_err)
}

#[tauri::command]
pub async fn driver_apply(bus: State<'_, Bus>, fix_id: String) -> Res<String> {
    let d = ug_proto::daemon::DriverProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    d.apply(&fix_id).await.map_err(daemon_err)
}

#[tauri::command]
pub async fn driver_ops(bus: State<'_, Bus>) -> Res<Vec<ug_proto::DriverOp>> {
    let d = ug_proto::daemon::DriverProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    d.ops().await.map_err(daemon_err)
}

#[tauri::command]
pub async fn driver_rollback(bus: State<'_, Bus>, op_id: String) -> Res<()> {
    let d = ug_proto::daemon::DriverProxy::new(bus.daemon().await?).await.map_err(daemon_err)?;
    d.rollback(&op_id).await.map_err(daemon_err)
}

#[tauri::command]
pub async fn tray_hide(bus: State<'_, Bus>) -> Res<()> {
    let t = ug_proto::agent::TrayProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    t.hide().await.map_err(agent_err)
}

#[tauri::command]
pub async fn settings_state(bus: State<'_, Bus>) -> Res<ug_proto::SettingsState> {
    let s = SettingsProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    s.state().await.map_err(agent_err)
}

#[tauri::command]
pub async fn settings_set(bus: State<'_, Bus>, settings: ug_proto::AgentSettings) -> Res<()> {
    let s = SettingsProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    s.set(&settings).await.map_err(agent_err)
}

#[tauri::command]
pub async fn traffic(bus: State<'_, Bus>, since_unix_ms: u64) -> Res<Vec<DayTraffic>> {
    let m = MonitorProxy::new(bus.agent().await?).await.map_err(agent_err)?;
    m.traffic(since_unix_ms).await.map_err(agent_err)
}
