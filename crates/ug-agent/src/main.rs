//! The Unix Guardian per-user agent.
//!
//! Runs in the user's session: it can see the user's processes, read their
//! launcher libraries and talk to their desktop, none of which the root daemon
//! should do. It notices games, records playtime, keeps games on preferred
//! cores, pauses distracting apps, and (next) asks the daemon for the
//! privileged part of game mode.

mod affinity;
mod alerts;
mod apps;
mod background;
mod battery_report;
mod choices;
mod core;
mod deskkeys;
mod fps;
mod gamemode;
mod health;
mod history;
mod holds;
mod iface;
mod keys;
mod notify;
mod overlay;
mod ping;
mod playtime;
mod ppd;
mod programs;
mod recorder;
mod settings;
mod shortcuts;
mod tray;
mod update;

use anyhow::{Context, Result};
use clap::Parser;
use futures_lite::StreamExt;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ug_hw::SysRoot;
use ug_proto::{AGENT_BUS_NAME, AGENT_OBJECT_PATH};

use crate::core::{AgentCore, Outcome};
use iface::{
    lock, now_ms, GamesIface, MonitorIface, OverlayIface, PingIface, PlaytimeIface, ProfilesIface,
    SettingsIface, Shared, ShortcutsIface,
};

const GAMEMODE_NAME: &str = "com.feralinteractive.GameMode";
const GAMEMODE_PATH: &str = "/com/feralinteractive/GameMode";

#[derive(Parser, Debug)]
#[command(name = "ug-agent", version, about = "Unix Guardian per-user agent")]
struct Args {
    /// Seconds between process-table scans; overrides the Settings page.
    #[arg(long)]
    scan_interval: Option<u32>,

    /// Do not answer on the GameMode D-Bus name.
    #[arg(long)]
    no_gamemode: bool,

    /// Do not show desktop notifications, whatever the Settings page says.
    #[arg(long)]
    quiet: bool,

    /// Reach the daemon on the session bus (for a daemon started with
    /// `ug-daemon --session` during development).
    #[arg(long)]
    daemon_session: bool,

    /// Do not show the tray icon.
    #[arg(long)]
    no_tray: bool,

    #[arg(long, default_value = "info")]
    log: String,
}

/// Keeps the daemon's game mode matching what the running games want.
///
/// Reconciled every tick rather than driven by start/stop events: an event
/// that fails — daemon restarting, polkit refusing, a timeout — is otherwise
/// lost, and the machine stays tuned after the game or untuned during it.
/// Comparing "wanted" with "confirmed" each tick recovers from all of those.
struct DaemonLink {
    session: bool,
    conn: Option<zbus::Connection>,
    /// What the daemon last confirmed; `Some` while game mode is on.
    applied: Option<ug_proto::GameModeRequest>,
    /// Ticks to wait before retrying after a failure, so a refusal is not
    /// re-sent every two seconds.
    backoff: u32,
    ticks: u64,
    last_error: Option<String>,
}

impl DaemonLink {
    fn new(session: bool) -> Self {
        DaemonLink {
            session,
            conn: None,
            applied: None,
            backoff: 0,
            ticks: 0,
            last_error: None,
        }
    }

    async fn proxy(&mut self) -> Option<ug_proto::daemon::GameModeProxy<'static>> {
        if self.conn.is_none() {
            let c = if self.session {
                zbus::Connection::session().await
            } else {
                zbus::Connection::system().await
            };
            self.conn = c.ok();
        }
        let conn = self.conn.clone()?;
        ug_proto::daemon::GameModeProxy::new(&conn).await.ok()
    }

    async fn sync(&mut self, want: Option<ug_proto::GameModeRequest>) {
        self.ticks += 1;
        if self.backoff > 0 {
            self.backoff -= 1;
            return;
        }
        // About once a minute while on, confirm the daemon still agrees: if it
        // restarted it reverted everything, and the game is running untuned.
        if self.applied.is_some() && self.ticks % 30 == 0 {
            if let Some(p) = self.proxy().await {
                if let Ok(false) = p.active().await {
                    tracing::info!("the service restarted during the game; re-applying game mode");
                    self.applied = None;
                }
            }
        }
        if want == self.applied {
            return;
        }
        let Some(p) = self.proxy().await else {
            self.failed("the Unix Guardian service is not reachable".into(), true);
            return;
        };
        // The service applies the system-wide settings only when game mode
        // starts; a different profile while on (by hand, or another game's)
        // puts everything back first and applies the new settings.
        let flags = |r: &ug_proto::GameModeRequest| ug_proto::GameModeRequest {
            pids: Vec::new(),
            ..r.clone()
        };
        if let (Some(w), Some(a)) = (&want, &self.applied) {
            if flags(w) != flags(a) {
                tracing::info!("game mode settings changed; re-applying");
                if let Err(e) = p.leave().await {
                    tracing::warn!(error = %e, "could not put game mode back before re-applying");
                }
            }
        }
        let result = match &want {
            Some(w) => p.enter(w).await,
            None => p.leave().await,
        };
        match result {
            Ok(report) => {
                for l in &report.applied {
                    tracing::info!(change = %l, "game mode");
                }
                for l in &report.skipped {
                    tracing::info!(skipped = %l, "game mode");
                }
                if want.is_none() {
                    tracing::info!("game mode off");
                }
                self.applied = want;
                self.last_error = None;
            }
            Err(e) => {
                // An error reply means the daemon is there and said no: keep the
                // connection. Anything else means it is gone: reconnect later.
                let reachable = matches!(e, zbus::Error::MethodError(..) | zbus::Error::FDO(..));
                self.failed(e.to_string(), !reachable);
            }
        }
    }

    /// Stores one monitoring reading (called once a minute). Charts the GPU
    /// that is working: one that is awake and reports a temperature.
    async fn record_history(&mut self, history: &Arc<history::History>) {
        if self.conn.is_none() {
            let c = if self.session {
                zbus::Connection::session().await
            } else {
                zbus::Connection::system().await
            };
            self.conn = c.ok();
        }
        let Some(conn) = self.conn.clone() else {
            return;
        };
        let Ok(d) = ug_proto::daemon::DaemonProxy::new(&conn).await else {
            return;
        };
        let Ok(s) = d.snapshot().await else { return };
        // An older service has no Extra: the reading is stored without it.
        let extra = d.extra().await.ok();
        let gpu = s
            .gpus
            .iter()
            .find(|g| !g.asleep && !g.temp_c.is_nan())
            .map(|g| g.pci.clone());
        // Per-app traffic, when the service counts it (internet control on).
        let apps = match ug_proto::daemon::NetProxy::new(&conn).await {
            Ok(n) => n.apps().await.unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        let h = Arc::clone(history);
        let _ = tokio::task::spawn_blocking(move || {
            h.record(&s, gpu.as_deref(), extra.as_ref());
            let root = ug_hw::SysRoot::real();
            h.record_power(
                s.ts_unix_ms,
                extra.as_ref().map(|x| x.on_ac),
                ug_hw::battery::screen_on(&root),
            );
            h.record_apps(s.ts_unix_ms, &apps);
            if s.ts_unix_ms % (24 * 3_600_000) < 60_000 {
                h.prune(s.ts_unix_ms);
            }
        })
        .await;
    }

    /// While the game runs: offers to cap the busiest background task, once
    /// per app per game session.
    async fn watch_background(
        &mut self,
        conn: &zbus::Connection,
        offered: &mut std::collections::HashSet<String>,
        pending: &Arc<Mutex<HashMap<u32, String>>>,
    ) {
        let Some(c) = self.conn.clone() else { return };
        let Ok(net) = ug_proto::daemon::NetProxy::new(&c).await else {
            return;
        };
        // Turned off on the Game mode page: no automatic limits, no offers.
        if !net.gaming_limits().await.unwrap_or(true) {
            return;
        }
        let (Ok(apps), Ok(rules)) = (net.apps().await, net.rules().await) else {
            return;
        };
        let ruled: std::collections::HashSet<String> =
            rules.iter().map(|r| r.app.to_lowercase()).collect();
        let Some(a) = background::pick(&apps, &ruled, offered) else {
            return;
        };
        offered.insert(a.name.to_lowercase());
        tracing::info!(app = %a.name, rx = a.rx_bps, tx = a.tx_bps, "background network use during the game");
        let actions = [
            (notify::LIMIT_WHILE_GAMING, "Limit while I play"),
            (notify::SHOW_TRAFFIC, "Show internet use"),
        ];
        let body = format!(
            "{} Limits are listed in Unix Guardian, Monitoring, Internet traffic.",
            background::sentence(a)
        );
        if let Some(id) = notify::show(conn, "Background download", &body, &actions).await {
            pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(id, a.name.clone());
        }
    }

    fn failed(&mut self, msg: String, drop_connection: bool) {
        if drop_connection {
            self.conn = None;
        }
        self.backoff = 15; // ~30 s at the default scan interval
        if self.last_error.as_deref() != Some(&msg) {
            tracing::warn!(error = %msg, "game mode could not be applied; detection and playtime still work");
            self.last_error = Some(msg);
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = Args::parse();
    ug_core::logging::init(&args.log);

    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .context("HOME is not set")?;
    // SAFETY: getuid cannot fail and has no preconditions.
    let uid = unsafe { libc::getuid() };
    let playtime = Arc::new(
        playtime::Playtime::open(&ug_core::paths::user_data().join("agent.db"))
            .context("opening the playtime database")?,
    );

    let history = Arc::new(
        history::History::open(&ug_core::paths::user_data().join("agent.db"))
            .context("opening the monitoring history")?,
    );
    let test_run = std::env::var("UG_TEST_RUN").ok().filter(|t| !t.is_empty());
    let retired = retire_session_files(test_run.is_some());
    let core: Shared = Arc::new(Mutex::new(
        tokio::task::spawn_blocking(move || {
            let mut core = AgentCore::new(
                SysRoot::real(),
                &home,
                &ug_core::paths::user_config(),
                playtime,
                uid,
            );
            core.set_test_run(test_run);
            // In the data folder, not the runtime one: test runs give the
            // agent a scratch data folder, so they never touch this file.
            core.keep_paused_in(ug_core::paths::user_data().join("paused.json"));
            core
        })
        .await?,
    ));

    let pinger = ping::Pinger::new(SysRoot::real());
    let config_home = ug_core::paths::user_config()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    let overlay = Arc::new(overlay::Overlay::new(
        fps::Fps::for_user(&config_home, &ug_core::paths::user_data()),
        &ug_core::paths::user_config(),
    ));
    let (announce_tx, mut announce_rx) = tokio::sync::mpsc::unbounded_channel::<Outcome>();
    let settings = Arc::new(settings::Settings::load(
        &ug_core::paths::user_config(),
        args.scan_interval.map(|s| s.max(1)),
    ));
    // A test run may give the alerts a scratch tree to read (fake heat).
    let alerts_root = std::env::var_os("UG_ALERTS_SYSROOT")
        .filter(|_| test_run_set())
        .map(SysRoot::at)
        .unwrap_or_else(SysRoot::real);
    let alerts = Arc::new(alerts::Alerts::new(
        &ug_core::paths::user_config(),
        &ug_core::paths::user_data(),
        alerts_root,
        args.daemon_session,
        Arc::clone(&settings),
        args.quiet,
    ));
    let (pressed_tx, mut pressed_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let shortcuts = shortcuts::Shortcuts::new(&ug_core::paths::user_config(), pressed_tx);
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    let recorder = Arc::new(recorder::Recorder::new(
        &ug_core::paths::user_config(),
        &home,
    ));
    // A recorder an older agent lost track of (see recorder.rs) is stopped
    // (not by a test run's agent).
    if std::env::var("UG_TEST_RUN").map_or(true, |t| t.is_empty()) {
        let r = Arc::clone(&recorder);
        tokio::task::spawn_blocking(move || r.stop_leftovers(&[]));
    }
    let updater = Arc::new(update::Updater::new(
        &ug_core::paths::user_config(),
        std::path::Path::new("/"),
    ));

    let conn = zbus::connection::Builder::session()?
        .serve_at(
            AGENT_OBJECT_PATH,
            GamesIface {
                core: Arc::clone(&core),
                announce: announce_tx.clone(),
            },
        )?
        .serve_at(
            AGENT_OBJECT_PATH,
            PlaytimeIface {
                core: Arc::clone(&core),
                announce: announce_tx.clone(),
            },
        )?
        .serve_at(
            AGENT_OBJECT_PATH,
            ProfilesIface {
                core: Arc::clone(&core),
            },
        )?
        .serve_at(
            AGENT_OBJECT_PATH,
            MonitorIface {
                history: Arc::clone(&history),
                overlay: Arc::clone(&overlay),
            },
        )?
        .serve_at(
            AGENT_OBJECT_PATH,
            OverlayIface {
                overlay: Arc::clone(&overlay),
            },
        )?
        .serve_at(
            AGENT_OBJECT_PATH,
            iface::RecorderIface {
                recorder: Arc::clone(&recorder),
            },
        )?
        .serve_at(
            AGENT_OBJECT_PATH,
            PingIface {
                pinger: pinger.clone(),
            },
        )?
        .serve_at(
            AGENT_OBJECT_PATH,
            SettingsIface {
                settings: Arc::clone(&settings),
            },
        )?
        .serve_at(
            AGENT_OBJECT_PATH,
            iface::AlertsIface {
                alerts: Arc::clone(&alerts),
            },
        )?
        .serve_at(
            AGENT_OBJECT_PATH,
            iface::UpdateIface {
                updater: Arc::clone(&updater),
                daemon_session: args.daemon_session,
            },
        )?
        .serve_at(
            AGENT_OBJECT_PATH,
            ShortcutsIface {
                shortcuts: Arc::clone(&shortcuts),
                core: Arc::clone(&core),
                overlay: Arc::clone(&overlay),
                recorder: Arc::clone(&recorder),
            },
        )?
        .build()
        .await
        .context("connecting to the session bus")?;

    // The user unit and the XDG autostart entry can both start an agent. The
    // second one steps aside and exits successfully, so systemd does not keep
    // restarting it; zbus's default flags would instead let it take the name
    // from the agent that is already tracking the running game.
    // An agent whose program file was replaced (an update while it ran) is
    // asked to stop instead: left in charge it would answer the updated
    // window with its old types ("Signature mismatch", 2026-10-04).
    if !claim_name(&conn).await? {
        tracing::info!("another agent is already running in this session; leaving it in charge");
        return Ok(());
    }

    if !args.no_gamemode {
        claim_gamemode(&conn, &core).await;
    }
    let tray = if args.no_tray {
        None
    } else {
        match tray::Tray::start(&conn, args.daemon_session, Arc::clone(&core)).await {
            Ok(t) => Some(t),
            Err(e) => {
                tracing::warn!(error = %e, "could not set up the tray icon");
                None
            }
        }
    };
    conn.object_server()
        .at(AGENT_OBJECT_PATH, tray::TrayControl { tray })
        .await
        .context("serving the tray control")?;
    // Only after the name is ours: a second agent must not bind them too.
    shortcuts.start();
    tokio::spawn(Arc::clone(&alerts).run(conn.clone()));
    tracing::info!("agent ready");

    // Notification id → executable, for the "not a game" button, and
    // notification id → app, for "limit while I play".
    let pending_dismissals: Arc<Mutex<HashMap<u32, String>>> = Arc::default();
    let pending_limits: Arc<Mutex<HashMap<u32, String>>> = Arc::default();
    if !args.quiet {
        tokio::spawn(watch_dismissals(
            conn.clone(),
            Arc::clone(&core),
            Arc::clone(&pending_dismissals),
            Arc::clone(&pending_limits),
            args.daemon_session,
            announce_tx.clone(),
        ));
    }
    let mut offered = std::collections::HashSet::new();
    let mut holder = ppd::Holder::new();
    let mut screen_on = holds::Hold::new(holds::Kind::ScreenOn);
    let mut dnd = holds::Hold::new(holds::Kind::DoNotDisturb);
    // power-profiles-daemon lives on the system bus even when the Unix
    // Guardian daemon is a development one on the session bus.
    let system_bus = if args.daemon_session {
        None
    } else {
        zbus::Connection::system().await.ok()
    };

    let scan_ticker = |s: &settings::Settings| {
        let mut t =
            tokio::time::interval(Duration::from_secs(s.get().scan_interval_s.max(1).into()));
        t.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        t
    };
    let mut ticker = scan_ticker(&settings);
    let mut settings_changed = settings.watch();
    // Notifications: off with --quiet, else as the Settings page says.
    // Do not disturb holds Unix Guardian's own notifications too.
    let quiet_now = std::sync::atomic::AtomicBool::new(false);
    let notify_on = || {
        !args.quiet
            && settings.get().notifications
            && !quiet_now.load(std::sync::atomic::Ordering::Relaxed)
    };
    let mut had_game = false;
    let mut heartbeat = tokio::time::interval(Duration::from_secs(60));
    let mut rescan = tokio::time::interval(Duration::from_secs(600));
    rescan.tick().await; // first tick is immediate; the library was just scanned

    // A driver change waiting for its check after the restart: say so once
    // at login, when the desktop has settled.
    if notify_on() {
        let (session, daemon_session) = (conn.clone(), args.daemon_session);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(20)).await;
            let bus = if daemon_session {
                zbus::Connection::session().await
            } else {
                zbus::Connection::system().await
            };
            let Ok(bus) = bus else { return };
            let Ok(d) = ug_proto::daemon::DriverProxy::new(&bus).await else {
                return;
            };
            for op in d
                .ops()
                .await
                .unwrap_or_default()
                .iter()
                .filter(|o| o.verify == "check")
            {
                let body = format!("{}: does everything work? Keep it or undo it in Unix Guardian, Driver troubleshooter (or: ug driver keep).", op.title);
                notify::show(
                    &session,
                    "Check the driver change after the restart",
                    &body,
                    &[],
                )
                .await;
            }
        });
    }
    // Old session-wide files were moved aside: say so, since the card
    // switch's is what broke the desktop (see `ug_core::session_env`).
    if !retired.is_empty() && !args.quiet {
        let moved: Vec<String> = retired
            .iter()
            .map(|r| r.moved_to.display().to_string())
            .collect();
        let body = format!(
            "Unix Guardian no longer sets the graphics card or MangoHud for the whole session; that could break the desktop. \
             The old settings were moved to {}. Programs started with ug run still use the card you chose.",
            moved.join(", ")
        );
        notify::show(&conn, "Old session-wide settings moved", &body, &[]).await;
    }
    // New versions: looked for a minute after login (the network is up by
    // then), then hourly whether a day has passed; never in a test run.
    let mut update_tick = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(60),
        Duration::from_secs(3600),
    );
    let check_updates =
        std::env::var_os("UG_TEST_RUN").is_none() || std::env::var_os("UG_UPDATE_FEED").is_some();
    let mut daemon = DaemonLink::new(args.daemon_session);
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let c = Arc::clone(&core);
                let outcomes = tokio::task::spawn_blocking(move || lock(&c).tick(Instant::now(), now_ms())).await?;
                for o in outcomes {
                    announce(&conn, o, notify_on(), &pending_dismissals).await;
                }
                // A game just started: the window gives its memory back.
                let gaming = !lock(&core).running().is_empty();
                if gaming && !had_game && settings.get().close_window_on_game {
                    tracing::info!("a game started; closing Unix Guardian's windows (Settings)");
                    tray::close_windows();
                }
                had_game = gaming;
                let want = lock(&core).gamemode_desired();
                let was_on = daemon.applied.is_some();
                daemon.sync(want).await;
                if daemon.applied.is_some() != was_on {
                    pinger.mark(if was_on { "gamemode-off" } else { "gamemode-on" }, now_ms());
                    offered.clear();
                }
                if daemon.applied.is_some() && notify_on() && daemon.ticks % 15 == 5 {
                    daemon.watch_background(&conn, &mut offered, &pending_limits).await;
                }
                follow_game(&core, &pinger).await;
                overlay.follow(!lock(&core).running().is_empty());
                let (wanted, game) = lock(&core).power_profile_wanted();
                holder.sync(system_bus.as_ref(), wanted, &game).await;
                let (wanted, why) = lock(&core).flag_wanted(|p| p.keep_screen_on);
                screen_on.sync(&conn, wanted, &format!("Playing: {why}")).await;
                let (wanted, why) = lock(&core).flag_wanted(|p| p.do_not_disturb);
                dnd.sync(&conn, wanted, &format!("Playing: {why}")).await;
                quiet_now.store(dnd.held(), std::sync::atomic::Ordering::Relaxed);
                alerts.dnd.store(dnd.held(), std::sync::atomic::Ordering::Relaxed);
            }
            Some(o) = announce_rx.recv() => announce(&conn, o, notify_on(), &pending_dismissals).await,
            Ok(()) = settings_changed.changed() => ticker = scan_ticker(&settings),
            Some(id) = pressed_rx.recv() => {
                // A key pressed anywhere has no other feedback than this.
                let said = shortcuts::perform(&id, &core, &overlay, &recorder).await;
                tracing::info!(shortcut = %id, result = ?said, "global shortcut");
                if id != "open-window" && notify_on() {
                    let text = said.unwrap_or_else(|e| e);
                    notify::show(&conn, &text, "", &[]).await;
                }
            }
            _ = heartbeat.tick() => {
                lock(&core).heartbeat(now_ms());
                daemon.record_history(&history).await;
                let (h, o) = (Arc::clone(&history), Arc::clone(&overlay));
                let _ = tokio::task::spawn_blocking(move || record_fps_sessions(&h, &o)).await;
            }
            _ = update_tick.tick() => {
                if check_updates && settings.get().check_updates && updater.due(now_ms()) {
                    let u = Arc::clone(&updater);
                    let (_, tell) = tokio::task::spawn_blocking(move || u.check(now_ms())).await?;
                    if let (Some(name), true) = (tell, notify_on()) {
                        let body = format!("{name} is out. See what changed and update it in Unix Guardian, Updates (or: ug update).");
                        notify::show(&conn, "A new Unix Guardian is available", &body, &[]).await;
                    }
                }
            }
            _ = rescan.tick() => {
                let c = Arc::clone(&core);
                tokio::task::spawn_blocking(move || lock(&c).rescan()).await?;
            }
            _ = tokio::signal::ctrl_c() => break,
            _ = term.recv() => break,
        }
    }

    // Resume anything we paused and close open sessions before going. The
    // daemon also reverts game mode by itself once we drop off the bus.
    shortcuts.shutdown().await;
    let r = Arc::clone(&recorder);
    let _ = tokio::task::spawn_blocking(move || r.shutdown()).await;
    lock(&core).shutdown(now_ms());
    if daemon.applied.is_some() {
        daemon.backoff = 0;
        daemon.sync(None).await;
    }
    tracing::info!("agent stopped");
    Ok(())
}

/// `UG_TEST_RUN` is set: an end-to-end test's agent.
fn test_run_set() -> bool {
    std::env::var("UG_TEST_RUN").is_ok_and(|t| !t.is_empty())
}

/// While a game runs, keeps ping measured and aimed at its server, so the
/// chart has the whole game session, before and after game mode went on.
async fn follow_game(core: &Shared, pinger: &ping::Pinger) {
    let game = lock(core).running().into_iter().next();
    let Some(g) = game else {
        pinger.set_game(None);
        return;
    };
    let pids = g.pids.clone();
    let peer =
        tokio::task::spawn_blocking(move || ug_hw::procnet::game_peer(&SysRoot::real(), &pids))
            .await
            .ok()
            .flatten();
    pinger.set_game(Some((g.name, peer)));
    pinger.touch();
}

/// Turns finished MangoHud logs into stored sessions and deletes them
/// (per-frame logs are large). Old leftovers are pruned too.
fn record_fps_sessions(history: &history::History, overlay: &overlay::Overlay) {
    let now = std::time::SystemTime::now();
    for (path, written) in overlay.fps.finished_logs(now) {
        match fps::session_stats(&path, written) {
            Some(s) => {
                tracing::info!(game = %s.game, avg = s.avg_fps, low1 = s.low_1pct_fps, "FPS session recorded");
                if history.record_fps(&s) {
                    let _ = std::fs::remove_file(&path);
                }
            }
            // Too short to mean anything (a launcher flashing MangoHud).
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    overlay.fps.prune(now);
}

/// Takes the GameMode name if nobody has it.
async fn claim_gamemode(conn: &zbus::Connection, core: &Shared) {
    if let Err(e) = conn
        .object_server()
        .at(
            GAMEMODE_PATH,
            gamemode::GameModeCompat {
                core: Arc::clone(core),
            },
        )
        .await
    {
        tracing::warn!(error = %e, "could not serve the GameMode interface");
        return;
    }
    match conn
        .request_name_with_flags(
            GAMEMODE_NAME,
            zbus::fdo::RequestNameFlags::DoNotQueue.into(),
        )
        .await
    {
        Ok(zbus::fdo::RequestNameReply::PrimaryOwner) => {
            tracing::info!("answering GameMode requests from launchers");
        }
        Ok(_) | Err(_) => tracing::info!(
            "Feral GameMode is already running; games will be detected from /proc instead. \
             Running both means two tools tuning the machine at once."
        ),
    }
}

/// Emits the D-Bus signal for a start or stop and shows a notification.
async fn announce(
    conn: &zbus::Connection,
    o: Outcome,
    notify: bool,
    pending: &Arc<Mutex<HashMap<u32, String>>>,
) {
    let emitter = match conn
        .object_server()
        .interface::<_, GamesIface>(AGENT_OBJECT_PATH)
        .await
    {
        Ok(i) => i,
        Err(e) => {
            tracing::warn!(error = %e, "games interface missing");
            return;
        }
    };
    match o {
        Outcome::Started(g) => {
            if notify {
                let body = if g.blocked_note.is_empty() {
                    format!("Game mode is on ({} profile).", g.profile)
                } else {
                    format!("Heads up: {}", g.blocked_note)
                };
                // Only a guess can be wrong in a way the user fixes by hand.
                let actions: &[(&str, &str)] = if g.evidence == "heuristic" {
                    &[(notify::NOT_A_GAME, "Not a game")]
                } else {
                    &[]
                };
                if let Some(id) =
                    notify::show(conn, &format!("{} detected", g.name), &body, actions).await
                {
                    if g.evidence == "heuristic" {
                        if let Some(exe) = g.key.strip_prefix("exe:") {
                            pending
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .insert(id, exe.to_string());
                        }
                    }
                }
            }
            let _ = GamesIface::game_started(emitter.signal_emitter(), g).await;
        }
        Outcome::Stopped(g, played) => {
            let _ = GamesIface::game_stopped(emitter.signal_emitter(), g, played).await;
        }
    }
}

/// Turns a click on "Not a game" into a dismissal, "Limit while I play"
/// into a rule, and a click on an internet-use notification into the
/// window at Monitoring, Internet traffic.
async fn watch_dismissals(
    conn: zbus::Connection,
    core: Shared,
    pending: Arc<Mutex<HashMap<u32, String>>>,
    limits: Arc<Mutex<HashMap<u32, String>>>,
    daemon_session: bool,
    announce: tokio::sync::mpsc::UnboundedSender<Outcome>,
) {
    // The "limited" notifications whose click opens internet traffic.
    let traffic_notes: Mutex<std::collections::HashSet<u32>> = Mutex::default();
    let Ok(proxy) = notify::NotificationsProxy::new(&conn).await else {
        return;
    };
    let Ok(mut clicks) = proxy.receive_action_invoked().await else {
        return;
    };
    while let Some(sig) = clicks.next().await {
        let Ok(args) = sig.args() else { continue };
        // A click on an internet-use notification: show where the limits
        // are. Only ours: the offer (still pending) or a "limited" one.
        if args.action_key == notify::SHOW_TRAFFIC {
            let ours = limits
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key(&args.id)
                || traffic_notes
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&args.id);
            if ours {
                if let Err(e) = tray::open_window_at("monitoring/traffic").await {
                    tracing::warn!(error = %e, "could not open the window at internet traffic");
                }
            }
            continue;
        }
        if args.action_key == notify::LIMIT_WHILE_GAMING {
            let app = limits
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&args.id);
            if let Some(app) = app {
                let daemon = if daemon_session {
                    zbus::Connection::session().await
                } else {
                    zbus::Connection::system().await
                };
                let result = match daemon {
                    Ok(d) => match ug_proto::daemon::NetProxy::new(&d).await {
                        Ok(n) => {
                            n.set_rule(
                                &app,
                                false,
                                background::OFFER_UP_KBPS,
                                background::OFFER_DOWN_KBPS,
                                "gaming",
                            )
                            .await
                        }
                        Err(e) => Err(e),
                    },
                    Err(e) => Err(e),
                };
                let (title, body) = match result {
                    Ok(()) => (format!("{app} is limited while you play"), "About 1 MB/s down and 125 kB/s up. See or undo it in Unix Guardian, Monitoring, Internet traffic (or: ug net allow).".to_string()),
                    Err(e) => (format!("Could not limit {app}"), e.to_string()),
                };
                if let Some(id) = notify::show(
                    &conn,
                    &title,
                    &body,
                    &[(notify::SHOW_TRAFFIC, "Show internet use")],
                )
                .await
                {
                    traffic_notes
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(id);
                }
            }
            continue;
        }
        if args.action_key != notify::NOT_A_GAME {
            continue;
        }
        let exe = pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&args.id);
        if let Some(exe) = exe {
            tracing::info!(exe, "user says this is not a game");
            for o in lock(&core).ignore(&exe, Instant::now(), now_ms()) {
                let _ = announce.send(o);
            }
        }
    }
}

/// Moves the files older versions wrote into `environment.d` aside: the
/// card switch's (it put KWin on the NVIDIA card, 2026-10-04) and the
/// overlay's `MANGOHUD=1`. Then drops exactly their variables from the
/// running user manager, which with `Linger=yes` keeps them across a
/// logout; nothing is ever added. Not during a test run, which must not
/// reach the real user manager. See `ug_core::session_env`.
fn retire_session_files(test_run: bool) -> Vec<ug_core::session_env::Retired> {
    let config_home = ug_core::gpu_default::config_home();
    let ug = ug_core::paths::user_config();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .to_string();
    let results = [
        ug_core::gpu_default::retire_legacy(&config_home, &ug, &stamp),
        ug_core::session_env::retire(
            &config_home,
            &ug,
            ug_core::session_env::LEGACY_FILES[1],
            &stamp,
        ),
    ];
    let mut out = Vec::new();
    for r in results {
        match r {
            Ok(Some(r)) => {
                tracing::warn!(to = %r.moved_to.display(), "moved an old session-wide file out of environment.d");
                out.push(r);
            }
            Ok(None) => {}
            Err(e) => tracing::warn!(error = %e, "could not move an old session-wide file aside"),
        }
    }
    let vars: Vec<&str> = out
        .iter()
        .flat_map(|r| r.vars.iter().map(|(k, _)| k.as_str()))
        .collect();
    if !test_run && !vars.is_empty() {
        let out = std::process::Command::new("systemctl")
            .args(["--user", "unset-environment"])
            .args(&vars)
            .output();
        if let Err(e) = out {
            tracing::warn!(error = %e, "could not drop the old variables from the user manager");
        }
    }
    out
}

/// Claims the agent's bus name. Taken by an up-to-date agent: false. Taken
/// by one running a program file that has since been replaced: that one is
/// stopped (SIGTERM, so it ends game mode and saves playtime) and the name
/// claimed once it is free.
async fn claim_name(conn: &zbus::Connection) -> Result<bool> {
    let flags = zbus::fdo::RequestNameFlags::DoNotQueue.into();
    let mut stopped = None;
    for _ in 0..50 {
        match conn.request_name_with_flags(AGENT_BUS_NAME, flags).await {
            Ok(zbus::fdo::RequestNameReply::PrimaryOwner) => return Ok(true),
            Ok(_) | Err(zbus::Error::NameTaken) => {}
            Err(e) => return Err(e).context("claiming org.unixguardian.Agent1"),
        }
        if stopped.is_none() {
            let Some(pid) = outdated_owner(conn).await else {
                return Ok(false);
            };
            tracing::warn!(
                pid,
                "an agent from before the update is running; stopping it"
            );
            // SAFETY: kill(2) on a process of this user, checked above.
            unsafe { libc::kill(pid as i32, libc::SIGTERM) };
            stopped = Some(pid);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    tracing::warn!("the old agent did not stop within 10 s");
    Ok(false)
}

/// The pid of the agent holding the name, if it is this user's and runs a
/// program file that has been replaced or removed.
async fn outdated_owner(conn: &zbus::Connection) -> Option<u32> {
    let dbus = zbus::fdo::DBusProxy::new(conn).await.ok()?;
    let name = zbus::names::BusName::try_from(AGENT_BUS_NAME).ok()?;
    let pid = dbus.get_connection_unix_process_id(name).await.ok()?;
    let proc = std::path::PathBuf::from(format!("/proc/{pid}"));
    use std::os::unix::fs::MetadataExt;
    // SAFETY: getuid cannot fail.
    if std::fs::metadata(&proc).ok()?.uid() != unsafe { libc::getuid() } {
        return None;
    }
    let exe = std::fs::read_link(proc.join("exe")).ok()?;
    replaced(&exe.to_string_lossy()).then_some(pid)
}

/// The kernel marks a running program whose file was replaced or removed.
fn replaced(exe_link: &str) -> bool {
    exe_link.ends_with(" (deleted)")
}

#[cfg(test)]
mod claim_tests {
    #[test]
    fn a_replaced_program_is_recognised() {
        assert!(super::replaced("/usr/local/bin/ug-agent (deleted)"));
        assert!(!super::replaced("/usr/local/bin/ug-agent"));
    }
}
