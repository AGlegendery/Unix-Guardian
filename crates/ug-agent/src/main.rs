//! The Unix Guardian per-user agent.
//!
//! Runs in the user's session: it can see the user's processes, read their
//! launcher libraries and talk to their desktop, none of which the root daemon
//! should do. It notices games, records playtime, keeps games on preferred
//! cores, pauses distracting apps, and (next) asks the daemon for the
//! privileged part of game mode.

mod affinity;
mod apps;
mod background;
mod choices;
mod core;
mod fps;
mod gamemode;
mod health;
mod history;
mod iface;
mod keys;
mod notify;
mod overlay;
mod ping;
mod playtime;
mod ppd;
mod settings;
mod shortcuts;
mod programs;
mod tray;

use anyhow::{Context, Result};
use clap::Parser;
use futures_lite::StreamExt;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ug_hw::SysRoot;
use ug_proto::{AGENT_BUS_NAME, AGENT_OBJECT_PATH};

use crate::core::{AgentCore, Outcome};
use iface::{lock, now_ms, GamesIface, MonitorIface, OverlayIface, PingIface, PlaytimeIface, ProfilesIface, SettingsIface, Shared, ShortcutsIface};

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
        DaemonLink { session, conn: None, applied: None, backoff: 0, ticks: 0, last_error: None }
    }

    async fn proxy(&mut self) -> Option<ug_proto::daemon::GameModeProxy<'static>> {
        if self.conn.is_none() {
            let c = if self.session { zbus::Connection::session().await } else { zbus::Connection::system().await };
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
            let c = if self.session { zbus::Connection::session().await } else { zbus::Connection::system().await };
            self.conn = c.ok();
        }
        let Some(conn) = self.conn.clone() else { return };
        let Ok(d) = ug_proto::daemon::DaemonProxy::new(&conn).await else { return };
        let Ok(s) = d.snapshot().await else { return };
        let gpu = s.gpus.iter().find(|g| !g.asleep && !g.temp_c.is_nan()).map(|g| g.pci.clone());
        let h = Arc::clone(history);
        let _ = tokio::task::spawn_blocking(move || {
            h.record(&s, gpu.as_deref());
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
        let Ok(net) = ug_proto::daemon::NetProxy::new(&c).await else { return };
        let (Ok(apps), Ok(rules)) = (net.apps().await, net.rules().await) else { return };
        let ruled: std::collections::HashSet<String> = rules.iter().map(|r| r.app.to_lowercase()).collect();
        let Some(a) = background::pick(&apps, &ruled, offered) else { return };
        offered.insert(a.name.to_lowercase());
        tracing::info!(app = %a.name, rx = a.rx_bps, tx = a.tx_bps, "background network use during the game");
        let actions = [(notify::LIMIT_WHILE_GAMING, "Limit while I play")];
        if let Some(id) = notify::show(conn, "Background download", &background::sentence(a), &actions).await {
            pending.lock().unwrap_or_else(|e| e.into_inner()).insert(id, a.name.clone());
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

    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).context("HOME is not set")?;
    // SAFETY: getuid cannot fail and has no preconditions.
    let uid = unsafe { libc::getuid() };
    let playtime = Arc::new(
        playtime::Playtime::open(&ug_core::paths::user_data().join("agent.db")).context("opening the playtime database")?,
    );

    let history = Arc::new(
        history::History::open(&ug_core::paths::user_data().join("agent.db")).context("opening the monitoring history")?,
    );
    let test_run = std::env::var("UG_TEST_RUN").ok().filter(|t| !t.is_empty());
    let core: Shared = Arc::new(Mutex::new(
        tokio::task::spawn_blocking(move || {
            let mut core = AgentCore::new(SysRoot::real(), &home, &ug_core::paths::user_config(), playtime, uid);
            core.set_test_run(test_run);
            core
        })
        .await?,
    ));

    let pinger = ping::Pinger::new(SysRoot::real());
    let config_home = ug_core::paths::user_config().parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let overlay = Arc::new(overlay::Overlay::new(
        fps::Fps::for_user(&config_home, &ug_core::paths::user_data()),
        &config_home,
        &ug_core::paths::user_config(),
    ));
    let (announce_tx, mut announce_rx) = tokio::sync::mpsc::unbounded_channel::<Outcome>();
    let settings = Arc::new(settings::Settings::load(&ug_core::paths::user_config(), args.scan_interval.map(|s| s.max(1))));
    let (pressed_tx, mut pressed_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let shortcuts = shortcuts::Shortcuts::new(&ug_core::paths::user_config(), pressed_tx);

    let conn = zbus::connection::Builder::session()?
        .serve_at(AGENT_OBJECT_PATH, GamesIface { core: Arc::clone(&core), announce: announce_tx.clone() })?
        .serve_at(AGENT_OBJECT_PATH, PlaytimeIface { core: Arc::clone(&core), announce: announce_tx.clone() })?
        .serve_at(AGENT_OBJECT_PATH, ProfilesIface { core: Arc::clone(&core) })?
        .serve_at(AGENT_OBJECT_PATH, MonitorIface { history: Arc::clone(&history), overlay: Arc::clone(&overlay) })?
        .serve_at(AGENT_OBJECT_PATH, OverlayIface { overlay: Arc::clone(&overlay) })?
        .serve_at(AGENT_OBJECT_PATH, PingIface { pinger: pinger.clone() })?
        .serve_at(AGENT_OBJECT_PATH, SettingsIface { settings: Arc::clone(&settings) })?
        .serve_at(
            AGENT_OBJECT_PATH,
            ShortcutsIface { shortcuts: Arc::clone(&shortcuts), core: Arc::clone(&core), overlay: Arc::clone(&overlay) },
        )?
        .build()
        .await
        .context("connecting to the session bus")?;

    // The user unit and the XDG autostart entry can both start an agent. The
    // second one steps aside and exits successfully, so systemd does not keep
    // restarting it; zbus's default flags would instead let it take the name
    // from the agent that is already tracking the running game.
    match conn.request_name_with_flags(AGENT_BUS_NAME, zbus::fdo::RequestNameFlags::DoNotQueue.into()).await {
        Ok(zbus::fdo::RequestNameReply::PrimaryOwner) => {}
        Ok(_) | Err(zbus::Error::NameTaken) => {
            tracing::info!("another agent is already running in this session; leaving it in charge");
            return Ok(());
        }
        Err(e) => return Err(e).context("claiming org.unixguardian.Agent1"),
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
    conn.object_server().at(AGENT_OBJECT_PATH, tray::TrayControl { tray }).await.context("serving the tray control")?;
    // Only after the name is ours: a second agent must not bind them too.
    shortcuts.start();
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
    // power-profiles-daemon lives on the system bus even when the Unix
    // Guardian daemon is a development one on the session bus.
    let system_bus = if args.daemon_session { None } else { zbus::Connection::system().await.ok() };

    let scan_ticker = |s: &settings::Settings| {
        let mut t = tokio::time::interval(Duration::from_secs(s.get().scan_interval_s.max(1).into()));
        t.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        t
    };
    let mut ticker = scan_ticker(&settings);
    let mut settings_changed = settings.watch();
    // Notifications: off with --quiet, else as the Settings page says.
    let notify_on = || !args.quiet && settings.get().notifications;
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
            let bus = if daemon_session { zbus::Connection::session().await } else { zbus::Connection::system().await };
            let Ok(bus) = bus else { return };
            let Ok(d) = ug_proto::daemon::DriverProxy::new(&bus).await else { return };
            for op in d.ops().await.unwrap_or_default().iter().filter(|o| o.verify == "check") {
                let body = format!("{}: does everything work? Keep it or undo it in Unix Guardian, Driver troubleshooter (or: ug driver keep).", op.title);
                notify::show(&session, "Check the driver change after the restart", &body, &[]).await;
            }
        });
    }
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
            }
            Some(o) = announce_rx.recv() => announce(&conn, o, notify_on(), &pending_dismissals).await,
            Ok(()) = settings_changed.changed() => ticker = scan_ticker(&settings),
            Some(id) = pressed_rx.recv() => {
                // A key pressed anywhere has no other feedback than this.
                let said = shortcuts::perform(&id, &core, &overlay).await;
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
    lock(&core).shutdown(now_ms());
    if daemon.applied.is_some() {
        daemon.backoff = 0;
        daemon.sync(None).await;
    }
    tracing::info!("agent stopped");
    Ok(())
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
        tokio::task::spawn_blocking(move || ug_hw::procnet::game_peer(&SysRoot::real(), &pids)).await.ok().flatten();
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
        .at(GAMEMODE_PATH, gamemode::GameModeCompat { core: Arc::clone(core) })
        .await
    {
        tracing::warn!(error = %e, "could not serve the GameMode interface");
        return;
    }
    match conn.request_name_with_flags(GAMEMODE_NAME, zbus::fdo::RequestNameFlags::DoNotQueue.into()).await {
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
async fn announce(conn: &zbus::Connection, o: Outcome, notify: bool, pending: &Arc<Mutex<HashMap<u32, String>>>) {
    let emitter = match conn.object_server().interface::<_, GamesIface>(AGENT_OBJECT_PATH).await {
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
                let actions: &[(&str, &str)] =
                    if g.evidence == "heuristic" { &[(notify::NOT_A_GAME, "Not a game")] } else { &[] };
                if let Some(id) = notify::show(conn, &format!("{} detected", g.name), &body, actions).await {
                    if g.evidence == "heuristic" {
                        if let Some(exe) = g.key.strip_prefix("exe:") {
                            pending.lock().unwrap_or_else(|e| e.into_inner()).insert(id, exe.to_string());
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

/// Turns a click on "Not a game" into a dismissal.
async fn watch_dismissals(
    conn: zbus::Connection,
    core: Shared,
    pending: Arc<Mutex<HashMap<u32, String>>>,
    limits: Arc<Mutex<HashMap<u32, String>>>,
    daemon_session: bool,
    announce: tokio::sync::mpsc::UnboundedSender<Outcome>,
) {
    let Ok(proxy) = notify::NotificationsProxy::new(&conn).await else { return };
    let Ok(mut clicks) = proxy.receive_action_invoked().await else { return };
    while let Some(sig) = clicks.next().await {
        let Ok(args) = sig.args() else { continue };
        if args.action_key == notify::LIMIT_WHILE_GAMING {
            let app = limits.lock().unwrap_or_else(|e| e.into_inner()).remove(&args.id);
            if let Some(app) = app {
                let daemon = if daemon_session { zbus::Connection::session().await } else { zbus::Connection::system().await };
                let result = match daemon {
                    Ok(d) => match ug_proto::daemon::NetProxy::new(&d).await {
                        Ok(n) => n.set_rule(&app, false, background::OFFER_UP_KBPS, background::OFFER_DOWN_KBPS, "gaming").await,
                        Err(e) => Err(e),
                    },
                    Err(e) => Err(e),
                };
                let (title, body) = match result {
                    Ok(()) => (format!("{app} is limited while you play"), "About 1 MB/s down and 125 kB/s up. Undo it in Unix Guardian or with `ug net allow`.".to_string()),
                    Err(e) => (format!("Could not limit {app}"), e.to_string()),
                };
                notify::show(&conn, &title, &body, &[]).await;
            }
            continue;
        }
        if args.action_key != notify::NOT_A_GAME {
            continue;
        }
        let exe = pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&args.id);
        if let Some(exe) = exe {
            tracing::info!(exe, "user says this is not a game");
            for o in lock(&core).ignore(&exe, Instant::now(), now_ms()) {
                let _ = announce.send(o);
            }
        }
    }
}
