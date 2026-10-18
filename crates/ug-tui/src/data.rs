//! What the terminal interface shows, fetched over the same D-Bus interfaces
//! as the window and the CLI. Each piece has its own refresh rate, and only
//! the page on screen refreshes: an idle TUI on an unchanging page does no
//! work.

use std::time::{Duration, Instant};
use ug_proto::agent::{
    GamesProxy, MonitorProxy, OverlayProxy, PingProxy, PlaytimeProxy, ProfilesProxy, SettingsProxy,
    ShortcutsProxy, TrayProxy,
};
use ug_proto::daemon::{CoolingProxy, DaemonProxy, GameModeProxy, GpuProxy, NetProxy, PowerProxy};
use ug_proto::{
    AppTraffic, BatteryCare, DayTraffic, ExtraSample, FpsReading, FpsSession, HistoryPoint,
    InstalledGameInfo, Inventory, NetControlStatus, NetRule, OverlaySettings, OverlayState,
    PingComparison, PingReport, PowerCapability, PowerJob, PowerWatch, ProfileInfo,
    RunningGameInfo, Snapshot,
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
            let root = std::path::Path::new("/");
            if who == "agent" {
                ug_core::missing::agent_down(root)
            } else {
                ug_core::missing::daemon_down(root)
            }
        }
        zbus::Error::MethodError(_, Some(m), _) => m,
        other => other.to_string(),
    }
}

impl Bus {
    pub fn connect(daemon_session: bool) -> anyhow::Result<Bus> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let (daemon, agent) = rt.block_on(async {
            let d = if daemon_session {
                zbus::Connection::session().await
            } else {
                zbus::Connection::system().await
            };
            (d.ok(), zbus::Connection::session().await.ok())
        });
        Ok(Bus { rt, daemon, agent })
    }

    /// No buses at all: every call reports that it cannot reach them.
    #[cfg(test)]
    pub fn offline() -> Bus {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        Bus {
            rt,
            daemon: None,
            agent: None,
        }
    }

    fn daemon(&self) -> Result<&zbus::Connection, String> {
        self.daemon
            .as_ref()
            .ok_or_else(|| "Cannot reach the system bus.".into())
    }

    fn agent(&self) -> Result<&zbus::Connection, String> {
        self.agent
            .as_ref()
            .ok_or_else(|| "Cannot reach the session bus.".into())
    }

    pub fn inventory(&self) -> Result<Inventory, String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async { DaemonProxy::new(c).await?.inventory().await })
            .map_err(|e| msg(e, "service"))
    }

    pub fn snapshot(&self) -> Result<Snapshot, String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async { DaemonProxy::new(c).await?.snapshot().await })
            .map_err(|e| msg(e, "service"))
    }

    pub fn extra(&self) -> Result<ExtraSample, String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async { DaemonProxy::new(c).await?.extra().await })
            .map_err(|e| msg(e, "service"))
    }

    pub fn fps(&self) -> Result<FpsReading, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { MonitorProxy::new(c).await?.fps().await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn set_up_fps(&self, on: bool) -> Result<(), String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { MonitorProxy::new(c).await?.set_up_fps(on).await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn ping(&self, since_ms: u64) -> Result<PingReport, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { PingProxy::new(c).await?.recent(since_ms).await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn ping_compare(&self, target: &str, mark_ms: u64) -> Result<PingComparison, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { PingProxy::new(c).await?.compare(target, mark_ms, 60).await })
            .map_err(|e| msg(e, "agent"))
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

    /// "Slow down background apps": (on, up kbit/s, down kbit/s, when).
    pub fn net_gaming_limits(&self) -> Result<bool, String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async { NetProxy::new(c).await?.gaming_limits().await })
            .map_err(|e| msg(e, "service"))
    }

    pub fn set_net_gaming_limits(&self, on: bool) -> Result<(), String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async { NetProxy::new(c).await?.set_gaming_limits(on).await })
            .map_err(|e| msg(e, "service"))
    }

    pub fn net_background(&self) -> Result<(bool, u32, u32, String), String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async { NetProxy::new(c).await?.background().await })
            .map_err(|e| msg(e, "service"))
    }

    pub fn net_set_background(
        &self,
        on: bool,
        up: u32,
        down: u32,
        when: &str,
    ) -> Result<(), String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async {
                NetProxy::new(c)
                    .await?
                    .set_background(on, up, down, when)
                    .await
            })
            .map_err(|e| msg(e, "service"))
    }

    pub fn net_set_rule(
        &self,
        app: &str,
        block: bool,
        up: u32,
        down: u32,
        when: &str,
    ) -> Result<(), String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async {
                NetProxy::new(c)
                    .await?
                    .set_rule(app, block, up, down, when)
                    .await
            })
            .map_err(|e| msg(e, "service"))
    }

    pub fn net_remove_rule(&self, app: &str) -> Result<bool, String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async { NetProxy::new(c).await?.remove_rule(app).await })
            .map_err(|e| msg(e, "service"))
    }

    pub fn overlay(&self) -> Result<OverlayState, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { OverlayProxy::new(c).await?.state().await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn overlay_set(&self, s: &OverlaySettings) -> Result<(), String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { OverlayProxy::new(c).await?.set(s).await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn overlay_set_up(&self, on: bool) -> Result<(), String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { OverlayProxy::new(c).await?.set_up(on).await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn driver_plan(&self, fix_id: &str) -> Result<ug_proto::DriverPlan, String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async {
                ug_proto::daemon::DriverProxy::new(c)
                    .await?
                    .plan(fix_id)
                    .await
            })
            .map_err(|e| msg(e, "service"))
    }

    pub fn driver_apply(&self, fix_id: &str) -> Result<String, String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async {
                ug_proto::daemon::DriverProxy::new(c)
                    .await?
                    .apply(fix_id)
                    .await
            })
            .map_err(|e| msg(e, "service"))
    }

    /// This boot's kernel log lines about firmware, from the service.
    pub fn firmware_log(&self) -> String {
        let Ok(c) = self.daemon() else {
            return String::new();
        };
        self.rt
            .block_on(async {
                ug_proto::daemon::DriverProxy::new(c)
                    .await?
                    .firmware_log()
                    .await
            })
            .unwrap_or_default()
    }

    pub fn driver_ops(&self) -> Result<Vec<ug_proto::DriverOp>, String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async { ug_proto::daemon::DriverProxy::new(c).await?.ops().await })
            .map_err(|e| msg(e, "service"))
    }

    pub fn driver_keep(&self, op_id: &str) -> Result<(), String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async {
                ug_proto::daemon::DriverProxy::new(c)
                    .await?
                    .keep(op_id)
                    .await
            })
            .map_err(|e| msg(e, "service"))
    }

    pub fn driver_rollback(&self, op_id: &str) -> Result<(), String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async {
                ug_proto::daemon::DriverProxy::new(c)
                    .await?
                    .rollback(op_id)
                    .await
            })
            .map_err(|e| msg(e, "service"))
    }

    pub fn alerts(&self) -> Result<ug_proto::AlertsState, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { ug_proto::agent::AlertsProxy::new(c).await?.state().await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn set_alerts(&self, s: &ug_proto::AlertSettings) -> Result<(), String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { ug_proto::agent::AlertsProxy::new(c).await?.set(s).await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn test_alert(&self) -> Result<ug_proto::AlertEvent, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { ug_proto::agent::AlertsProxy::new(c).await?.test().await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn clear_alerts(&self) -> Result<u32, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { ug_proto::agent::AlertsProxy::new(c).await?.clear().await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn settings(&self) -> Result<ug_proto::SettingsState, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { SettingsProxy::new(c).await?.state().await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn settings_set(&self, s: &ug_proto::AgentSettings) -> Result<(), String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { SettingsProxy::new(c).await?.set(s).await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn update_state(&self) -> Result<ug_proto::UpdateState, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { ug_proto::agent::UpdateProxy::new(c).await?.state().await })
            .map_err(|e| msg(e, "agent"))
    }

    /// Looks on GitHub now (waits for the answer).
    pub fn update_check(&self) -> Result<ug_proto::UpdateState, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { ug_proto::agent::UpdateProxy::new(c).await?.check().await })
            .map_err(|e| msg(e, "agent"))
    }

    /// Starts the download and install; `update_state` follows it.
    pub fn update_install(&self) -> Result<(), String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { ug_proto::agent::UpdateProxy::new(c).await?.install().await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn update_rollback(&self) -> Result<String, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { ug_proto::agent::UpdateProxy::new(c).await?.rollback().await })
            .map_err(|e| msg(e, "agent"))
    }

    /// The daemon's version, for the Settings page.
    pub fn daemon_version(&self) -> Result<String, String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async { DaemonProxy::new(c).await?.version().await })
            .map_err(|e| msg(e, "service"))
    }

    pub fn tray_visible(&self) -> Result<bool, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { TrayProxy::new(c).await?.visible().await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn tray_set(&self, on: bool) -> Result<(), String> {
        let c = self.agent()?;
        self.rt
            .block_on(async {
                let t = TrayProxy::new(c).await?;
                if on {
                    t.show().await
                } else {
                    t.hide().await
                }
            })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn shortcuts(&self) -> Result<ug_proto::ShortcutsState, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { ShortcutsProxy::new(c).await?.state().await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn shortcuts_set_enabled(&self, on: bool) -> Result<(), String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { ShortcutsProxy::new(c).await?.set_enabled(on).await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn desktop_keys(&self) -> Result<ug_proto::DesktopKeys, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { ShortcutsProxy::new(c).await?.desktop_keys().await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn set_up_desktop_keys(&self, on: bool) -> Result<String, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { ShortcutsProxy::new(c).await?.set_up_desktop_keys(on).await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn shortcut_run(&self, id: &str) -> Result<String, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { ShortcutsProxy::new(c).await?.run(id).await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn recorder(&self) -> Result<ug_proto::RecorderState, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { ug_proto::agent::RecorderProxy::new(c).await?.state().await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn recorder_set(&self, s: &ug_proto::RecorderSettings) -> Result<(), String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { ug_proto::agent::RecorderProxy::new(c).await?.set(s).await })
            .map_err(|e| msg(e, "agent"))
    }

    /// `start`, `stop`, `replay`, `save` or `setup`; a file where there is one.
    pub fn recorder_do(&self, what: &str) -> Result<String, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async {
                let r = ug_proto::agent::RecorderProxy::new(c).await?;
                match what {
                    "start" => r.start().await,
                    "stop" => r.stop().await,
                    "replay" => r.start_replay().await.map(|()| String::new()),
                    "save" => r.save_replay().await,
                    _ => r.set_up(true).await.map(|()| String::new()),
                }
            })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn shortcut_set_keys(&self, id: &str, keys: &str) -> Result<String, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { ShortcutsProxy::new(c).await?.set_keys(id, keys).await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn fps_sessions(&self, since_ms: u64) -> Result<Vec<FpsSession>, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { MonitorProxy::new(c).await?.fps_sessions(since_ms).await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn cooling(&self) -> Result<ug_proto::CoolingState, String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async { CoolingProxy::new(c).await?.state().await })
            .map_err(|e| msg(e, "service"))
    }

    pub fn set_fan_curve(&self, id: &str, points: &[(f64, f64)]) -> Result<(), String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async { CoolingProxy::new(c).await?.set_curve(id, points).await })
            .map_err(|e| msg(e, "service"))
    }

    pub fn set_fan_auto(&self, id: &str) -> Result<(), String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async { CoolingProxy::new(c).await?.set_auto(id).await })
            .map_err(|e| msg(e, "service"))
    }

    pub fn set_fan_mode(&self, mode: i32) -> Result<(), String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async { CoolingProxy::new(c).await?.set_fan_mode(mode).await })
            .map_err(|e| msg(e, "service"))
    }

    /// The system power profile (power-profiles-daemon); empty when absent.
    pub fn power_profile(&self) -> String {
        self.rt
            .block_on(async {
                let c = zbus::Connection::system().await.ok()?;
                let p = zbus::Proxy::new(
                    &c,
                    "org.freedesktop.UPower.PowerProfiles",
                    "/org/freedesktop/UPower/PowerProfiles",
                    "org.freedesktop.UPower.PowerProfiles",
                )
                .await
                .ok()?;
                p.get_property::<String>("ActiveProfile").await.ok()
            })
            .unwrap_or_default()
    }

    pub fn set_power_profile(&self, profile: &str) -> Result<(), String> {
        self.rt.block_on(async {
            let c = zbus::Connection::system()
                .await
                .map_err(|e| e.to_string())?;
            let p = zbus::Proxy::new(
                &c,
                "org.freedesktop.UPower.PowerProfiles",
                "/org/freedesktop/UPower/PowerProfiles",
                "org.freedesktop.UPower.PowerProfiles",
            )
            .await
            .map_err(|e| e.to_string())?;
            p.set_property("ActiveProfile", profile)
                .await
                .map_err(|e| e.to_string())
        })
    }

    pub fn active_gpu(&self) -> Result<String, String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async { GpuProxy::new(c).await?.active_gpu().await })
            .map_err(|e| msg(e, "service"))
    }

    pub fn gpu_tune_state(&self, pci: &str) -> Result<ug_proto::GpuTuneState, String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async { GpuProxy::new(c).await?.tune_state(pci).await })
            .map_err(|e| msg(e, "service"))
    }

    /// `op`: try (with `t`), keep, cancel or reset.
    pub fn gpu_tune(&self, pci: &str, op: &str, t: &ug_proto::GpuTune) -> Result<(), String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async {
                let g = GpuProxy::new(c).await?;
                match op {
                    "try" => g.try_tune(pci, t).await,
                    "keep" => g.keep_tune(pci).await,
                    "cancel" => g.cancel_tune(pci).await,
                    _ => g.reset_tune(pci).await,
                }
            })
            .map_err(|e| msg(e, "service"))
    }

    pub fn gamemode_active(&self) -> Result<bool, String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async { GameModeProxy::new(c).await?.active().await })
            .map_err(|e| msg(e, "service"))
    }

    pub fn running(&self) -> Result<Vec<RunningGameInfo>, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { GamesProxy::new(c).await?.running().await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn installed(&self) -> Result<Vec<InstalledGameInfo>, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { GamesProxy::new(c).await?.installed().await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn playtime(&self, since_ms: u64) -> Result<Vec<ug_proto::PlaytimeEntry>, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { PlaytimeProxy::new(c).await?.summary(since_ms).await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn playtime_sessions(
        &self,
        since_ms: u64,
    ) -> Result<Vec<ug_proto::PlaytimeSession>, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { PlaytimeProxy::new(c).await?.sessions(since_ms).await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn launch_options(&self, appid: u32) -> Result<ug_proto::LaunchInfo, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { GamesProxy::new(c).await?.launch_options(appid).await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn preview_launch_options(
        &self,
        appid: u32,
        gpu: &str,
        overlay: bool,
    ) -> Result<String, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async {
                GamesProxy::new(c)
                    .await?
                    .preview_launch_options(appid, gpu, overlay)
                    .await
            })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn set_launch_options(
        &self,
        appid: u32,
        gpu: &str,
        overlay: bool,
    ) -> Result<(String, String), String> {
        let c = self.agent()?;
        self.rt
            .block_on(async {
                GamesProxy::new(c)
                    .await?
                    .set_launch_options(appid, gpu, overlay)
                    .await
            })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn undo_launch_options(&self, appid: u32) -> Result<(String, String), String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { GamesProxy::new(c).await?.undo_launch_options(appid).await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn tracked_apps(&self) -> Result<Vec<ug_proto::TrackedApp>, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { PlaytimeProxy::new(c).await?.tracked_apps().await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn track_app(&self, exe: &str, name: &str) -> Result<(), String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { PlaytimeProxy::new(c).await?.track_app(exe, name).await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn untrack_app(&self, exe: &str) -> Result<bool, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { PlaytimeProxy::new(c).await?.untrack_app(exe).await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn running_programs(&self) -> Result<Vec<ug_proto::TrackedApp>, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { PlaytimeProxy::new(c).await?.running_programs().await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn move_to_apps(&self, key: &str) -> Result<u32, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { PlaytimeProxy::new(c).await?.move_to_apps(key).await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn import_steam(&self) -> Result<(u32, u64), String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { PlaytimeProxy::new(c).await?.import_steam().await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn playtime_imported(&self) -> Result<Vec<ug_proto::PlaytimeEntry>, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { PlaytimeProxy::new(c).await?.imported().await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn forget_imported(&self) -> Result<u32, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { PlaytimeProxy::new(c).await?.forget_imported().await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn playtime_forget(&self, key: &str) -> Result<u32, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { PlaytimeProxy::new(c).await?.forget(key).await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn profiles(&self) -> Result<Vec<ProfileInfo>, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { ProfilesProxy::new(c).await?.list().await })
            .map_err(|e| msg(e, "agent"))
    }

    /// Game mode by hand ("on", "off" or "") and the profile it uses.
    pub fn game_mode_hand(&self) -> Result<(String, String), String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { ProfilesProxy::new(c).await?.game_mode().await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn set_game_mode(&self, on: bool, profile: &str) -> Result<(), String> {
        let c = self.agent()?;
        self.rt
            .block_on(async {
                ProfilesProxy::new(c)
                    .await?
                    .set_game_mode(on, profile)
                    .await
            })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn set_option(&self, profile: &str, option: &str, value: bool) -> Result<(), String> {
        let c = self.agent()?;
        self.rt
            .block_on(async {
                ProfilesProxy::new(c)
                    .await?
                    .set_option(profile, option, value)
                    .await
            })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn set_game_profile(&self, key: &str, profile: &str) -> Result<(), String> {
        let c = self.agent()?;
        self.rt
            .block_on(async {
                ProfilesProxy::new(c)
                    .await?
                    .set_game_profile(key, profile)
                    .await
            })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn history(
        &self,
        since_ms: u64,
        until_ms: u64,
        points: u32,
    ) -> Result<Vec<HistoryPoint>, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async {
                MonitorProxy::new(c)
                    .await?
                    .history(since_ms, until_ms, points)
                    .await
            })
            .map_err(|e| msg(e, "agent"))
    }

    /// Today's charge and use and the health estimates.
    pub fn battery_report(&self) -> Result<ug_proto::BatteryReport, String> {
        let c = self.agent()?;
        let now = now_ms();
        let since = ug_core::playtime::local_midnight_offset(now, 0);
        self.rt
            .block_on(async { MonitorProxy::new(c).await?.battery_report(since, now).await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn traffic(&self, since_ms: u64) -> Result<Vec<DayTraffic>, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { MonitorProxy::new(c).await?.traffic(since_ms).await })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn traffic_buckets(
        &self,
        since_ms: u64,
        until_ms: u64,
        bucket_ms: u64,
    ) -> Result<Vec<ug_proto::TrafficBucket>, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async {
                MonitorProxy::new(c)
                    .await?
                    .traffic_buckets(since_ms, until_ms, bucket_ms)
                    .await
            })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn app_traffic(
        &self,
        since_ms: u64,
        until_ms: u64,
    ) -> Result<Vec<ug_proto::AppUsage>, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async {
                MonitorProxy::new(c)
                    .await?
                    .app_traffic(since_ms, until_ms)
                    .await
            })
            .map_err(|e| msg(e, "agent"))
    }

    pub fn reset_traffic(&self) -> Result<bool, String> {
        let c = self.agent()?;
        self.rt
            .block_on(async { MonitorProxy::new(c).await?.reset_traffic().await })
            .map_err(|e| msg(e, "agent"))
    }

    /// Monitoring → Internet traffic: today by hour, this week (Saturday to
    /// Friday) and this month by day, each with its busiest apps, and the
    /// total since the counts were last reset.
    pub fn traffic_view(&self, now: u64) -> Result<TrafficView, String> {
        use ug_core::playtime::{local_midnight_offset, local_month, local_time, local_week};
        let day = (local_midnight_offset(now, 0), local_midnight_offset(now, 1));
        let hours = self.traffic_buckets(day.0, day.1, 3_600_000)?;
        let all = self.traffic(0)?;
        let by_date: std::collections::HashMap<&str, &DayTraffic> =
            all.iter().map(|d| (d.date.as_str(), d)).collect();
        let days = |(a, b): (u64, u64), label: &dyn Fn(u64) -> String| -> Vec<(String, u64, u64)> {
            (0..31)
                .map(|i| local_midnight_offset(a, i))
                .take_while(|&t| t < b)
                .map(|t| {
                    let d = by_date
                        .get(&local_time(t)[..10])
                        .map(|d| (d.rx_bytes, d.tx_bytes))
                        .unwrap_or((0, 0));
                    (label(t), d.0, d.1)
                })
                .collect()
        };
        const WEEK: [&str; 7] = ["Sa", "Su", "Mo", "Tu", "We", "Th", "Fr"];
        let week = local_week(now);
        let month = local_month(now);
        let week_bars = days(week, &|_| String::new())
            .into_iter()
            .enumerate()
            .map(|(i, (_, r, t))| (WEEK[i % 7].to_string(), r, t))
            .collect();
        let month_bars = days(month, &|t| {
            local_time(t)[8..10].trim_start_matches('0').to_string()
        });
        let period = |title: String,
                      bars: Vec<(String, u64, u64)>,
                      (a, b): (u64, u64)|
         -> Result<TrafficPeriod, String> {
            Ok(TrafficPeriod {
                title,
                bars,
                apps: self.app_traffic(a, b)?,
            })
        };
        Ok(TrafficView {
            so_far: all.iter().map(|d| d.rx_bytes + d.tx_bytes).sum(),
            since: all.first().map(|d| d.date.clone()).unwrap_or_default(),
            periods: [
                period(
                    format!("Today, {}", &local_time(day.0)[..10]),
                    hours
                        .iter()
                        .enumerate()
                        .map(|(i, h)| (format!("{}", i + 1), h.rx_bytes, h.tx_bytes))
                        .collect(),
                    day,
                )?,
                period(
                    format!(
                        "Week {} to {}",
                        &local_time(week.0)[..10],
                        &local_time(week.1 - 1)[..10]
                    ),
                    week_bars,
                    week,
                )?,
                period(
                    format!("Month {}", &local_time(month.0)[..7]),
                    month_bars,
                    month,
                )?,
            ],
        })
    }

    /// Bytes (down, up) between two times, from the agent's minute history.
    pub fn traffic_total(&self, since_ms: u64, until_ms: u64) -> Result<(u64, u64), String> {
        let c = self.agent()?;
        let b = self
            .rt
            .block_on(async {
                MonitorProxy::new(c)
                    .await?
                    .traffic_buckets(
                        since_ms,
                        until_ms,
                        until_ms.saturating_sub(since_ms).max(60_000),
                    )
                    .await
            })
            .map_err(|e| msg(e, "agent"))?;
        Ok(b.iter()
            .fold((0, 0), |(r, t), x| (r + x.rx_bytes, t + x.tx_bytes)))
    }

    pub fn power_caps(&self) -> Result<Vec<PowerCapability>, String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async { PowerProxy::new(c).await?.capabilities().await })
            .map_err(|e| msg(e, "service"))
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
        self.rt
            .block_on(async { PowerProxy::new(c).await?.now(action).await })
            .map_err(|e| msg(e, "service"))
    }

    /// Like `power_now`, but a refusal for another program keeping the
    /// computer awake comes back as `Ok(Some(blockers in plain words))`.
    pub fn power_now_or_blockers(&self, action: &str) -> Result<Option<Vec<String>>, String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async {
                let p = PowerProxy::new(c).await?;
                match p.now(action).await {
                    Ok(()) => Ok(None),
                    Err(e) if ug_proto::power_blocked(&e) => {
                        let list = p.blockers(action).await.unwrap_or_default();
                        Ok(Some(
                            list.iter()
                                .map(|(w, y)| ug_core::sleepguard::describe(w, y))
                                .collect(),
                        ))
                    }
                    Err(e) => Err(e),
                }
            })
            .map_err(|e| msg(e, "service"))
    }

    pub fn power_now_anyway(&self, action: &str) -> Result<(), String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async { PowerProxy::new(c).await?.now_anyway(action).await })
            .map_err(|e| msg(e, "service"))
    }

    pub fn power_schedule(
        &self,
        action: &str,
        at_ms: u64,
        repeat: &str,
    ) -> Result<PowerJob, String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async {
                PowerProxy::new(c)
                    .await?
                    .schedule(action, at_ms, repeat, "")
                    .await
            })
            .map_err(|e| msg(e, "service"))
    }

    pub fn power_cancel(&self, id: u32) -> Result<bool, String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async { PowerProxy::new(c).await?.cancel(id).await })
            .map_err(|e| msg(e, "service"))
    }

    pub fn power_unwatch(&self, id: u32) -> Result<bool, String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async { PowerProxy::new(c).await?.cancel_watch(id).await })
            .map_err(|e| msg(e, "service"))
    }

    pub fn battery(&self) -> Result<BatteryCare, String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async { PowerProxy::new(c).await?.battery().await })
            .map_err(|e| msg(e, "service"))
    }

    pub fn sleep_settings(&self) -> Result<ug_proto::SleepSettings, String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async { PowerProxy::new(c).await?.sleep_settings().await })
            .map_err(|e| msg(e, "service"))
    }

    pub fn set_sleep_mode(&self, mode: &str, keep: bool) -> Result<(), String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async { PowerProxy::new(c).await?.set_sleep_mode(mode, keep).await })
            .map_err(|e| msg(e, "service"))
    }

    pub fn set_button_action(&self, which: &str, action: &str) -> Result<(), String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async {
                PowerProxy::new(c)
                    .await?
                    .set_button_action(which, action)
                    .await
            })
            .map_err(|e| msg(e, "service"))
    }

    pub fn set_battery(&self, mode: &str, limit: u8) -> Result<(), String> {
        let c = self.daemon()?;
        self.rt
            .block_on(async {
                PowerProxy::new(c)
                    .await?
                    .set_battery_care(mode, limit)
                    .await
            })
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
        Cached {
            value: None,
            error: None,
            at: None,
            every,
        }
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
        let n = libc::strftime(
            buf.as_mut_ptr() as *mut libc::c_char,
            buf.len(),
            c"%a %H:%M".as_ptr(),
            &tm,
        );
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }
}

/// One period on the Internet traffic tab.
#[derive(Debug, Clone, Default)]
pub struct TrafficPeriod {
    pub title: String,
    /// Label, bytes down, bytes up: an hour or a day each.
    pub bars: Vec<(String, u64, u64)>,
    pub apps: Vec<ug_proto::AppUsage>,
}

/// Monitoring → Internet traffic: day, week and month.
#[derive(Debug, Clone, Default)]
pub struct TrafficView {
    pub so_far: u64,
    /// The first day counted (after the last reset).
    pub since: String,
    pub periods: [TrafficPeriod; 3],
}
