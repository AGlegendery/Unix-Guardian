//! `org.unixguardian.Daemon1.GameMode`: the privileged half of game mode.

use crate::polkit::{action, Gate};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use ug_hw::SysRoot;
use ug_proto::{GameModeReport, GameModeRequest};
use ug_tune::{units::Units, Report, Request, Tuner};

/// How the daemon reaches the service manager.
///
/// `Disabled` in development runs (`--session` or `--sysroot`): those point
/// every file write at a scratch tree, but `systemctl` would still act on the
/// real machine — and, from an unprivileged process, raise a password prompt
/// on the user's desktop for each unit. A development daemon must never
/// touch the real system's services.
pub enum ServiceManager {
    Systemctl,
    Disabled,
}

impl Units for ServiceManager {
    fn is_active(&self, unit: &str) -> bool {
        match self {
            ServiceManager::Disabled => false,
            ServiceManager::Systemctl => std::process::Command::new("systemctl")
                .args(["is-active", "--quiet", unit])
                .status()
                .map(|s| s.success())
                .unwrap_or(false),
        }
    }
    // `--no-block` so a slow unit can never stall a game launch, and
    // `--no-ask-password` so a misconfigured install fails instead of
    // prompting the user mid-game.
    fn stop(&self, unit: &str) -> Result<(), String> {
        match self {
            ServiceManager::Disabled => {
                Err("service management is disabled in development mode".into())
            }
            ServiceManager::Systemctl => run(&["stop", "--no-block", "--no-ask-password", unit]),
        }
    }
    fn start(&self, unit: &str) -> Result<(), String> {
        match self {
            ServiceManager::Disabled => {
                Err("service management is disabled in development mode".into())
            }
            ServiceManager::Systemctl => run(&["start", "--no-block", "--no-ask-password", unit]),
        }
    }
    fn wifi_power_save(&self, iface: &str) -> Option<bool> {
        if let ServiceManager::Disabled = self {
            return None;
        }
        let out = std::process::Command::new(iw()?)
            .args(["dev", iface, "get", "power_save"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let v = text.trim().strip_prefix("Power save:")?.trim();
        Some(v == "on")
    }
    fn set_wifi_power_save(&self, iface: &str, on: bool) -> Result<(), String> {
        if let ServiceManager::Disabled = self {
            return Err("Wi-Fi changes are disabled in development mode".into());
        }
        let iw = iw().ok_or("the iw program is not installed")?;
        let out = std::process::Command::new(iw)
            .args([
                "dev",
                iface,
                "set",
                "power_save",
                if on { "on" } else { "off" },
            ])
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }
}

/// The `iw` program, which sets Wi-Fi power saving through nl80211.
fn iw() -> Option<&'static str> {
    ["/usr/sbin/iw", "/sbin/iw", "/usr/bin/iw"]
        .into_iter()
        .find(|p| std::path::Path::new(p).exists())
}

fn run(args: &[&str]) -> Result<(), String> {
    let out = std::process::Command::new("systemctl")
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

pub type SharedTuner = Arc<Mutex<Tuner<ServiceManager>>>;

pub fn tuner(root: SysRoot, state: std::path::PathBuf, services: ServiceManager) -> SharedTuner {
    let (t, recovered) = Tuner::new(root, state, services);
    for line in recovered.applied.iter().chain(&recovered.skipped) {
        tracing::warn!(%line, "recovered from a previous run");
    }
    Arc::new(Mutex::new(t))
}

fn to_wire(r: Report) -> GameModeReport {
    GameModeReport {
        applied: r.applied,
        skipped: r.skipped,
    }
}

/// Who turned game mode on.
#[derive(Debug, Clone)]
pub struct Owner {
    /// The connection that entered; if it leaves the bus, game mode is reverted.
    pub name: String,
    /// Its user. The same user may take over from a new connection — an agent
    /// that reconnected — but no other user may touch this session's game mode.
    pub uid: u32,
}

pub struct GameModeIface {
    /// The game's processes, for the net interface (which warns before a
    /// rule would also cap the game).
    pub game_pids: Arc<Mutex<Vec<i32>>>,
    pub tuner: SharedTuner,
    pub gate: Gate,
    pub owner: Arc<Mutex<Option<Owner>>>,
    /// Mirrors the tuner's state so the `Active` property can be read without
    /// waiting on the tuner's lock, which an `Enter` may hold for a while.
    pub active: Arc<AtomicBool>,
}

impl GameModeIface {
    /// Reverts game mode if `client` owned it. Called when a name leaves the bus.
    pub fn owner_gone(
        tuner: &SharedTuner,
        owner: &Mutex<Option<Owner>>,
        active: &AtomicBool,
        client: &str,
    ) {
        let mut o = owner.lock().unwrap_or_else(|e| e.into_inner());
        if o.as_ref().is_some_and(|o| o.name == client) {
            *o = None;
            drop(o);
            tracing::warn!(
                client,
                "game mode owner left the bus without leaving game mode; reverting"
            );
            let mut t = tuner.lock().unwrap_or_else(|e| e.into_inner());
            t.leave();
            active.store(t.active(), Ordering::SeqCst);
        }
    }
}

#[zbus::interface(name = "org.unixguardian.Daemon1.GameMode")]
impl GameModeIface {
    async fn enter(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
        request: GameModeRequest,
    ) -> zbus::fdo::Result<GameModeReport> {
        self.gate.authorize(&hdr, action::GAMEMODE).await?;
        let sender = hdr.sender().map(|s| s.to_string()).unwrap_or_default();
        let uid = caller_uid(conn, &sender).await?;

        {
            let mut owner = self.owner.lock().unwrap_or_else(|e| e.into_inner());
            if owner.as_ref().is_some_and(|o| o.uid != uid) {
                return Err(zbus::fdo::Error::AccessDenied(
                    "game mode is in use by another user's session".into(),
                ));
            }
            *owner = Some(Owner {
                name: sender.clone(),
                uid,
            });
        }

        let req = Request {
            cpu_performance: request.cpu_performance,
            cpu_no_idle_states: request.cpu_no_idle_states,
            cpu_priority: request.cpu_priority,
            memory_tuning: request.memory_tuning,
            pause_services: request.pause_services,
            network_tuning: request.network_tuning,
            pids: request.pids,
        };
        *self.game_pids.lock().unwrap_or_else(|e| e.into_inner()) = req.pids.clone();
        let tuner = Arc::clone(&self.tuner);
        let active = Arc::clone(&self.active);
        let report = tokio::task::spawn_blocking(move || {
            let mut t = tuner.lock().unwrap_or_else(|e| e.into_inner());
            let r = t.enter(&req, uid);
            active.store(t.active(), Ordering::SeqCst);
            r
        })
        .await
        .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        for l in &report.applied {
            tracing::info!(change = %l, "game mode");
        }
        for l in &report.skipped {
            tracing::info!(skipped = %l, "game mode");
        }
        Ok(to_wire(report))
    }

    async fn leave(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<GameModeReport> {
        let sender = hdr.sender().map(|s| s.to_string()).unwrap_or_default();
        let uid = caller_uid(conn, &sender).await?;
        {
            let mut owner = self.owner.lock().unwrap_or_else(|e| e.into_inner());
            if owner.as_ref().is_some_and(|o| o.uid != uid) {
                return Err(zbus::fdo::Error::AccessDenied(
                    "game mode belongs to another user's session".into(),
                ));
            }
            *owner = None;
        }
        let tuner = Arc::clone(&self.tuner);
        let active = Arc::clone(&self.active);
        let report = tokio::task::spawn_blocking(move || {
            let mut t = tuner.lock().unwrap_or_else(|e| e.into_inner());
            let r = t.leave();
            active.store(t.active(), Ordering::SeqCst);
            r
        })
        .await
        .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        tracing::info!("game mode off");
        Ok(to_wire(report))
    }

    #[zbus(property)]
    async fn active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }
}

/// The Unix user behind a bus name, as the bus itself vouches for it.
async fn caller_uid(conn: &zbus::Connection, sender: &str) -> zbus::fdo::Result<u32> {
    let dbus = zbus::fdo::DBusProxy::new(conn).await?;
    let name = zbus::names::BusName::try_from(sender)
        .map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?;
    dbus.get_connection_unix_user(name).await
}
