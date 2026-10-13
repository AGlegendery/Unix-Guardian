//! `org.unixguardian.Daemon1.Net`: traffic per app, counted in our own
//! nftables table.
//!
//! New cgroups are picked up every 5 s (a directory walk; `nft` runs only
//! when something is new). Counters are read once a minute, or every 2 s for
//! a minute after someone asks, so an idle machine costs one `nft` call a
//! minute.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ug_hw::SysRoot;
use ug_net::account::Accountant;
use ug_net::nft::NftCmd;
use ug_net::rules::{Background, Policy, Rule, Rules, When};
use ug_net::shaper::{RealCmd, Shaper};
use ug_proto::{AppTraffic, NetControlStatus, NetRule};

use crate::polkit::{action, Gate};

const WATCHED: Duration = Duration::from_secs(60);

pub enum Backend {
    Nft(Box<Accountant<NftCmd>>, Box<Shaper<RealCmd>>),
    /// A development daemon on the host: real app names, invented numbers.
    Simulated {
        root: SysRoot,
        start: Instant,
    },
    Unavailable,
}

pub struct Net {
    backend: Backend,
    root: SysRoot,
    rules: Rules,
    /// "Slow down background apps" and where it is saved.
    background: Background,
    background_file: std::path::PathBuf,
    gaming: Arc<AtomicBool>,
    /// The running game's processes, as game mode last received them.
    game_pids: Arc<Mutex<Vec<i32>>>,
    /// What the rules mean right now; recomputed each tick.
    policy: Policy,
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
    if root.join("run/firewalld/firewalld.pid").exists()
        || root.join("var/run/firewalld/firewalld.pid").exists()
    {
        out.push("firewalld".into());
    }
    let ufw = std::fs::read_to_string(root.join("etc/ufw/ufw.conf")).unwrap_or_default();
    if ufw
        .lines()
        .any(|l| l.trim().eq_ignore_ascii_case("ENABLED=yes"))
    {
        out.push("ufw".into());
    }
    out
}

impl Net {
    pub fn new(
        root: SysRoot,
        simulate: bool,
        state_dir: &Path,
        gaming: Arc<AtomicBool>,
        game_pids: Arc<Mutex<Vec<i32>>>,
    ) -> Net {
        let (rules, problem) = Rules::load(&state_dir.join("net.json"));
        let background_file = state_dir.join("net-background.json");
        let background = Background::load(&background_file);
        if let Some(p) = problem {
            tracing::warn!(%p, "internet rules");
        }
        let firewalls = other_firewalls(&root);
        if !firewalls.is_empty() {
            tracing::info!(
                ?firewalls,
                "another firewall is active; Unix Guardian keeps to its own nftables table"
            );
        }
        let (backend, status) = if simulate {
            tracing::warn!(
                "development run: per-app traffic is simulated, nftables is not touched"
            );
            (
                Backend::Simulated {
                    root: root.clone(),
                    start: Instant::now(),
                },
                NetControlStatus {
                    available: true,
                    simulated: true,
                    firewalls,
                    ..Default::default()
                },
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
                    let mut a = Accountant::new(cmd, root.clone());
                    match a.install() {
                        Ok(()) => (
                            Backend::Nft(Box::new(a), {
                                // A crash may have left our shaping behind.
                                let mut s = Shaper::new(RealCmd);
                                s.clean_leftovers(
                                    &ug_hw::net::default_iface(&root).unwrap_or_default(),
                                );
                                Box::new(s)
                            }),
                            NetControlStatus {
                                available: true,
                                firewalls,
                                ..Default::default()
                            },
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
        Net {
            backend,
            root,
            rules,
            background,
            background_file,
            gaming,
            game_pids,
            policy: Policy::default(),
            status,
            wanted_until: None,
            last_read: None,
            apps: Vec::new(),
        }
    }

    fn watched(&self, now: Instant) -> bool {
        self.wanted_until.is_some_and(|t| t > now)
    }

    /// One step of the background loop.
    fn tick(&mut self, now: Instant) {
        let due = match self.last_read {
            None => true,
            Some(t) => {
                now.duration_since(t)
                    >= if self.watched(now) {
                        Duration::from_secs(2)
                    } else {
                        Duration::from_secs(60)
                    }
            }
        };
        let gaming = self.gaming.load(Ordering::Relaxed);
        let game_pids = if gaming {
            self.game_pids
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
        } else {
            Vec::new()
        };
        self.policy = Policy::from_rules(&self.rules.list, gaming).with_background(
            &self.background,
            gaming,
            &game_pids,
        );
        let watched = self.watched(now);
        match &mut self.backend {
            Backend::Nft(a, shaper) => {
                a.set_policy(self.policy.clone());
                // Where the internet really goes: through a VPN's TUN device
                // when one is up (often by policy routing, which the main
                // table does not show).
                // Asking the kernel costs a fork, so only while it matters.
                let iface =
                    if !self.policy.limits.is_empty() || watched || self.status.iface.is_empty() {
                        ug_hw::procnet::route_dev("1.1.1.1".parse().expect("address"))
                            .or_else(|| ug_hw::net::default_iface(&self.root))
                            .unwrap_or_default()
                    } else {
                        self.status.iface.clone()
                    };
                let mut problems = shaper.apply(&iface, &self.policy.limits);
                self.status.vpn_tun = ug_hw::procnet::is_tun(&self.root, &iface);
                self.status.vpn = self
                    .apps
                    .iter()
                    .find(|x| x.category == "vpn")
                    .map(|x| x.name.clone())
                    .unwrap_or_default();
                if !self.status.vpn.is_empty()
                    && !self.status.vpn_tun
                    && !self.policy.limits.is_empty()
                {
                    problems.push(format!(
                        "{} is running as a proxy (not in TUN mode): apps that use it reach the internet through it, \
                         so their speed caps cannot apply. Blocking still works. Switch it to TUN mode for caps.",
                        self.status.vpn
                    ));
                }
                self.status.shaping_problems = problems;
                self.status.iface = iface;
                let r = if due {
                    a.refresh(now)
                } else {
                    a.sync_groups().map(|_| ())
                };
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
                    let states = self.policy.states();
                    self.apps = simulated(root, now.duration_since(*start).as_secs());
                    for a in &mut self.apps {
                        a.state = states
                            .get(&a.name.to_lowercase())
                            .map(|s| s.to_string())
                            .unwrap_or_default();
                        if a.state.is_empty()
                            && self
                                .policy
                                .background_class(&a.name, &a.exe, &a.pids)
                                .is_some()
                        {
                            a.state = "background".into();
                        }
                    }
                }
            }
            Backend::Unavailable => {}
        }
        if due {
            self.last_read = Some(now);
            let pids = if self.gaming.load(Ordering::Relaxed) {
                self.game_pids
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone()
            } else {
                Vec::new()
            };
            for a in &mut self.apps {
                a.has_game = a.pids.iter().any(|p| pids.contains(p));
            }
        }
    }

    /// Removes our table; the daemon's last act on a clean stop.
    pub fn shutdown(&mut self) {
        if let Backend::Nft(a, shaper) = &mut self.backend {
            shaper.clear();
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
            let exe = g
                .pids
                .first()
                .map(|&p| ug_net::cgroups::comm(root, p))
                .unwrap_or_default();
            let l = ug_net::cgroups::label(&g.path, &exe);
            let category = ug_net::cgroups::category(&l.name, &exe).into();
            let busy = l.kind == "app" && i % 3 == 0;
            let wave = (secs + i as u64 * 7) % 20;
            let (rx_bps, tx_bps) = if busy {
                (40_000 * wave, 3_000 * wave)
            } else {
                (0, 0)
            };
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
                state: String::new(),
                category,
                has_game: false,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        (b.rx_bps + b.tx_bps)
            .cmp(&(a.rx_bps + a.tx_bps))
            .then(a.name.cmp(&b.name))
    });
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
    pub gate: Gate,
}

fn rule_out(r: &Rule, gaming: bool) -> NetRule {
    NetRule {
        app: r.app.clone(),
        block: r.block,
        up_kbps: r.up_kbps,
        down_kbps: r.down_kbps,
        when: r.when.as_str().into(),
        active: r.when == When::Always || gaming,
    }
}

#[zbus::interface(name = "org.unixguardian.Daemon1.Net")]
impl NetIface {
    async fn status(&self) -> NetControlStatus {
        lock(&self.net).status.clone()
    }

    /// Every rule, with whether it applies now.
    async fn rules(&self) -> Vec<NetRule> {
        let n = lock(&self.net);
        let gaming = n.gaming.load(Ordering::Relaxed);
        n.rules.list.iter().map(|r| rule_out(r, gaming)).collect()
    }

    /// Blocks an app, or caps its upload and/or download (kbit/s, 0 = no
    /// cap), `always` or only while `gaming`. Replaces the app's rule.
    async fn set_rule(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        app: &str,
        block: bool,
        up_kbps: u32,
        down_kbps: u32,
        when: &str,
    ) -> zbus::fdo::Result<()> {
        let when = When::parse(when).map_err(zbus::fdo::Error::InvalidArgs)?;
        self.gate.authorize(&hdr, action::NET).await?;
        let rule = Rule {
            app: app.to_string(),
            block,
            up_kbps: if block { 0 } else { up_kbps },
            down_kbps: if block { 0 } else { down_kbps },
            when,
        };
        let net = Arc::clone(&self.net);
        tokio::task::spawn_blocking(move || {
            let mut n = lock(&net);
            n.rules.set(rule)?;
            n.last_read = None;
            n.tick(Instant::now());
            Ok::<_, String>(())
        })
        .await
        .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
        .map_err(zbus::fdo::Error::InvalidArgs)
    }

    /// Removes an app's rule; returns false if it had none.
    async fn remove_rule(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        app: &str,
    ) -> zbus::fdo::Result<bool> {
        self.gate.authorize(&hdr, action::NET).await?;
        let net = Arc::clone(&self.net);
        let app = app.to_string();
        tokio::task::spawn_blocking(move || {
            let mut n = lock(&net);
            let r = n.rules.remove(&app)?;
            n.last_read = None;
            n.tick(Instant::now());
            Ok::<_, String>(r)
        })
        .await
        .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
        .map_err(zbus::fdo::Error::Failed)
    }

    /// "Slow down background apps": on, upload and download caps for all
    /// background apps together (kbit/s), and `always` or `gaming`.
    async fn background(&self) -> (bool, u32, u32, String) {
        let b = lock(&self.net).background;
        (b.on, b.up_kbps, b.down_kbps, b.when.as_str().into())
    }

    /// Turns "slow down background apps" on or off. Every app without a rule
    /// of its own shares the caps, except the running game, VPNs and
    /// proxies, and DNS managers.
    async fn set_background(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        on: bool,
        up_kbps: u32,
        down_kbps: u32,
        when: &str,
    ) -> zbus::fdo::Result<()> {
        let when = When::parse(when).map_err(zbus::fdo::Error::InvalidArgs)?;
        self.gate.authorize(&hdr, action::NET).await?;
        let b = Background {
            on,
            up_kbps,
            down_kbps,
            when,
        };
        let net = Arc::clone(&self.net);
        tokio::task::spawn_blocking(move || {
            let mut n = lock(&net);
            b.save(&n.background_file)?;
            n.background = b;
            n.last_read = None;
            n.tick(Instant::now());
            Ok::<_, String>(())
        })
        .await
        .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
        .map_err(zbus::fdo::Error::InvalidArgs)
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
