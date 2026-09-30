//! The Unix Guardian per-user agent.
//!
//! Runs in the user's session: it can see the user's processes, read their
//! launcher libraries and talk to their desktop, none of which the root daemon
//! should do. It notices games, records playtime, keeps games on preferred
//! cores, pauses distracting apps, and (next) asks the daemon for the
//! privileged part of game mode.

mod affinity;
mod apps;
mod core;
mod gamemode;
mod iface;
mod notify;
mod playtime;

use anyhow::{Context, Result};
use clap::Parser;
use futures_lite::StreamExt;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ug_hw::SysRoot;
use ug_proto::{AGENT_BUS_NAME, AGENT_OBJECT_PATH};

use crate::core::{AgentCore, Outcome};
use iface::{lock, now_ms, GamesIface, PlaytimeIface, Shared};

const GAMEMODE_NAME: &str = "com.feralinteractive.GameMode";
const GAMEMODE_PATH: &str = "/com/feralinteractive/GameMode";

#[derive(Parser, Debug)]
#[command(name = "ug-agent", version, about = "Unix Guardian per-user agent")]
struct Args {
    /// Seconds between process-table scans.
    #[arg(long, default_value_t = 2)]
    scan_interval: u64,

    /// Do not answer on the GameMode D-Bus name.
    #[arg(long)]
    no_gamemode: bool,

    /// Do not show desktop notifications.
    #[arg(long)]
    quiet: bool,

    /// Reach the daemon on the session bus (for a daemon started with
    /// `ug-daemon --session` during development).
    #[arg(long)]
    daemon_session: bool,

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

    let core: Shared = Arc::new(Mutex::new(
        tokio::task::spawn_blocking(move || {
            AgentCore::new(SysRoot::real(), &home, &ug_core::paths::user_config(), playtime, uid)
        })
        .await?,
    ));

    let (announce_tx, mut announce_rx) = tokio::sync::mpsc::unbounded_channel::<Outcome>();

    let conn = zbus::connection::Builder::session()?
        .name(AGENT_BUS_NAME)
        .context("claiming org.unixguardian.Agent1 — is another agent already running?")?
        .serve_at(AGENT_OBJECT_PATH, GamesIface { core: Arc::clone(&core), announce: announce_tx.clone() })?
        .serve_at(AGENT_OBJECT_PATH, PlaytimeIface { core: Arc::clone(&core) })?
        .build()
        .await
        .context("connecting to the session bus")?;

    if !args.no_gamemode {
        claim_gamemode(&conn, &core).await;
    }
    tracing::info!("agent ready");

    // Notification id → executable, for the "not a game" button.
    let pending_dismissals: Arc<Mutex<HashMap<u32, String>>> = Arc::default();
    if !args.quiet {
        tokio::spawn(watch_dismissals(conn.clone(), Arc::clone(&core), Arc::clone(&pending_dismissals), announce_tx.clone()));
    }

    let mut ticker = tokio::time::interval(Duration::from_secs(args.scan_interval.max(1)));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut heartbeat = tokio::time::interval(Duration::from_secs(60));
    let mut rescan = tokio::time::interval(Duration::from_secs(600));
    rescan.tick().await; // first tick is immediate; the library was just scanned

    let mut daemon = DaemonLink::new(args.daemon_session);
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let c = Arc::clone(&core);
                let outcomes = tokio::task::spawn_blocking(move || lock(&c).tick(Instant::now(), now_ms())).await?;
                for o in outcomes {
                    announce(&conn, o, !args.quiet, &pending_dismissals).await;
                }
                let want = lock(&core).gamemode_desired();
                daemon.sync(want).await;
            }
            Some(o) = announce_rx.recv() => announce(&conn, o, !args.quiet, &pending_dismissals).await,
            _ = heartbeat.tick() => lock(&core).heartbeat(now_ms()),
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
    lock(&core).shutdown(now_ms());
    if daemon.applied.is_some() {
        daemon.backoff = 0;
        daemon.sync(None).await;
    }
    tracing::info!("agent stopped");
    Ok(())
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
    announce: tokio::sync::mpsc::UnboundedSender<Outcome>,
) {
    let Ok(proxy) = notify::NotificationsProxy::new(&conn).await else { return };
    let Ok(mut clicks) = proxy.receive_action_invoked().await else { return };
    while let Some(sig) = clicks.next().await {
        let Ok(args) = sig.args() else { continue };
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
