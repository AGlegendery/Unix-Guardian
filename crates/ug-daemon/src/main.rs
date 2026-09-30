//! The Unix Guardian privileged daemon.
//!
//! Owns every operation that needs privilege — sysfs writes, NVML, nftables,
//! logind — and exposes them on the system bus behind polkit. Nothing else in
//! the project runs as root.
//!
//! # Running it during development
//!
//! `--session` puts the daemon on the session bus as a normal user. Reads all
//! work there, because sensors are world-readable; writes will refuse once they
//! exist, and the inventory reports `privileged: false` so the UI can say so.
//! That is enough to develop against without installing anything or running a
//! build as root.
//!
//! ```console
//! $ cargo run -p ug-daemon -- --session
//! $ cargo run -p ug-cli -- --session status
//! ```

mod battery;
mod conditions;
mod energy;
mod gamemode;
mod iface;
mod polkit;
mod power;
mod rtc;
mod sampler;
mod schedule;

use anyhow::{Context, Result};
use clap::Parser;
use std::sync::Arc;
use ug_hw::SysRoot;
use ug_proto::{DAEMON_BUS_NAME, DAEMON_OBJECT_PATH};

use iface::{GpuIface, Subscribers, SystemIface};
use sampler::Sampler;

#[derive(Parser, Debug)]
#[command(name = "ug-daemon", version, about = "Unix Guardian privileged daemon")]
struct Args {
    /// Serve on the session bus instead of the system bus, for development.
    #[arg(long)]
    session: bool,

    /// Treat this directory as `/`. For replaying a captured machine.
    #[arg(long, value_name = "DIR")]
    sysroot: Option<std::path::PathBuf>,

    /// Where game-mode undo state is kept. Defaults to /var/lib/unix-guardian.
    #[arg(long, value_name = "DIR")]
    state_dir: Option<std::path::PathBuf>,

    /// Log filter; `UG_LOG` overrides it.
    #[arg(long, default_value = "info")]
    log: String,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    let args = Args::parse();
    ug_core::logging::init(&args.log);

    let root = match &args.sysroot {
        Some(p) => SysRoot::at(p.clone()),
        None => SysRoot::real(),
    };

    // Writes need CAP_SYS_ADMIN and friends; being uid 0 is the simple proxy
    // for "the unit granted them", and a development run is expected to fail it.
    let privileged = is_root();
    if !privileged {
        tracing::warn!(
            "running unprivileged: sensors are readable, tuning is not. \
             This is expected with --session."
        );
    }

    let sampler = Arc::new(Sampler::new(root.clone(), privileged));
    let subscribers = Arc::new(Subscribers::default());

    // Default to the first discrete GPU. On a hybrid laptop the firmware posts
    // the integrated one, so `boot_vga` points at exactly the card the user does
    // not want tuned; falling back to it only when there is nothing else.
    let gpus = sampler.gpu_infos();
    let active_gpu = gpus
        .iter()
        .find(|g| !g.boot_vga)
        .or_else(|| gpus.first())
        .map(|g| g.pci.clone())
        .unwrap_or_default();

    let gate = if args.session {
        tracing::warn!("session bus: polkit checks are disabled for this development run");
        polkit::Gate::Development
    } else {
        // polkit lives on the system bus; a dedicated connection keeps its
        // calls independent of the one serving our own interfaces.
        polkit::Gate::Polkit(
            zbus::Connection::system()
                .await
                .context("connecting to the system bus for polkit")?,
        )
    };

    let system = SystemIface {
        sampler: Arc::clone(&sampler),
        subscribers: Arc::clone(&subscribers),
        reload: ug_core::logging::reload_handle(),
        gate: gate.clone(),
    };
    let gpu = GpuIface {
        sampler: Arc::clone(&sampler),
        active: std::sync::Mutex::new(active_gpu),
    };

    let builder = if args.session {
        zbus::connection::Builder::session()?
    } else {
        zbus::connection::Builder::system()?
    };
    let conn = builder
        .serve_at(DAEMON_OBJECT_PATH, system)?
        .serve_at(DAEMON_OBJECT_PATH, gpu)?
        .build()
        .await
        .context("connecting to D-Bus")?;

    // The name is claimed before crash recovery, and without letting anyone
    // take it over: a second daemon (started by hand, or a unit started twice)
    // would otherwise "recover" the undo list of the one still running and
    // revert a game's settings mid-game. zbus's default flags allow exactly
    // that replacement, so they are not used.
    let flags = zbus::fdo::RequestNameFlags::DoNotQueue.into();
    match conn.request_name_with_flags(DAEMON_BUS_NAME, flags).await {
        Ok(zbus::fdo::RequestNameReply::PrimaryOwner) => {}
        Ok(_) | Err(zbus::Error::NameTaken) => {
            anyhow::bail!("another ug-daemon already owns {DAEMON_BUS_NAME}; not starting a second one")
        }
        Err(e) => {
            return Err(e).context(
                "claiming the bus name. On the system bus this needs the daemon's \
                 config file in /usr/share/dbus-1/system.d and root; try --session \
                 for a development run.",
            )
        }
    }

    // Constructing the tuner undoes anything a crashed previous run left on.
    let state_dir = args.state_dir.clone().unwrap_or_else(ug_core::paths::system_state);
    let development = args.session || args.sysroot.is_some();
    let services = if development {
        tracing::warn!("development run: game mode will never stop or start the real system's services");
        gamemode::ServiceManager::Disabled
    } else {
        gamemode::ServiceManager::Systemctl
    };
    let tuner = gamemode::tuner(root.clone(), state_dir.join("gamemode.json"), services);
    let gamemode_owner: Arc<std::sync::Mutex<Option<gamemode::Owner>>> = Arc::default();
    let gamemode_active = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let gamemode = gamemode::GameModeIface {
        tuner: Arc::clone(&tuner),
        gate: gate.clone(),
        owner: Arc::clone(&gamemode_owner),
        active: Arc::clone(&gamemode_active),
    };
    // Served only once recovery is done, so no caller sees a half-restored
    // machine as "game mode off". The agent retries if it asks in between.
    conn.object_server()
        .at(DAEMON_OBJECT_PATH, gamemode)
        .await
        .context("serving the game-mode interface")?;

    // Power: a development run records instead of acting, whatever bus it is on.
    let backend = Arc::new(if development {
        tracing::warn!("development run: power actions are simulated, never performed");
        power::Backend::Simulated(std::sync::Mutex::new(Vec::new()))
    } else {
        power::Backend::Logind(zbus::Connection::system().await.context("connecting to the system bus for logind")?)
    });
    let (schedule, notes) = schedule::Schedule::load(&state_dir.join("schedule.json"), now_unix_ms());
    for n in notes {
        tracing::warn!(%n, "power schedule");
    }
    let schedule = Arc::new(std::sync::Mutex::new(schedule));
    let watches = Arc::new(std::sync::Mutex::new(conditions::Watches::load(&state_dir.join("conditions.json"))));
    let watches_changed = Arc::new(tokio::sync::Notify::new());
    let battery = Arc::new(std::sync::Mutex::new(battery::Battery::new(
        root.clone(),
        &state_dir.join("battery.json"),
        // A --session run on the real /sys must not change charging.
        args.session && args.sysroot.is_none(),
    )));
    let energy = Arc::new(std::sync::Mutex::new(energy::EnergyMeter::load(root.clone(), &state_dir.join("energy.json"))));
    let observer = Arc::new(power::Observer {
        sampler: Arc::clone(&sampler),
        game_active: Arc::clone(&gamemode_active),
        root: root.clone(),
    });
    let schedule_changed = Arc::new(tokio::sync::Notify::new());
    conn.object_server()
        .at(
            DAEMON_OBJECT_PATH,
            power::PowerIface {
                schedule: Arc::clone(&schedule),
                backend: Arc::clone(&backend),
                gate: gate.clone(),
                changed: Arc::clone(&schedule_changed),
                // A --session run on the real /sys must not arm the real
                // clock; with --sysroot the scratch tree is safe to write.
                rtc: (!args.session || args.sysroot.is_some()).then(|| rtc::Rtc::new(root.clone())),
                simulated_alarm: std::sync::Mutex::new(None),
                watches: Arc::clone(&watches),
                observer: Arc::clone(&observer),
                watches_changed: Arc::clone(&watches_changed),
                energy: Arc::clone(&energy),
                battery: Arc::clone(&battery),
            },
        )
        .await
        .context("serving the power interface")?;
    let backend_for_battery = Arc::clone(&backend);
    tokio::spawn(power::run(conn.clone(), schedule, Arc::clone(&backend), schedule_changed));
    tokio::spawn(power::watch(watches, observer, backend, watches_changed));
    tokio::spawn(power::meter(energy));
    tokio::spawn(power::keep_battery(battery, match &*backend_for_battery {
        power::Backend::Logind(c) => Some(c.clone()),
        power::Backend::Simulated(_) => None,
    }));

    tracing::info!(
        bus = if args.session { "session" } else { "system" },
        name = DAEMON_BUS_NAME,
        "daemon ready"
    );

    let watch_conn = conn.clone();
    let watch_subs = Arc::clone(&subscribers);
    let watch_tuner = Arc::clone(&tuner);
    let watch_owner = Arc::clone(&gamemode_owner);
    let watch_active = Arc::clone(&gamemode_active);
    tokio::spawn(async move {
        if let Err(e) = release_departed_clients(watch_conn, watch_subs, watch_tuner, watch_owner, watch_active).await {
            tracing::error!(error = %e, "cannot watch for disconnecting clients");
        }
    });

    let release_sampler = Arc::clone(&sampler);
    tokio::spawn(async move {
        release_gpu_libs_when_idle(release_sampler).await;
    });

    let loop_conn = conn.clone();
    let loop_subs = Arc::clone(&subscribers);
    let loop_sampler = Arc::clone(&sampler);
    tokio::spawn(async move {
        sampling_loop(loop_conn, loop_subs, loop_sampler).await;
    });

    shutdown_signal().await;
    tracing::info!("shutting down");
    // Stopping the service ends game mode; the next start would undo it from
    // the state file anyway, but a stopped daemon should leave a clean system.
    let report = tokio::task::spawn_blocking(move || tuner.lock().unwrap_or_else(|e| e.into_inner()).leave()).await?;
    for l in report.skipped {
        tracing::warn!(%l, "could not restore on shutdown; will retry on next start");
    }
    Ok(())
}

/// Emits a snapshot per interval while anyone is subscribed, and parks
/// completely when nobody is.
async fn sampling_loop(
    conn: zbus::Connection,
    subscribers: Arc<Subscribers>,
    sampler: Arc<Sampler>,
) {
    loop {
        subscribers.wait_for_interest().await;

        let snapshot = sampler.snapshot();
        let emitted = async {
            let iface = conn
                .object_server()
                .interface::<_, SystemIface>(DAEMON_OBJECT_PATH)
                .await?;
            SystemIface::sample(iface.signal_emitter(), snapshot).await
        }
        .await;

        if let Err(e) = emitted {
            tracing::warn!(error = %e, "failed to emit sample");
        }

        tokio::time::sleep(subscribers.interval()).await;
    }
}

/// How long a GPU vendor library stays loaded after its last use.
///
/// NVML costs ~15 MB resident; releasing it returns all of that. Thirty seconds
/// spans the gap between a user's clicks without keeping it around for the
/// hours a machine spends with no UI open.
const GPU_LIB_GRACE: std::time::Duration = std::time::Duration::from_secs(30);

/// Releases GPU vendor libraries once they have gone unused for the grace
/// period, then parks until something uses them again.
///
/// Event-driven rather than a periodic sweep, so an idle daemon still does no
/// periodic work: after a release, this task sleeps until the next reading.
async fn release_gpu_libs_when_idle(sampler: Arc<Sampler>) {
    loop {
        sampler.gpu_activity().await;
        let mut wait = GPU_LIB_GRACE;
        loop {
            tokio::time::sleep(wait).await;
            match sampler.release_idle_gpu_libs(GPU_LIB_GRACE) {
                None => break,
                Some(remaining) => wait = remaining,
            }
        }
    }
}

/// Releases the subscription of any client that leaves the bus.
///
/// `NameOwnerChanged` with an empty new owner is the bus announcing that a
/// unique name is gone for good — unique names are never reused.
async fn release_departed_clients(
    conn: zbus::Connection,
    subscribers: Arc<Subscribers>,
    tuner: gamemode::SharedTuner,
    gamemode_owner: Arc<std::sync::Mutex<Option<gamemode::Owner>>>,
    gamemode_active: Arc<std::sync::atomic::AtomicBool>,
) -> Result<()> {
    use futures_lite::StreamExt;
    let dbus = zbus::fdo::DBusProxy::new(&conn).await?;
    let mut changes = dbus.receive_name_owner_changed().await?;
    while let Some(change) = changes.next().await {
        let Ok(args) = change.args() else { continue };
        if args.new_owner().is_none() && args.name().starts_with(':') {
            subscribers.client_gone(args.name());
            // An agent that crashed or was killed mid-game must not leave the
            // machine in game mode.
            let (t, o, a, n) =
                (Arc::clone(&tuner), Arc::clone(&gamemode_owner), Arc::clone(&gamemode_active), args.name().to_string());
            tokio::task::spawn_blocking(move || gamemode::GameModeIface::owner_gone(&t, &o, &a, &n));
        }
    }
    Ok(())
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// True when the process can perform privileged writes.
fn is_root() -> bool {
    // Reading `/proc/self/status` avoids pulling in libc for one number.
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))?
                .split_whitespace()
                .nth(1)?
                .parse::<u32>()
                .ok()
        })
        .map(|uid| uid == 0)
        .unwrap_or(false)
}

/// Resolves on SIGINT or SIGTERM.
///
/// systemd sends SIGTERM on stop, and the revert paths added in later phases
/// hang off this: hardware left in a modified state must be restored before the
/// process goes away.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(e) => {
                tracing::error!(error = %e, "cannot listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    tokio::select! {
        _ = ctrl_c => {}
        _ = term => {}
    }
}
