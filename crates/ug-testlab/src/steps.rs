//! The steps themselves.
//!
//! Each step reads what it can, tries what is safe to try, and writes one
//! line per finding. A step never fails the run: a missing service or a
//! refused call becomes a FAIL or SKIP line with the reason, and the next
//! step goes on, because a report with gaps is still worth sending.

use crate::journal::{Journal, Undo};
use crate::{bench, scrub::Scrubber, utc_stamp, Check, Options, Report, Status, PROBE, REPORT_FORMAT};
use std::future::Future;
use std::time::{Duration, Instant};
use ug_proto::agent as ag;
use ug_proto::daemon as dm;
use ug_proto::{Inventory, Snapshot};

/// Longest a single call may take (stopping the recorder can take ten
/// seconds by itself).
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// A call's error as the sentence the service gave, not zbus's wrapping.
fn why(e: zbus::Error) -> String {
    let s = e.to_string();
    if let Some(m) = ug_proto::outdated_peer(&s) {
        return m.to_string();
    }
    match e {
        zbus::Error::MethodError(_, Some(m), _) => m,
        zbus::Error::FDO(f) => match *f {
            zbus::fdo::Error::Failed(m)
            | zbus::fdo::Error::InvalidArgs(m)
            | zbus::fdo::Error::AccessDenied(m)
            | zbus::fdo::Error::NotSupported(m)
            | zbus::fdo::Error::AuthFailed(m) => m,
            other => other.to_string(),
        },
        _ => s,
    }
}

/// Runs a call with a time limit; the error is a sentence.
async fn call<T>(f: impl Future<Output = zbus::Result<T>>) -> Result<T, String> {
    match tokio::time::timeout(CALL_TIMEOUT, f).await {
        Ok(r) => r.map_err(why),
        Err(_) => Err(format!("no answer within {} s", CALL_TIMEOUT.as_secs())),
    }
}

/// Same, and how long it took in milliseconds.
async fn timed<T>(f: impl Future<Output = zbus::Result<T>>) -> (Result<T, String>, f64) {
    let t = Instant::now();
    let r = call(f).await;
    (r, t.elapsed().as_secs_f64() * 1000.0)
}

fn opt(v: f64, unit: &str) -> String {
    if v.is_nan() {
        "none".into()
    } else {
        format!("{v:.1} {unit}")
    }
}

/// A Unix Guardian process's resident memory (MB) and CPU time (s), found by
/// name in /proc. Both files are readable for any user.
fn proc_usage(comm: &str) -> Option<(f64, f64)> {
    let tick = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f64;
    for e in std::fs::read_dir("/proc").ok()?.flatten() {
        let p = e.path();
        if std::fs::read_to_string(p.join("comm")).ok().is_none_or(|c| c.trim() != comm) {
            continue;
        }
        let status = std::fs::read_to_string(p.join("status")).ok()?;
        let rss_kb: f64 = status.lines().find_map(|l| l.strip_prefix("VmRSS:"))?.trim().trim_end_matches("kB").trim().parse().ok()?;
        let stat = std::fs::read_to_string(p.join("stat")).ok()?;
        // Fields after the `)` that closes the program name.
        let rest: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
        let ticks: f64 = rest.get(11)?.parse::<f64>().ok()? + rest.get(12)?.parse::<f64>().ok()?;
        return Some((rss_kb / 1024.0, ticks / tick));
    }
    None
}

pub(crate) struct Lab {
    quick: bool,
    front: &'static str,
    started: Instant,
    started_ms: u64,
    daemon: Option<zbus::Connection>,
    daemon_on_session: bool,
    agent: Option<zbus::Connection>,
    daemon_problem: String,
    agent_problem: String,
    journal: Journal,
    env: Vec<(String, String)>,
    checks: Vec<Check>,
    benches: Vec<(String, String)>,
    manual: Vec<(String, String)>,
    leftovers: Vec<String>,
    inventory: Option<Inventory>,
    snapshot: Option<Snapshot>,
    /// CPU seconds the daemon and agent had used when the run began.
    cpu_at_start: (Option<f64>, Option<f64>),
}

impl Lab {
    pub(crate) async fn new(opts: &Options) -> Lab {
        let daemon = if opts.daemon_on_session { zbus::Connection::session().await } else { zbus::Connection::system().await };
        let agent = zbus::Connection::session().await;
        Lab {
            quick: opts.quick,
            front: opts.front,
            started: Instant::now(),
            started_ms: now_ms(),
            daemon_problem: daemon.as_ref().err().map(|e| e.to_string()).unwrap_or_default(),
            agent_problem: agent.as_ref().err().map(|e| e.to_string()).unwrap_or_default(),
            daemon: daemon.ok(),
            daemon_on_session: opts.daemon_on_session,
            agent: agent.ok(),
            journal: Journal::open(&opts.journal),
            env: Vec::new(),
            checks: Vec::new(),
            benches: Vec::new(),
            manual: Vec::new(),
            leftovers: Vec::new(),
            inventory: None,
            snapshot: None,
            cpu_at_start: (None, None),
        }
    }

    fn add(&mut self, area: &str, name: &str, status: Status, detail: impl Into<String>) {
        self.checks.push(Check { area: area.into(), name: name.into(), status, detail: detail.into() });
    }

    fn bench(&mut self, name: &str, value: impl Into<String>) {
        self.benches.push((name.into(), value.into()));
    }

    fn manual(&mut self, what: &str, why: &str) {
        self.manual.push((what.into(), why.into()));
    }

    fn env(&mut self, k: &str, v: impl Into<String>) {
        self.env.push((k.into(), v.into()));
    }

    /// The daemon connection, or a SKIP line saying why there is none.
    fn need_daemon(&mut self, area: &str) -> Option<zbus::Connection> {
        if self.daemon.is_none() || self.inventory.is_none() {
            self.add(area, "all", Status::Skip, "the Unix Guardian service is not reachable (see Service above)");
        }
        self.daemon.clone().filter(|_| self.inventory.is_some())
    }

    fn need_agent(&mut self, area: &str) -> Option<zbus::Connection> {
        if self.agent.is_none() {
            self.add(area, "all", Status::Skip, "the Unix Guardian agent is not reachable (see Agent above)");
        }
        self.agent.clone()
    }

    async fn fresh_snapshot(&self) -> Option<Snapshot> {
        let d = dm::DaemonProxy::new(self.daemon.as_ref()?).await.ok()?;
        call(d.snapshot()).await.ok()
    }

    /// Records an undo before the change; if even that fails, the change is
    /// not made.
    fn remember(&mut self, u: Undo) -> bool {
        match self.journal.push(u) {
            Ok(()) => true,
            Err(e) => {
                self.add("lab", "undo file", Status::Fail, format!("could not write the undo file, so nothing is turned on: {e}"));
                false
            }
        }
    }

    fn forget(&mut self, u: &Undo) {
        let _ = self.journal.done(u);
    }

    pub(crate) async fn step(&mut self, id: &str) {
        match id {
            "recover" => {
                let done = self.put_back().await;
                if !done.is_empty() {
                    self.add("lab", "earlier test", Status::Info, format!("an earlier test was interrupted; put back: {}", done.join("; ")));
                }
            }
            "env" => self.environment(),
            "services" => self.services().await,
            "sensors" => self.sensors().await,
            "speed" => self.speed().await,
            "gpu" => self.gpu().await,
            "cpu-bench" => self.cpu_bench().await,
            "gamemode" => self.gamemode().await,
            "profiles" => self.profiles().await,
            "games" => self.games().await,
            "playtime" => self.playtime().await,
            "history" => self.history().await,
            "ping" => self.ping().await,
            "power" => self.power().await,
            "fans" => self.fans().await,
            "net" => self.net().await,
            "overlay" => self.overlay().await,
            "recorder" => self.recorder().await,
            "misc" => self.misc().await,
            "drivers" => self.drivers().await,
            "cleanup" => self.cleanup().await,
            _ => {}
        }
    }

    // ---- steps ---------------------------------------------------------

    fn environment(&mut self) {
        let root = ug_hw::SysRoot::real();
        let d = ug_core::distro::Distro::detect(&root);
        let read = |p: &str| std::fs::read_to_string(p).map(|s| s.trim().to_string()).unwrap_or_default();
        let var = |k: &str| std::env::var(k).unwrap_or_default();
        self.env("App", format!("Unix Guardian {} ({})", env!("CARGO_PKG_VERSION"), self.front));
        self.env("Distribution", format!("{} {} (id {}, {:?}, tier {:?}{})", d.name, d.version_id, d.id, d.family, d.tier, if d.is_immutable() { ", immutable" } else { "" }));
        self.env("Kernel", read("/proc/sys/kernel/osrelease"));
        self.env("Desktop", format!("{} on {}", or_none(&var("XDG_CURRENT_DESKTOP")), or_none(&var("XDG_SESSION_TYPE"))));
        self.env("Init", if d.has_systemd { "systemd" } else { "not systemd" });
        self.env("Language", or_none(&var("LANG")));
        let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
        self.env("Installed at", exe);
        let model = read("/sys/class/dmi/id/product_name");
        let vendor = read("/sys/class/dmi/id/sys_vendor");
        let chassis = match read("/sys/class/dmi/id/chassis_type").as_str() {
            "8" | "9" | "10" | "14" | "31" | "32" => "laptop",
            "3" | "4" | "5" | "6" | "7" | "13" | "35" => "desktop",
            "" => "unknown",
            _ => "other",
        };
        self.env("Machine", format!("{} {} ({chassis})", or_none(&vendor), model).trim().to_string());
    }

    async fn services(&mut self) {
        let bus = if self.daemon_on_session { "session" } else { "system" };
        match self.daemon.clone() {
            None => self.add("service", "reachable", Status::Fail, format!("cannot open the {bus} bus: {}", self.daemon_problem)),
            Some(c) => {
                let d = match dm::DaemonProxy::new(&c).await {
                    Ok(d) => d,
                    Err(e) => {
                        self.add("service", "reachable", Status::Fail, why(e));
                        return;
                    }
                };
                let (inv, ms) = timed(d.inventory()).await;
                match inv {
                    Ok(inv) => {
                        let ver = call(d.version()).await.unwrap_or_default();
                        let proto = call(d.protocol_version()).await.unwrap_or(0);
                        let st = if proto == ug_proto::PROTOCOL_VERSION { Status::Pass } else { Status::Warn };
                        self.add("service", "reachable", st, format!("version {ver}, protocol {proto} (this app speaks {}), {ms:.0} ms", ug_proto::PROTOCOL_VERSION));
                        if !ver.is_empty() && ver != env!("CARGO_PKG_VERSION") {
                            self.add("service", "same version as this app", Status::Warn, format!("service {ver}, app {}", env!("CARGO_PKG_VERSION")));
                        }
                        if !inv.privileged && !self.daemon_on_session {
                            self.add("service", "runs as root", Status::Warn, "the service is not privileged; tuning will not work");
                        }
                        self.hardware(&inv);
                        self.inventory = Some(inv);
                    }
                    Err(e) => self.add(
                        "service",
                        "reachable",
                        Status::Fail,
                        format!("{e}. Is it installed and running? sudo systemctl start unix-guardiand"),
                    ),
                }
            }
        }
        match self.agent.clone() {
            None => self.add("agent", "reachable", Status::Fail, format!("cannot open the session bus: {}", self.agent_problem)),
            Some(c) => {
                let r = match ag::SettingsProxy::new(&c).await {
                    Ok(s) => timed(s.state()).await,
                    Err(e) => (Err(why(e)), 0.0),
                };
                match r {
                    (Ok(s), ms) => {
                        self.add("agent", "reachable", Status::Pass, format!("version {}, {ms:.0} ms", s.agent_version));
                        if s.agent_version != env!("CARGO_PKG_VERSION") {
                            self.add("agent", "same version as this app", Status::Warn, format!("agent {}, app {}", s.agent_version, env!("CARGO_PKG_VERSION")));
                        }
                    }
                    (Err(e), _) => {
                        self.add("agent", "reachable", Status::Fail, format!("{e}. Start it: systemctl --user start ug-agent"));
                        self.agent = None;
                    }
                }
            }
        }
        self.cpu_at_start = (proc_usage("ug-daemon").map(|u| u.1), proc_usage("ug-agent").map(|u| u.1));
    }

    fn hardware(&mut self, inv: &Inventory) {
        let c = &inv.cpu;
        let mut cpu = format!("{} — {} threads, {} cores", c.model, c.logical, c.physical);
        if !c.p_cores.is_empty() {
            cpu += &format!(" ({} P-threads, {} E-threads)", c.p_cores.len(), c.e_cores.len());
        }
        if c.has_vcache {
            cpu += ", 3D V-Cache";
        }
        self.env("CPU", cpu);
        self.env("Memory", format!("{:.1} GB", inv.mem_total_kb as f64 / 1_048_576.0));
        for (i, g) in inv.gpus.iter().enumerate() {
            let ver = if g.driver_version.is_empty() { String::new() } else { format!(" {}", g.driver_version) };
            let vram = if g.vram_total_mb > 0 { format!(", {} MB", g.vram_total_mb) } else { String::new() };
            let boot = if g.boot_vga { ", shows the desktop" } else { "" };
            self.env(&format!("GPU {}", i + 1), format!("{} [{:?} {:04x}] at {}, driver {}{ver}{vram}{boot}", g.name, g.vendor, g.device_id, g.pci, or_none(&g.driver)));
        }
        let bat = std::fs::read_dir("/sys/class/power_supply")
            .map(|d| d.flatten().any(|e| std::fs::read_to_string(e.path().join("type")).is_ok_and(|t| t.trim() == "Battery")))
            .unwrap_or(false);
        self.env("Battery", if bat { "yes" } else { "no" });
    }

    async fn sensors(&mut self) {
        let Some(c) = self.need_daemon("sensors") else { return };
        let Ok(d) = dm::DaemonProxy::new(&c).await else { return };
        let s = match call(d.snapshot()).await {
            Ok(s) => s,
            Err(e) => return self.add("sensors", "reading", Status::Fail, e),
        };
        let cpu = &s.cpu;
        let t = |ok: bool| if ok { Status::Pass } else { Status::Warn };
        self.add("sensors", "CPU temperature", t(!cpu.temp_c.is_nan()), if cpu.temp_c.is_nan() { "no CPU temperature sensor found (hwmon)".into() } else { opt(cpu.temp_c, "°C") });
        self.add("sensors", "CPU power", t(!cpu.power_w.is_nan()), if cpu.power_w.is_nan() { "no power reading (RAPL not available)".into() } else { opt(cpu.power_w, "W") });
        self.add("sensors", "CPU clock", t(cpu.freq_avg_mhz > 0), format!("{} MHz average, {} MHz highest", cpu.freq_avg_mhz, cpu.freq_max_mhz));
        let logical = self.inventory.as_ref().map(|i| i.cpu.logical as usize).unwrap_or(0);
        self.add("sensors", "load per CPU thread", t(cpu.per_core_load.len() == logical), format!("{} of {logical} threads", cpu.per_core_load.len()));
        self.add("sensors", "CPU frequency control", Status::Info, format!("driver {}, governor {}, energy preference {}", or_none(&cpu.driver), or_none(&cpu.governor), or_none(&cpu.epp)));
        let m = &s.mem;
        self.add("sensors", "memory", t(m.total_kb > 0), format!("{:.0} % used, swap {:.1} GB, zram {:.1} GB used", m.used_pct(), m.swap_total_kb as f64 / 1_048_576.0, m.zram_used_kb as f64 / 1_048_576.0));
        self.add("sensors", "network", t(!s.net.iface.is_empty()), if s.net.iface.is_empty() { "no default route found".into() } else { format!("default route via a {} interface", iface_kind(&s.net.iface)) });
        for g in &s.gpus {
            let name = format!("GPU {}", g.pci);
            if g.asleep {
                self.add("sensors", &name, Status::Info, "asleep (power saving); not woken for the test, as in normal use");
                continue;
            }
            let have: Vec<&str> = [
                ("temperature", !g.temp_c.is_nan()),
                ("load", !g.load_pct.is_nan()),
                ("power", !g.power_w.is_nan()),
                ("clock", g.core_clock_mhz > 0),
                ("video memory", g.vram_used_mb > 0),
                ("fan", !g.fan_pct.is_nan() || g.fan_rpm > 0),
            ]
            .iter()
            .filter(|x| x.1)
            .map(|x| x.0)
            .collect();
            let driver = self.inventory.as_ref().and_then(|i| i.gpus.iter().find(|x| x.pci == g.pci)).map(|x| x.driver.clone()).unwrap_or_default();
            let own = gpu_has_own_temperature(&driver);
            let st = if have.contains(&"temperature") || (!own && !have.is_empty()) { Status::Pass } else { Status::Warn };
            let note = if !own && !have.contains(&"temperature") { "; integrated graphics have no temperature sensor of their own (the CPU's temperature covers them)" } else { "" };
            self.add("sensors", &name, st, format!("reads: {} ({}, {}){note}", if have.is_empty() { "nothing".into() } else { have.join(", ") }, opt(g.temp_c, "°C"), opt(g.power_w, "W")));
        }
        match call(d.extra()).await {
            Ok(e) => {
                let bat = if e.battery_pct.is_nan() { "no battery".into() } else { format!("battery {:.0} % {} ({})", e.battery_pct, e.battery_status, opt(e.battery_w, "W")) };
                self.add("sensors", "disk, battery, load", Status::Pass, format!("{} per-thread clocks, disk busy {}, {bat}, {} processes", e.per_core_mhz.len(), opt(e.disk_busy_pct, "%"), e.processes));
            }
            Err(e) => self.add("sensors", "disk, battery, load", Status::Fail, e),
        }
        self.snapshot = Some(s);
    }

    async fn speed(&mut self) {
        if let Some(c) = self.daemon.clone().filter(|_| self.inventory.is_some()) {
            if let Ok(d) = dm::DaemonProxy::new(&c).await {
                let mut ms = Vec::new();
                for _ in 0..20 {
                    if let (Ok(_), t) = timed(d.snapshot()).await {
                        ms.push(t);
                    }
                }
                if !ms.is_empty() {
                    let avg = ms.iter().sum::<f64>() / ms.len() as f64;
                    let max = ms.iter().cloned().fold(0.0, f64::max);
                    let st = if avg < 100.0 { Status::Pass } else { Status::Warn };
                    self.add("speed", "full reading from the service", st, format!("average {avg:.1} ms, slowest {max:.1} ms (20 reads)"));
                    self.bench("Service: one full reading", format!("{avg:.1} ms average, {max:.1} ms slowest"));
                }
            }
        }
        if let Some(c) = self.agent.clone() {
            if let Ok(g) = ag::GamesProxy::new(&c).await {
                let mut ms = Vec::new();
                for _ in 0..5 {
                    if let (Ok(_), t) = timed(g.running()).await {
                        ms.push(t);
                    }
                }
                if !ms.is_empty() {
                    self.bench("Agent: running games", format!("{:.1} ms average", ms.iter().sum::<f64>() / ms.len() as f64));
                }
            }
        }
        for (comm, label) in [("ug-daemon", "Service"), ("ug-agent", "Agent")] {
            match proc_usage(comm) {
                Some((rss, _)) => self.bench(&format!("{label} memory"), format!("{rss:.1} MB")),
                None => self.bench(&format!("{label} memory"), "process not found (development build or other name)"),
            }
        }
    }

    async fn gpu(&mut self) {
        let root = ug_hw::SysRoot::real();
        let gpus = ug_hw::gpu_use::gpus(&root);
        let chosen = ug_core::gpu_default::chosen(&ug_core::paths::user_config()).unwrap_or_default();
        if gpus.len() > 1 {
            let other = gpus.iter().find(|g| !g.boot_vga).or(gpus.get(1));
            match other {
                Some(g) => {
                    let env = ug_hw::gpu_use::offload_env(&root, g);
                    let names: Vec<&str> = env.iter().map(|e| e.0.as_str()).collect();
                    let st = if env.is_empty() { Status::Warn } else { Status::Pass };
                    self.add("gpu", "run a program on the other card", st, format!("{} {} via {}; programs start on: {}", g.vendor_name(), g.pci, if names.is_empty() { "nothing known".into() } else { names.join(" ") }, or_none(&chosen)));
                }
                None => {}
            }
            self.manual("Running a game on the other graphics card", "needs a real program on screen: try `ug run --gpu discrete -- <program>` or the dashboard switch");
        } else {
            self.add("gpu", "graphics cards", Status::Info, format!("{} card(s); no switching needed", gpus.len()));
        }
        let Some(c) = self.need_daemon("gpu") else { return };
        let Ok(gp) = dm::GpuProxy::new(&c).await else { return };
        match call(gp.list()).await {
            Ok(list) => self.add("gpu", "list", Status::Pass, format!("{} card(s); tuning acts on {}", list.len(), or_none(&call(gp.active_gpu()).await.unwrap_or_default()))),
            Err(e) => return self.add("gpu", "list", Status::Fail, e),
        }
        let awake: Vec<String> = self.snapshot.as_ref().map(|s| s.gpus.iter().filter(|g| !g.asleep).map(|g| g.pci.clone()).collect()).unwrap_or_default();
        let gpus = self.inventory.as_ref().map(|i| i.gpus.clone()).unwrap_or_default();
        for g in gpus {
            if !awake.contains(&g.pci) {
                self.add("gpu", &format!("tuning {}", g.pci), Status::Skip, "asleep; reading its limits would wake it");
                continue;
            }
            match call(gp.tune_state(&g.pci)).await {
                Ok(t) => {
                    let mut can = Vec::new();
                    if t.power_limit {
                        can.push(format!("power limit {:.0}–{:.0} W", t.power_min_w, t.power_max_w));
                    }
                    if t.core_offset {
                        can.push(format!("core offset {}..{} MHz", t.core_min_mhz, t.core_max_mhz));
                    }
                    if t.mem_offset {
                        can.push(format!("memory offset {}..{} MHz", t.mem_min_mhz, t.mem_max_mhz));
                    }
                    let notes = if t.notes.is_empty() { String::new() } else { format!(" — {}", t.notes.join("; ")) };
                    self.add("gpu", &format!("tuning {}", g.pci), Status::Pass, format!("can change: {}{notes}", if can.is_empty() { "nothing".into() } else { can.join(", ") }));
                }
                Err(e) => self.add("gpu", &format!("tuning {}", g.pci), Status::Warn, e),
            }
        }
        self.manual("GPU tuning (power limit, clock offsets)", "asks for the administrator password and changes the card: try it on the Game mode page");
    }

    async fn cpu_bench(&mut self) {
        let (one, all) = if self.quick { (Duration::from_millis(100), Duration::from_millis(200)) } else { (Duration::from_secs(2), Duration::from_secs(4)) };
        let before = self.fresh_snapshot().await;
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let ok = bench::self_check();
            let _ = tx.send((bench::cpu(one, all), ok));
        });
        // A reading in the middle of the all-core part: the sensors must
        // see the load.
        tokio::time::sleep(one + all / 2).await;
        let during = self.fresh_snapshot().await;
        let Ok((score, ok)) = rx.await else {
            return self.add("cpu", "benchmark", Status::Fail, "the benchmark thread stopped");
        };
        self.add("cpu", "arithmetic check", if ok { Status::Pass } else { Status::Fail }, if ok { "same result every time" } else { "different results: the CPU is unstable" });
        let ratio = score.all_cores / score.one_core.max(1e-9);
        self.bench("CPU one thread", format!("{:.0} M rounds/s", score.one_core));
        self.bench("CPU all threads", format!("{:.0} M rounds/s on {} threads ({ratio:.1}× one thread)", score.all_cores, score.threads));
        if let (Some(b), Some(d)) = (before, during) {
            let st = if d.cpu.load_pct > 60.0 { Status::Pass } else { Status::Warn };
            self.add("cpu", "load reading follows a real load", st, format!("{:.0} % before, {:.0} % during", b.cpu.load_pct, d.cpu.load_pct));
            self.bench(
                "CPU under load",
                format!("{} MHz, {} (was {}), {}", d.cpu.freq_avg_mhz, opt(d.cpu.temp_c, "°C"), opt(b.cpu.temp_c, "°C"), opt(d.cpu.power_w, "W")),
            );
        }
    }

    async fn gamemode(&mut self) {
        let Some(c) = self.need_daemon("game mode") else { return };
        // `Active` is a property the service does not signal, so a caching
        // proxy would keep answering what it read first.
        let Ok(gm) = dm::GameModeProxy::builder(&c).cache_properties(zbus::proxy::CacheProperties::No).build().await else { return };
        if call(gm.active()).await.unwrap_or(false) {
            return self.add("game mode", "on and off", Status::Skip, "game mode is on now (a game or by hand), so it was not touched");
        }
        if let Some(a) = self.agent.clone() {
            if let Ok(p) = ag::ProfilesProxy::new(&a).await {
                match call(p.game_mode()).await {
                    Ok((hand, _)) if hand == "on" => {
                        return self.add("game mode", "on and off", Status::Skip, "game mode is on by hand, so it was not touched");
                    }
                    Ok(_) => self.add("game mode", "agent's game mode state", Status::Pass, "read"),
                    Err(e) => self.add("game mode", "agent's game mode state", Status::Fail, e),
                }
            }
        }
        let before = self.fresh_snapshot().await;
        // Everything a profile can ask for, on the lab's own process. If the
        // lab dies now, the service reverts by itself when this connection
        // goes away.
        let req = ug_proto::GameModeRequest {
            cpu_performance: true,
            cpu_no_idle_states: true,
            cpu_priority: true,
            memory_tuning: true,
            pause_services: true,
            network_tuning: true,
            pids: vec![std::process::id() as i32],
        };
        let (r, enter_ms) = timed(gm.enter(&req)).await;
        let report = match r {
            Ok(r) => r,
            Err(e) => return self.add("game mode", "turn on", Status::Fail, e),
        };
        let on = call(gm.active()).await.unwrap_or(false);
        let during = self.fresh_snapshot().await;
        let skipped = if report.skipped.is_empty() { String::new() } else { format!("; skipped: {}", report.skipped.join("; ")) };
        self.add(
            "game mode",
            "turn on",
            if on { Status::Pass } else { Status::Fail },
            format!("applied: {}{skipped}", if report.applied.is_empty() { "nothing".into() } else { report.applied.join("; ") }),
        );
        if let (Some(b), Some(d)) = (&before, &during) {
            self.add("game mode", "CPU settings while on", Status::Info, format!("governor {} → {}, energy preference {} → {}", or_none(&b.cpu.governor), or_none(&d.cpu.governor), or_none(&b.cpu.epp), or_none(&d.cpu.epp)));
        }
        let (r, leave_ms) = timed(gm.leave()).await;
        let off = !call(gm.active()).await.unwrap_or(true);
        let after = self.fresh_snapshot().await;
        match r {
            Ok(rep) => {
                let left = if rep.skipped.is_empty() { String::new() } else { format!("; could not restore: {}", rep.skipped.join("; ")) };
                let st = if off && rep.skipped.is_empty() { Status::Pass } else { Status::Fail };
                self.add("game mode", "turn off and restore", st, format!("restored: {}{left}", rep.applied.len()));
                if !off {
                    self.leftovers.push("game mode is still on: turn it off on the Game mode page".into());
                }
            }
            Err(e) => {
                self.add("game mode", "turn off and restore", Status::Fail, e);
                self.leftovers.push("game mode may still be on: turn it off on the Game mode page".into());
            }
        }
        if let (Some(b), Some(a)) = (&before, &after) {
            let same = b.cpu.governor == a.cpu.governor && b.cpu.epp == a.cpu.epp;
            self.add("game mode", "CPU settings back as they were", if same { Status::Pass } else { Status::Fail }, format!("governor {}, energy preference {}", or_none(&a.cpu.governor), or_none(&a.cpu.epp)));
        }
        self.bench("Game mode", format!("on in {enter_ms:.0} ms, off in {leave_ms:.0} ms"));
    }

    async fn profiles(&mut self) {
        let Some(c) = self.need_agent("profiles") else { return };
        let Ok(p) = ag::ProfilesProxy::new(&c).await else { return };
        match call(p.list()).await {
            Ok(l) => self.add("profiles", "list", Status::Pass, format!("{} profiles ({} built in)", l.len(), l.iter().filter(|p| p.builtin).count())),
            Err(e) => return self.add("profiles", "list", Status::Fail, e),
        }
        let undo = Undo::DeleteProfile { id: PROBE.into() };
        if !self.remember(undo.clone()) {
            return;
        }
        let r: Result<(), String> = async {
            call(p.create(PROBE, "balanced")).await?;
            call(p.set_option(PROBE, "cpu_priority", false)).await?;
            call(p.set_pause_apps(PROBE, &[PROBE])).await?;
            let l = call(p.list()).await?;
            let mine = l.iter().find(|x| x.id == PROBE).ok_or("the new profile is not listed")?;
            if mine.cpu_priority || mine.pause_apps != [PROBE] {
                return Err("the changed settings did not stick".into());
            }
            Ok(())
        }
        .await;
        self.add("profiles", "create, change and delete one", if r.is_ok() { Status::Pass } else { Status::Fail }, r.err().unwrap_or_else(|| "worked".into()));
        self.undo_profile(&p, &undo).await;
    }

    async fn undo_profile(&mut self, p: &ag::ProfilesProxy<'_>, undo: &Undo) {
        let gone = match call(p.list()).await {
            Ok(l) if !l.iter().any(|x| x.id == PROBE) => true,
            _ => call(p.delete(PROBE)).await.is_ok() && call(p.list()).await.is_ok_and(|l| !l.iter().any(|x| x.id == PROBE)),
        };
        if gone {
            self.forget(undo);
        }
    }

    async fn games(&mut self) {
        let Some(c) = self.need_agent("games") else { return };
        let Ok(g) = ag::GamesProxy::new(&c).await else { return };
        let (r, ms) = timed(g.rescan()).await;
        match r {
            Ok(_) => self.bench("Game library scan", format!("{ms:.0} ms")),
            Err(e) => self.add("games", "scan", Status::Fail, e),
        }
        match call(g.installed()).await {
            Ok(list) => {
                let mut by: std::collections::BTreeMap<String, usize> = Default::default();
                for x in &list {
                    *by.entry(x.launcher.clone()).or_default() += 1;
                }
                let per: Vec<String> = by.iter().map(|(l, n)| format!("{l} {n}")).collect();
                self.add("games", "installed games found", if list.is_empty() { Status::Info } else { Status::Pass }, if per.is_empty() { "none (no Steam, Lutris, Heroic or Bottles library)".into() } else { per.join(", ") });
                if let Some(appid) = list.iter().find_map(|x| x.id.strip_prefix("steam:").and_then(|n| n.parse::<u32>().ok())) {
                    match call(g.launch_options(appid)).await {
                        Ok(l) => self.add("games", "Steam launch options", Status::Pass, format!("read (Steam {}, {})", if l.steam_running { "running" } else { "closed" }, if l.flatpak { "Flatpak" } else { "native" })),
                        Err(e) => self.add("games", "Steam launch options", Status::Fail, e),
                    }
                }
            }
            Err(e) => self.add("games", "installed games found", Status::Fail, e),
        }
        match call(g.running()).await {
            Ok(r) => self.add("games", "running games", Status::Pass, format!("{} running now", r.len())),
            Err(e) => self.add("games", "running games", Status::Fail, e),
        }
        self.manual("Game detection and game mode with a real game", "start a game: it should be detected within seconds and game mode should turn on");
    }

    async fn playtime(&mut self) {
        let Some(c) = self.need_agent("playtime") else { return };
        let Ok(p) = ag::PlaytimeProxy::new(&c).await else { return };
        match call(p.summary(0)).await {
            Ok(s) => self.add("playtime", "summary", Status::Pass, format!("{} games and apps with time", s.len())),
            Err(e) => self.add("playtime", "summary", Status::Fail, e),
        }
        match call(p.sessions(now_ms().saturating_sub(7 * 86_400_000))).await {
            Ok(s) => self.add("playtime", "sessions this week", Status::Pass, format!("{}", s.len())),
            Err(e) => self.add("playtime", "sessions this week", Status::Fail, e),
        }
        match call(p.running_programs()).await {
            Ok(s) => self.add("playtime", "open programs seen", if s.is_empty() { Status::Warn } else { Status::Pass }, format!("{}", s.len())),
            Err(e) => self.add("playtime", "open programs seen", Status::Fail, e),
        }
        let undo = Undo::UntrackApp { exe: PROBE.into() };
        if !self.remember(undo.clone()) {
            return;
        }
        let r: Result<(), String> = async {
            call(p.track_app(PROBE, "Unix Guardian test")).await?;
            if !call(p.tracked_apps()).await?.iter().any(|a| a.exe == PROBE) {
                return Err("the app is not listed after adding it".into());
            }
            Ok(())
        }
        .await;
        let gone = call(p.untrack_app(PROBE)).await.is_ok() && call(p.tracked_apps()).await.is_ok_and(|l| !l.iter().any(|a| a.exe == PROBE));
        if gone {
            self.forget(&undo);
        }
        let r = r.and_then(|_| if gone { Ok(()) } else { Err("could not remove it again".into()) });
        self.add("playtime", "track an app and stop", if r.is_ok() { Status::Pass } else { Status::Fail }, r.err().unwrap_or_else(|| "worked".into()));
    }

    async fn history(&mut self) {
        let Some(c) = self.need_agent("history") else { return };
        let Ok(m) = ag::MonitorProxy::new(&c).await else { return };
        let now = now_ms();
        match call(m.history(now - 3_600_000, now, 60)).await {
            Ok(h) => {
                let st = if h.is_empty() { Status::Warn } else { Status::Pass };
                let filled = |f: &dyn Fn(&ug_proto::HistoryPoint) -> f64| h.iter().any(|p| !f(p).is_nan());
                let cols: Vec<&str> = [
                    ("CPU °C", filled(&|p| p.cpu_temp_c)),
                    ("GPU °C", filled(&|p| p.gpu_temp_c)),
                    ("CPU W", filled(&|p| p.cpu_w)),
                    ("disk", filled(&|p| p.disk_busy_pct)),
                    ("battery", filled(&|p| p.battery_pct)),
                ]
                .iter()
                .filter(|c| c.1)
                .map(|c| c.0)
                .collect();
                let what = if h.is_empty() { "nothing recorded in the last hour (normal right after installing)".into() } else { format!("{} points in the last hour; has {}", h.len(), cols.join(", ")) };
                self.add("history", "recorded readings", st, what);
            }
            Err(e) => self.add("history", "recorded readings", Status::Fail, e),
        }
        match call(m.traffic(now - 7 * 86_400_000)).await {
            Ok(d) => self.add("history", "internet use per day", Status::Pass, format!("{} days", d.len())),
            Err(e) => self.add("history", "internet use per day", Status::Fail, e),
        }
        match call(m.traffic_buckets(now - 3_600_000, now, 600_000)).await {
            Ok(b) => self.add("history", "internet use this hour", Status::Pass, format!("{} buckets", b.len())),
            Err(e) => self.add("history", "internet use this hour", Status::Fail, e),
        }
        match call(m.fps()).await {
            Ok(f) => self.add("fps", "frame rate reading", Status::Info, format!("state {}{}", f.state, if f.mangohud_version.is_empty() { ", MangoHud not found".into() } else { format!(", MangoHud {}", f.mangohud_version) })),
            Err(e) => self.add("fps", "frame rate reading", Status::Fail, e),
        }
        match call(m.fps_sessions(now - 7 * 86_400_000)).await {
            Ok(s) => self.add("fps", "game sessions this week", Status::Pass, format!("{}", s.len())),
            Err(e) => self.add("fps", "game sessions this week", Status::Fail, e),
        }
    }

    async fn ping(&mut self) {
        let Some(c) = self.need_agent("ping") else { return };
        let Ok(p) = ag::PingProxy::new(&c).await else { return };
        match call(p.recent(now_ms() - 60_000)).await {
            Ok(r) => {
                if r.targets.is_empty() {
                    return self.add("ping", "targets", Status::Warn, "nothing to ping (no network?)");
                }
                for t in &r.targets {
                    let s = &t.stats;
                    let st = if s.received > 0 { Status::Pass } else if t.problem.is_empty() { Status::Info } else { Status::Warn };
                    let via = if t.through_vpn { ", through a VPN" } else { "" };
                    let problem = if t.problem.is_empty() { String::new() } else { format!(" — {}", t.problem) };
                    self.add("ping", &t.label, st, format!("{}/{} answered{via}{problem}", s.received, s.sent));
                    if s.received > 0 {
                        self.bench(&format!("Ping {}", t.label), format!("{:.1} ms median, {:.1} ms jitter, {:.0} % lost", s.median_ms, s.jitter_ms, s.loss_pct));
                    }
                }
            }
            Err(e) => self.add("ping", "recent", Status::Fail, e),
        }
    }

    async fn power(&mut self) {
        let Some(c) = self.need_daemon("power") else { return };
        let Some(p) = power_proxy(Some(&c)).await else { return };
        let caps = match call(p.capabilities()).await {
            Ok(caps) => caps,
            Err(e) => return self.add("power", "actions", Status::Fail, e),
        };
        let on: Vec<&str> = caps.iter().filter(|c| c.available).map(|c| c.action.as_str()).collect();
        let off: Vec<String> = caps.iter().filter(|c| !c.available).map(|c| format!("{} ({})", c.action, c.reason)).collect();
        self.add("power", "actions offered", Status::Pass, format!("{}{}", on.join(", "), if off.is_empty() { String::new() } else { format!("; not offered: {}", off.join(", ")) }));
        self.manual("Shut down, restart and sleep now", "the test never does these: try them from the System control page when convenient");
        // Scheduled a long way ahead and cancelled at once: tests the
        // schedule file and the service's permission without any risk of it
        // firing.
        if let Some(action) = ["suspend", "power-off", "reboot"].into_iter().find(|a| on.contains(a)) {
            let undo = Undo::CancelPowerJobs { note: PROBE.into() };
            if self.remember(undo.clone()) {
                let at = now_ms() + 300 * 86_400_000;
                let r: Result<(), String> = async {
                    let job = call(p.schedule(action, at, "once", PROBE)).await?;
                    if !call(p.jobs()).await?.iter().any(|j| j.id == job.id) {
                        return Err("the job is not listed".into());
                    }
                    Ok(())
                }
                .await;
                let cleared = cancel_probe_jobs(&p).await;
                if cleared.is_ok() {
                    self.forget(&undo);
                }
                let r = r.and(cleared);
                self.add("power", "schedule and cancel", if r.is_ok() { Status::Pass } else { Status::Fail }, r.err().unwrap_or_else(|| format!("{action} 300 days ahead, cancelled")));
            }
        }
        match call(p.usage()).await {
            Ok(u) => self.add("power", "power use", Status::Pass, format!("CPU {}, GPU {}, battery {}, CPU energy recorded: {}", opt(u.cpu_w, "W"), opt(u.gpu_w, "W"), opt(u.battery_w, "W"), u.cpu_energy_recorded)),
            Err(e) => self.add("power", "power use", Status::Fail, e),
        }
        match call(p.battery()).await {
            Ok(b) if b.supported => {
                self.add("power", "battery care", Status::Pass, format!("method {}, mode {}, limit {} %, {}", b.method, b.mode, b.limit, if b.adjustable { "adjustable" } else { "fixed" }));
                self.manual("Battery charge limit", "changes how the battery charges: set a limit on the System control page and watch it stop charging");
            }
            Ok(b) => self.add("power", "battery care", Status::Info, format!("not available: {}", or_none(&b.reason))),
            Err(e) => self.add("power", "battery care", Status::Fail, e),
        }
        self.wake_alarm(&p).await;
    }

    async fn wake_alarm(&mut self, p: &dm::PowerProxy<'_>) {
        if !call(p.wake_alarm_supported()).await.unwrap_or(false) {
            return self.add("power", "wake alarm", Status::Info, "this machine has no settable wake alarm");
        }
        if call(p.wake_alarm()).await.unwrap_or(0) != 0 {
            return self.add("power", "wake alarm", Status::Skip, "an alarm is already set; it was not touched");
        }
        let at = test_alarm_at(now_ms());
        let undo = Undo::ClearWakeAlarm { at_unix_ms: at };
        if !self.remember(undo.clone()) {
            return;
        }
        let r: Result<(), String> = async {
            call(p.set_wake_alarm(at)).await?;
            let got = call(p.wake_alarm()).await?;
            if got.abs_diff(at) > 2000 {
                return Err(format!("set for {at}, reads back {got}"));
            }
            Ok(())
        }
        .await;
        let cleared = call(p.clear_wake_alarm()).await.is_ok() && call(p.wake_alarm()).await.is_ok_and(|v| v == 0);
        if cleared {
            self.forget(&undo);
        }
        let r = r.and_then(|_| if cleared { Ok(()) } else { Err("could not clear it again".into()) });
        self.add("power", "wake alarm set and clear", if r.is_ok() { Status::Pass } else { Status::Fail }, r.err().unwrap_or_else(|| "worked".into()));
    }

    async fn fans(&mut self) {
        let Some(c) = self.need_daemon("fans") else { return };
        let Ok(f) = dm::CoolingProxy::new(&c).await else { return };
        match call(f.state()).await {
            Ok(s) => {
                let spinning = s.fans.iter().filter(|f| f.rpm > 0 || f.pct > 0.0).count();
                let modes: Vec<&str> = s.fan_modes.iter().map(|m| m.1.as_str()).collect();
                let notes = if s.notes.is_empty() { String::new() } else { format!(" — {}", s.notes.join("; ")) };
                self.add(
                    "fans",
                    "fans and modes",
                    if s.fans.is_empty() && modes.is_empty() { Status::Info } else { Status::Pass },
                    format!("{} fans ({spinning} reading), modes: {}{}{notes}", s.fans.len(), if modes.is_empty() { "none".into() } else { modes.join(", ") }, if s.simulated { ", SIMULATED" } else { "" }),
                );
                if !s.fans.is_empty() || !modes.is_empty() {
                    self.manual("Fan curves and fan modes", "change the cooling: try them on the System control page while watching the temperatures");
                }
            }
            Err(e) => self.add("fans", "fans and modes", Status::Fail, e),
        }
    }

    async fn net(&mut self) {
        let Some(c) = self.need_daemon("internet control") else { return };
        let Ok(n) = dm::NetProxy::new(&c).await else { return };
        let st = match call(n.status()).await {
            Ok(s) => s,
            Err(e) => return self.add("internet control", "status", Status::Fail, e),
        };
        let fw = if st.firewalls.is_empty() { String::new() } else { format!(", firewalls: {}", st.firewalls.join(", ")) };
        let shaping = if st.shaping_problems.is_empty() { String::new() } else { format!(", speed caps: {}", st.shaping_problems.join("; ")) };
        let vpn = if st.vpn.is_empty() { String::new() } else { ", a VPN is on".into() };
        self.add(
            "internet control",
            "status",
            if st.available { Status::Pass } else { Status::Warn },
            format!("{}{fw}{shaping}{vpn}{}", if st.available { "available".into() } else { format!("not available: {}", st.problem) }, if st.simulated { ", SIMULATED" } else { "" }),
        );
        match call(n.apps()).await {
            Ok(a) => self.add("internet control", "apps using the internet", if a.is_empty() { Status::Warn } else { Status::Pass }, format!("{}", a.len())),
            Err(e) => self.add("internet control", "apps using the internet", Status::Fail, e),
        }
        if !st.available {
            return;
        }
        let undo = Undo::RemoveNetRule { app: PROBE.into() };
        if !self.remember(undo.clone()) {
            return;
        }
        let r: Result<(), String> = async {
            call(n.set_rule(PROBE, true, 0, 0, "always")).await?;
            if !call(n.rules()).await?.iter().any(|r| r.app == PROBE && r.block) {
                return Err("the rule is not listed".into());
            }
            Ok(())
        }
        .await;
        let gone = call(n.remove_rule(PROBE)).await.is_ok() && call(n.rules()).await.is_ok_and(|l| !l.iter().any(|r| r.app == PROBE));
        if gone {
            self.forget(&undo);
        }
        let r = r.and_then(|_| if gone { Ok(()) } else { Err("could not remove the rule again".into()) });
        self.add("internet control", "add and remove a block rule", if r.is_ok() { Status::Pass } else { Status::Fail }, r.err().unwrap_or_else(|| "worked (for a made-up program)".into()));
        self.manual("Blocking or slowing a real app", "cuts that app off: try it with a browser, then allow it again");
    }

    async fn overlay(&mut self) {
        let Some(c) = self.need_agent("overlay") else { return };
        let Ok(o) = ag::OverlayProxy::new(&c).await else { return };
        match call(o.state()).await {
            Ok(s) if !s.installed => {
                self.add("overlay", "MangoHud", Status::Skip, format!("not installed; installing is a package change, not done by the test ({})", or_none(&s.install_hint)));
            }
            Ok(s) => {
                self.add("overlay", "MangoHud", Status::Pass, format!("installed, {}", if s.set_up { "set up" } else { "not set up yet" }));
                if s.set_up {
                    // Writing the same settings back exercises the file
                    // handling without changing anything the user sees.
                    let r = call(o.set(&s.settings)).await;
                    self.add("overlay", "save settings", if r.is_ok() { Status::Pass } else { Status::Fail }, r.err().unwrap_or_else(|| "saved unchanged".into()));
                }
                self.manual("The overlay in a game", "start a game and press the overlay key");
            }
            Err(e) => self.add("overlay", "MangoHud", Status::Fail, e),
        }
    }

    async fn recorder(&mut self) {
        let Some(c) = self.need_agent("recorder") else { return };
        let Ok(r) = ag::RecorderProxy::new(&c).await else { return };
        let s = match call(r.state()).await {
            Ok(s) => s,
            Err(e) => return self.add("recorder", "state", Status::Fail, e),
        };
        if s.engine.is_empty() {
            self.add("recorder", "gpu-screen-recorder", Status::Skip, "not set up; setting it up downloads it, which the test does not do");
            return self.manual("Screen recording", "set up the recorder on its page, then record a few seconds");
        }
        self.add("recorder", "gpu-screen-recorder", Status::Pass, format!("{}; can capture: {}", s.engine, s.captures.join(", ")));
        if s.mode != "idle" {
            return self.add("recorder", "record 3 seconds", Status::Skip, format!("the recorder is in use ({})", s.mode));
        }
        if recording_asks_first(&s.settings.capture, wayland_session()) {
            self.add("recorder", "record 3 seconds", Status::Skip, "recording here opens the desktop's screen picker or asks for a password first, so the test does not start one");
            return self.manual("Screen recording", "record a few seconds from the Recorder page and allow it in the desktop's dialog");
        }
        let file = match call(r.start()).await {
            Ok(f) => f,
            Err(e) => return self.add("recorder", "record 3 seconds", Status::Fail, e),
        };
        let undo = Undo::DeleteRecording { file: file.clone() };
        let remembered = self.remember(undo.clone());
        tokio::time::sleep(if self.quick { Duration::from_millis(500) } else { Duration::from_secs(3) }).await;
        let stopped = call(r.stop()).await;
        let problem = call(r.state()).await.map(|s| s.problem).unwrap_or_default();
        let size = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
        let removed = std::fs::remove_file(&file).is_ok() || !std::path::Path::new(&file).exists();
        if removed && remembered {
            self.forget(&undo);
        }
        let res = match stopped {
            Err(e) => Err(e),
            Ok(_) if size == 0 => Err(format!("no video was written{}", if problem.is_empty() { String::new() } else { format!(": {problem}") })),
            Ok(_) => Ok(()),
        };
        if res.is_ok() {
            self.bench("Recording", format!("{:.1} MB for {} s at {} fps, {}", size as f64 / 1_048_576.0, if self.quick { 0.5 } else { 3.0 }, s.settings.fps, s.settings.codec));
        }
        self.add("recorder", "record 3 seconds", if res.is_ok() { Status::Pass } else { Status::Fail }, res.err().unwrap_or_else(|| "recorded and deleted".into()));
    }

    async fn misc(&mut self) {
        let Some(c) = self.need_agent("settings") else { return };
        if let Ok(sh) = ag::ShortcutsProxy::new(&c).await {
            match call(sh.state()).await {
                Ok(s) => {
                    let problem = if s.problem.is_empty() { String::new() } else { format!(" — {}", s.problem) };
                    self.add("shortcuts", "global shortcuts", Status::Info, format!("{}, status {}{problem}", if s.enabled { "on" } else { "off" }, or_none(&s.status)));
                    self.manual("Global shortcuts", "turning them on opens the desktop's confirmation dialog: try it in Settings");
                }
                Err(e) => self.add("shortcuts", "global shortcuts", Status::Fail, e),
            }
        }
        if let Ok(se) = ag::SettingsProxy::new(&c).await {
            let r: Result<(), String> = async {
                let s = call(se.state()).await?;
                call(se.set(&s.settings)).await?;
                let again = call(se.state()).await?;
                if again.settings.scan_interval_s != s.settings.scan_interval_s || again.settings.notifications != s.settings.notifications {
                    return Err("settings changed when saved unchanged".into());
                }
                Ok(())
            }
            .await;
            self.add("settings", "read and save", if r.is_ok() { Status::Pass } else { Status::Fail }, r.err().unwrap_or_else(|| "saved unchanged".into()));
        }
        if let Ok(u) = ag::UpdateProxy::new(&c).await {
            match call(u.state()).await {
                Ok(s) => {
                    let last = if s.checked_unix_ms == 0 { "never checked".into() } else { format!("last checked {}", utc_stamp(s.checked_unix_ms)) };
                    self.add("updates", "update state", Status::Info, format!("installed as {}, {last}{}", or_none(&s.kind), if s.can_install { ", can update itself" } else { "" }));
                }
                Err(e) => self.add("updates", "update state", Status::Fail, e),
            }
        }
        self.manual("Checking for and installing updates", "the test does not use the internet for this: try it on the Updates page");
    }

    async fn drivers(&mut self) {
        let root = ug_hw::SysRoot::real();
        let mut env = ug_driver::Env::from_process();
        let mut ops = None;
        if let Some(c) = self.daemon.clone().filter(|_| self.inventory.is_some()) {
            if let Ok(d) = dm::DriverProxy::new(&c).await {
                if env.kernel_log.is_empty() {
                    env.kernel_log = call(d.firmware_log()).await.unwrap_or_default();
                }
                ops = call(d.ops()).await.ok();
            }
        }
        let t = Instant::now();
        let r = block_in_place_or(move || ug_driver::diagnose(&root, &env));
        let ms = t.elapsed().as_millis();
        let count = |s: &str| r.findings.iter().filter(|f| f.severity == s).count();
        self.add(
            "drivers",
            "hardware and driver check",
            Status::Pass,
            format!("{} facts, {} findings ({} critical, {} important) in {ms} ms", r.facts.len(), r.findings.len(), count("critical"), count("important")),
        );
        for f in &r.findings {
            self.add("drivers", &f.id, Status::Info, format!("{} [{}, {}]{}", f.title_en, f.severity, f.category, if f.fix_auto { ", can be fixed automatically" } else { "" }));
        }
        let family = format!("{:?}", ug_core::distro::Distro::detect(&ug_hw::SysRoot::real()).family).to_lowercase();
        let snap = ug_driver::snapshot::detect(&ug_hw::SysRoot::real(), &family);
        self.add("drivers", "snapshot tool", if snap.tool.is_some() { Status::Pass } else { Status::Info }, snap.summary);
        if let Some(ops) = ops {
            self.add("drivers", "changes made so far", Status::Info, format!("{}", ops.len()));
        }
        self.manual("Driver fixes and boot parameters", "ask for the password and may need a restart: only use them for a real problem");
    }

    async fn cleanup(&mut self) {
        let done = self.put_back().await;
        if !done.is_empty() {
            self.add("lab", "put back late", Status::Warn, done.join("; "));
        }
        for u in self.journal.items().to_vec() {
            self.leftovers.push(describe(&u));
        }
        let mins = self.started.elapsed().as_secs_f64();
        for ((comm, label), before) in [("ug-daemon", "Service"), ("ug-agent", "Agent")].into_iter().zip([self.cpu_at_start.0, self.cpu_at_start.1]) {
            if let (Some((rss, cpu)), Some(b)) = (proc_usage(comm), before) {
                self.bench(&format!("{label} during the test"), format!("{:.2} CPU-seconds in {mins:.0} s, {rss:.1} MB now", cpu - b));
            }
        }
        self.manual("Anything that needs a log-out, log-in or restart", "the test never logs out, restarts or changes the whole desktop session");
    }

    /// Undoes everything in the journal; returns what was put back.
    pub(crate) async fn put_back(&mut self) -> Vec<String> {
        let mut done = Vec::new();
        for u in self.journal.items().to_vec() {
            if self.undo(&u).await {
                self.forget(&u);
                done.push(describe(&u));
            }
        }
        done
    }

    async fn undo(&self, u: &Undo) -> bool {
        let (d, a) = (self.daemon.as_ref(), self.agent.as_ref());
        match u {
            Undo::DeleteProfile { id } => {
                let Some(p) = proxy(a, ag::ProfilesProxy::new).await else { return false };
                match call(p.list()).await {
                    Ok(l) if !l.iter().any(|x| &x.id == id) => true,
                    Ok(_) => call(p.delete(id)).await.is_ok(),
                    Err(_) => false,
                }
            }
            Undo::UntrackApp { exe } => {
                let Some(p) = proxy(a, ag::PlaytimeProxy::new).await else { return false };
                call(p.untrack_app(exe)).await.is_ok()
            }
            Undo::CancelPowerJobs { .. } => {
                let Some(p) = power_proxy(d).await else { return false };
                cancel_probe_jobs(&p).await.is_ok()
            }
            Undo::RemoveNetRule { app } => {
                let Some(n) = proxy(d, dm::NetProxy::new).await else { return false };
                call(n.remove_rule(app)).await.is_ok()
            }
            Undo::ClearWakeAlarm { at_unix_ms } => {
                let Some(p) = power_proxy(d).await else { return false };
                match call(p.wake_alarm()).await {
                    // Someone set another alarm since: theirs stays.
                    Ok(v) if v == 0 || v.abs_diff(*at_unix_ms) > 2000 => true,
                    Ok(_) => call(p.clear_wake_alarm()).await.is_ok(),
                    Err(_) => false,
                }
            }
            Undo::DeleteRecording { file } => {
                if let Some(r) = proxy(a, ag::RecorderProxy::new).await {
                    if call(r.state()).await.is_ok_and(|s| s.mode == "recording" && &s.file == file) {
                        let _ = call(r.stop()).await;
                    }
                }
                let p = std::path::Path::new(file);
                !p.exists() || std::fs::remove_file(p).is_ok()
            }
        }
    }

    pub(crate) fn finish(self) -> Report {
        let mut rep = Report { leftovers: self.leftovers.clone(), checks: self.checks.clone(), ..Default::default() };
        for c in &self.checks {
            match c.status {
                Status::Pass => rep.passed += 1,
                Status::Warn => rep.warned += 1,
                Status::Fail => rep.failed += 1,
                Status::Skip => rep.skipped += 1,
                Status::Info => {}
            }
        }
        let d = ug_core::distro::Distro::detect(&ug_hw::SysRoot::real());
        let mut keep: Vec<&str> = d.id_like.iter().map(String::as_str).collect();
        keep.push(&d.id);
        let sc = Scrubber::from_session(&keep);
        rep.text = sc.scrub(&self.text(&rep));
        rep.checks.iter_mut().for_each(|c| c.detail = sc.scrub(&c.detail));
        rep
    }

    fn text(&self, rep: &Report) -> String {
        use std::fmt::Write;
        let mut o = String::new();
        let _ = writeln!(o, "Unix Guardian test lab report");
        let _ = writeln!(o, "=============================");
        let _ = writeln!(o, "Made {}, took {} s. Report format {REPORT_FORMAT}.", utc_stamp(self.started_ms), self.started.elapsed().as_secs());
        let _ = writeln!(o, "Personal details were removed (home folder, user and host names, disk IDs,");
        let _ = writeln!(o, "MAC and IP addresses). Please read it before sending.");
        let _ = writeln!(o);
        let _ = writeln!(o, "RESULT: {} passed, {} warnings, {} failed, {} skipped", rep.passed, rep.warned, rep.failed, rep.skipped);
        let _ = writeln!(o);
        let _ = writeln!(o, "THIS COMPUTER");
        let w = self.env.iter().map(|e| e.0.chars().count()).max().unwrap_or(0);
        for (k, v) in &self.env {
            let _ = writeln!(o, "  {k:<w$}  {v}");
        }
        let _ = writeln!(o);
        let _ = writeln!(o, "CHECKS");
        for c in &self.checks {
            let _ = writeln!(o, "  [{}] {}: {} — {}", c.status.tag(), c.area, c.name, c.detail);
        }
        let _ = writeln!(o);
        if !self.benches.is_empty() {
            let _ = writeln!(o, "MEASUREMENTS");
            let w = self.benches.iter().map(|e| e.0.chars().count()).max().unwrap_or(0);
            for (k, v) in &self.benches {
                let _ = writeln!(o, "  {k:<w$}  {v}");
            }
            let _ = writeln!(o);
        }
        let _ = writeln!(o, "NOT TESTED AUTOMATICALLY (needs a manual test)");
        for (what, why) in &self.manual {
            let _ = writeln!(o, "  - {what}: {why}");
        }
        let _ = writeln!(o);
        let _ = writeln!(o, "PUT BACK");
        if rep.leftovers.is_empty() {
            let _ = writeln!(o, "  Everything the test turned on was turned off again.");
        } else {
            for l in &rep.leftovers {
                let _ = writeln!(o, "  NOT put back: {l}");
            }
        }
        let _ = writeln!(o);
        let _ = writeln!(o, "Send this report to {}", ug_core::BUG_REPORT_EMAIL);
        o
    }
}

fn or_none(s: &str) -> String {
    if s.is_empty() {
        "unknown".into()
    } else {
        s.to_string()
    }
}

/// `wlan`, `ethernet`… from an interface name; the name itself can carry a
/// MAC address (`wlx…`).
fn iface_kind(name: &str) -> &'static str {
    match name.get(..2) {
        Some("wl") => "Wi-Fi",
        Some("en") | Some("et") => "wired",
        Some("ww") => "mobile",
        _ if name.starts_with("tun") || name.starts_with("wg") => "VPN",
        _ => "other",
    }
}

fn describe(u: &Undo) -> String {
    match u {
        Undo::DeleteProfile { id } => format!("test profile {id} deleted"),
        Undo::UntrackApp { exe } => format!("test app {exe} no longer tracked"),
        Undo::CancelPowerJobs { .. } => "test power action cancelled".into(),
        Undo::RemoveNetRule { app } => format!("internet rule for {app} removed"),
        Undo::ClearWakeAlarm { .. } => "test wake alarm cleared".into(),
        Undo::DeleteRecording { .. } => "test recording deleted".into(),
    }
}

async fn cancel_probe_jobs(p: &dm::PowerProxy<'_>) -> Result<(), String> {
    for j in call(p.jobs()).await?.into_iter().filter(|j| j.note == PROBE) {
        call(p.cancel(j.id)).await?;
    }
    if call(p.jobs()).await?.iter().any(|j| j.note == PROBE) {
        return Err("the test job is still scheduled".into());
    }
    Ok(())
}

/// The wake alarm is a property the service does not signal: read it fresh.
async fn power_proxy(conn: Option<&zbus::Connection>) -> Option<dm::PowerProxy<'static>> {
    dm::PowerProxy::builder(conn?).cache_properties(zbus::proxy::CacheProperties::No).build().await.ok()
}

async fn proxy<'a, P, F>(conn: Option<&'a zbus::Connection>, new: impl FnOnce(&'a zbus::Connection) -> F) -> Option<P>
where
    F: Future<Output = zbus::Result<P>>,
{
    new(conn?).await.ok()
}

/// Runs blocking work without stalling the runtime's other tasks when it
/// can (a multi-threaded runtime), and in place otherwise.
fn block_in_place_or<T>(f: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current().map(|h| h.runtime_flavor()) {
        Ok(tokio::runtime::RuntimeFlavor::MultiThread) => tokio::task::block_in_place(f),
        _ => f(),
    }
}

/// When the test wake alarm is set for: six hours ahead. PC clocks
/// (rtc_cmos) often refuse an alarm more than a day ahead; it is cleared at
/// once, and the undo journal clears it after a crash.
pub(crate) fn test_alarm_at(now_ms: u64) -> u64 {
    (now_ms + 6 * 3_600_000) / 1000 * 1000
}

/// Whether starting a recording would open a dialog or ask for a password:
/// the portal always does (automatic on Wayland is the portal), and on
/// Wayland a monitor is captured through a helper that needs the password.
pub(crate) fn recording_asks_first(capture: &str, wayland: bool) -> bool {
    capture == "portal" || wayland
}

/// Whether a GPU driver reports a temperature of its own. Intel's
/// integrated graphics share the CPU's package and have none.
pub(crate) fn gpu_has_own_temperature(driver: &str) -> bool {
    !matches!(driver, "i915" | "xe")
}

fn wayland_session() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty()) || std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v == "wayland")
}
