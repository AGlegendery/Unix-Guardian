//! `org.unixguardian.Daemon1.Net`: traffic per app, counted in our own
//! nftables table.
//!
//! New cgroups are picked up every 5 s (a directory walk; `nft` runs only
//! when something is new). Counters are read once a minute, or every 2 s for
//! a minute after someone asks, so an idle machine costs one `nft` call a
//! minute.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ug_hw::SysRoot;
use ug_net::account::Accountant;
use ug_net::nft::NftCmd;
use ug_proto::{AppTraffic, NetControlStatus};

const WATCHED: Duration = Duration::from_secs(60);

pub enum Backend {
    Nft(Box<Accountant<NftCmd>>),
    /// A development daemon on the host: real app names, invented numbers.
    Simulated { root: SysRoot, start: Instant },
    Unavailable,
}

pub struct Net {
    backend: Backend,
    status: NetControlStatus,
    wanted_until: Option<Instant>,
    last_read: Option<Instant>,
    apps: Vec<AppTraffic>,
}

pub type SharedNet = Arc<Mutex<Net>>;

fn lock(n: &SharedNet) -> std::sync::MutexGuard<'_, Net> {
    n.lock().unwrap_or_else(|e| e.into_inner())
}

/// True inside a user namespace (`unshare -rn`): root there cannot touch the
/// host's firewall, so tests may use the real nftables code path.
pub fn in_private_userns() -> bool {
    std::fs::read_to_string("/proc/self/uid_map")
        .map(|m| m.split_whitespace().collect::<Vec<_>>() != ["0", "0", "4294967295"])
        .unwrap_or(false)
}

/// Firewalls that manage their own rules alongside ours.
fn other_firewalls(root: &SysRoot) -> Vec<String> {
    let mut out = Vec::new();
    if root.join("run/firewalld/firewalld.pid").exists() || root.join("var/run/firewalld/firewalld.pid").exists() {
        out.push("firewalld".into());
    }
    let ufw = std::fs::read_to_string(root.join("etc/ufw/ufw.conf")).unwrap_or_default();
    if ufw.lines().any(|l| l.trim().eq_ignore_ascii_case("ENABLED=yes")) {
        out.push("ufw".into());
    }
    out
}

impl Net {
    pub fn new(root: SysRoot, simulate: bool) -> Net {
        let firewalls = other_firewalls(&root);
        if !firewalls.is_empty() {
            tracing::info!(?firewalls, "another firewall is active; Unix Guardian keeps to its own nftables table");
        }
        let (backend, status) = if simulate {
            tracing::warn!("development run: per-app traffic is simulated, nftables is not touched");
            (
                Backend::Simulated { root, start: Instant::now() },
                NetControlStatus { available: true, simulated: true, firewalls, problem: String::new() },
            )
        } else {
            match NftCmd::find() {
                None => (
                    Backend::Unavailable,
                    NetControlStatus {
                        problem: "nftables is not installed (the nft program is missing).".into(),
                        firewalls,
                        ..Default::default()
                    },
                ),
                Some(cmd) => {
                    let mut a = Accountant::new(cmd, root);
                    match a.install() {
                        Ok(()) => (
                            Backend::Nft(Box::new(a)),
                            NetControlStatus { available: true, firewalls, ..Default::default() },
                        ),
                        Err(e) => {
                            tracing::warn!(error = %e, "per-app traffic unavailable");
                            (
                                Backend::Unavailable,
                                NetControlStatus {
                                    problem: format!(
                                        "The kernel refused Unix Guardian's nftables table ({e}). \
                                         Counting per app needs Linux 5.13 or newer with nft_socket."
                                    ),
                                    firewalls,
                                    ..Default::default()
                                },
                            )
                        }
                    }
                }
            }
        };
        Net { backend, status, wanted_until: None, last_read: None, apps: Vec::new() }
    }

    fn watched(&self, now: Instant) -> bool {
        self.wanted_until.is_some_and(|t| t > now)
    }

    /// One step of the background loop.
    fn tick(&mut self, now: Instant) {
        let due = match self.last_read {
            None => true,
            Some(t) => now.duration_since(t) >= if self.watched(now) { Duration::from_secs(2) } else { Duration::from_secs(60) },
        };
        match &mut self.backend {
            Backend::Nft(a) => {
                let r = if due { a.refresh(now) } else { a.sync_groups().map(|_| ()) };
                match r {
                    Ok(()) => {
                        if due {
                            self.apps = a.apps();
                        }
                    }
                    Err(e) => tracing::debug!(error = %e, "counting per app"),
                }
            }
            Backend::Simulated { root, start } => {
                if due {
                    self.apps = simulated(root, now.duration_since(*start).as_secs());
                }
            }
            Backend::Unavailable => {}
        }
        if due {
            self.last_read = Some(now);
        }
    }

    /// Removes our table; the daemon's last act on a clean stop.
    pub fn shutdown(&mut self) {
        if let Backend::Nft(a) = &mut self.backend {
            if let Err(e) = a.uninstall() {
                tracing::warn!(error = %e, "could not remove the nftables table");
            }
        }
    }
}

/// Plausible numbers for the apps really running, so a development window
/// shows real names moving.
fn simulated(root: &SysRoot, secs: u64) -> Vec<AppTraffic> {
    let mut out: Vec<AppTraffic> = ug_net::cgroups::scan(root)
        .into_iter()
        .enumerate()
        .map(|(i, g)| {
            let exe = g.pids.first().map(|&p| ug_net::cgroups::comm(root, p)).unwrap_or_default();
            let l = ug_net::cgroups::label(&g.path, &exe);
            let busy = l.kind == "app" && i % 3 == 0;
            let wave = ((secs + i as u64 * 7) % 20) as u64;
            let (rx_bps, tx_bps) = if busy { (40_000 * wave, 3_000 * wave) } else { (0, 0) };
            AppTraffic {
                id: g.path,
                name: l.name,
                kind: l.kind.into(),
                uid: l.uid,
                exe,
                pids: g.pids,
                rx_bps,
                tx_bps,
                rx_bytes: rx_bps * secs / 2,
                tx_bytes: tx_bps * secs / 2,
            }
        })
        .collect();
    out.sort_by(|a, b| (b.rx_bps + b.tx_bps).cmp(&(a.rx_bps + a.tx_bps)).then(a.name.cmp(&b.name)));
    out
}

pub async fn run(net: SharedNet) {
    loop {
        let n = Arc::clone(&net);
        let _ = tokio::task::spawn_blocking(move || lock(&n).tick(Instant::now())).await;
        let watched = lock(&net).watched(Instant::now());
        tokio::time::sleep(Duration::from_secs(if watched { 2 } else { 5 })).await;
    }
}

pub struct NetIface {
    pub net: SharedNet,
}

#[zbus::interface(name = "org.unixguardian.Daemon1.Net")]
impl NetIface {
    async fn status(&self) -> NetControlStatus {
        lock(&self.net).status.clone()
    }

    async fn apps(&self) -> Vec<AppTraffic> {
        let net = Arc::clone(&self.net);
        tokio::task::spawn_blocking(move || {
            let mut n = lock(&net);
            let now = Instant::now();
            let was_watched = n.watched(now);
            n.wanted_until = Some(now + WATCHED);
            // The first look after a quiet spell gets fresh numbers at once.
            if !was_watched {
                n.last_read = None;
                n.tick(now);
            }
            n.apps.clone()
        })
        .await
        .unwrap_or_default()
    }
}
