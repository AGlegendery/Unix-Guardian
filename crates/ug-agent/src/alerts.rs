//! Threshold alerts: a desktop notification when the CPU or GPU runs hot,
//! memory fills up or a disk is nearly full.
//!
//! A reading has to stay above its limit for the whole sensitivity window
//! (3–30 s) before it alerts, so a compile burst or a loading screen never
//! does; a reading that dips below, goes missing or is interrupted by a
//! pause (suspend) starts the window again. After an alert the same thing
//! is quiet for the cooldown, which is counted from the recent-alerts list
//! on disk, so restarting the agent does not repeat an alert.
//!
//! Readings: the CPU package sensor, memory and the chosen mounts are read
//! here (cheap files and one `statvfs` each); GPU temperatures come from the
//! daemon's snapshot, which never wakes a runtime-suspended card. A sleeping
//! card is reported `asleep` and checked again when it wakes on its own.
//! Hotspot and memory temperatures are offered only once a card has been
//! seen reporting them. Nothing is read at all while every alert is off.
//!
//! Game mode's do not disturb holds the desktop notification, not the
//! alert: it is still recorded in the recent list, marked as held.

use crate::settings::Settings;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use ug_hw::SysRoot;
use ug_proto::{AlertEvent, AlertMount, AlertSensor, AlertSettings, AlertsState, ALERT_COOLDOWNS};

/// How often readings are taken while an alert is on.
pub const EVERY: Duration = Duration::from_secs(2);
/// Recent alerts kept on disk.
const KEEP: usize = 50;
/// Chosen mounts at most (the picker lists the machine's own).
const MAX_MOUNTS: usize = 16;

/// Checks settings before they are saved.
pub fn validate(s: &AlertSettings) -> Result<(), String> {
    let range = |what: &str, v: u32, lo: u32, hi: u32| {
        if (lo..=hi).contains(&v) {
            Ok(())
        } else {
            Err(format!("{what} must be {lo} to {hi}, not {v}"))
        }
    };
    range("the CPU temperature limit", s.cpu_temp_c, 50, 110)?;
    range("the GPU temperature limit", s.gpu_temp_c, 50, 110)?;
    range("the GPU hotspot limit", s.gpu_hotspot_c, 60, 120)?;
    range("the GPU memory temperature limit", s.gpu_vram_c, 60, 120)?;
    range("the memory limit", s.mem_pct, 50, 99)?;
    range("the disk limit", s.disk_pct, 50, 99)?;
    range("the sensitivity", s.sustain_s, 3, 30)?;
    if !ALERT_COOLDOWNS.contains(&s.cooldown_min) {
        return Err(format!(
            "the cooldown must be one of {ALERT_COOLDOWNS:?} minutes, not {}",
            s.cooldown_min
        ));
    }
    if s.disk_mounts.len() > MAX_MOUNTS {
        return Err(format!("at most {MAX_MOUNTS} mounts can be watched"));
    }
    if let Some(m) = s.disk_mounts.iter().find(|m| !m.starts_with('/')) {
        return Err(format!("{m:?} is not a mount point (it must start with /)"));
    }
    Ok(())
}

/// The limit for a kind, and whether that alert is on.
fn rule(s: &AlertSettings, kind: &str) -> (bool, u32) {
    match kind {
        "cpu_temp" => (s.cpu_temp_on, s.cpu_temp_c),
        "gpu_temp" => (s.gpu_temp_on, s.gpu_temp_c),
        "gpu_hotspot" => (s.gpu_hotspot_on, s.gpu_hotspot_c),
        "gpu_vram" => (s.gpu_vram_on, s.gpu_vram_c),
        "memory" => (s.mem_on, s.mem_pct),
        "disk" => (s.disk_on, s.disk_pct),
        _ => (false, 0),
    }
}

fn any_on(s: &AlertSettings) -> bool {
    s.cpu_temp_on || s.gpu_temp_on || s.gpu_hotspot_on || s.gpu_vram_on || s.mem_on || s.disk_on
}

fn gpu_on(s: &AlertSettings) -> bool {
    s.gpu_temp_on || s.gpu_hotspot_on || s.gpu_vram_on
}

/// One sensor as read this time: status as in [`AlertSensor::status`].
#[derive(Debug, Clone, PartialEq)]
pub struct Observed {
    pub kind: &'static str,
    pub subject: String,
    pub status: &'static str,
    pub value: Option<f64>,
}

impl Observed {
    pub fn ok(kind: &'static str, subject: &str, value: f64) -> Self {
        Observed {
            kind,
            subject: subject.into(),
            status: "ok",
            value: Some(value),
        }
    }
    pub fn not(kind: &'static str, subject: &str, status: &'static str) -> Self {
        Observed {
            kind,
            subject: subject.into(),
            status,
            value: None,
        }
    }
}

/// An alert to show.
#[derive(Debug, Clone, PartialEq)]
pub struct Fire {
    pub kind: &'static str,
    pub subject: String,
    pub value: f64,
    pub threshold: u32,
}

/// The sustained-window and cooldown logic, apart from any reading, so it
/// can be tested with made-up times.
#[derive(Default)]
pub struct Evaluator {
    /// Since when each (kind, subject) has been above its limit.
    above: HashMap<(String, String), u64>,
    /// When each (kind, subject) last alerted.
    fired: HashMap<(String, String), u64>,
    last_ms: Option<u64>,
}

impl Evaluator {
    /// Cooldowns carry over from the alerts already recorded.
    pub fn new(recent: &[AlertEvent]) -> Self {
        let mut e = Evaluator::default();
        for a in recent.iter().filter(|a| a.kind != "test") {
            let k = (a.kind.clone(), a.subject.clone());
            let t = e.fired.entry(k).or_insert(0);
            *t = (*t).max(a.ts_unix_ms);
        }
        e
    }

    /// Takes one round of readings; returns the sensors with how long each
    /// has been above its limit, and the alerts to show now.
    pub fn update(
        &mut self,
        now_ms: u64,
        seen: &[Observed],
        s: &AlertSettings,
    ) -> (Vec<AlertSensor>, Vec<Fire>) {
        // A gap far longer than the reading interval (suspend, a stalled
        // daemon) means nothing is known about the time in between.
        if self
            .last_ms
            .is_some_and(|l| now_ms.saturating_sub(l) > 4 * EVERY.as_millis() as u64)
        {
            self.above.clear();
        }
        self.last_ms = Some(now_ms);
        let sustain_ms = u64::from(s.sustain_s) * 1000;
        let cooldown_ms = u64::from(s.cooldown_min) * 60_000;
        let mut still = HashSet::new();
        let mut sensors = Vec::new();
        let mut fires = Vec::new();
        for o in seen {
            let (on, limit) = rule(s, o.kind);
            let key = (o.kind.to_string(), o.subject.clone());
            let mut above_s = 0;
            if let (true, "ok", Some(v)) = (on, o.status, o.value) {
                if v >= f64::from(limit) {
                    let since = *self.above.entry(key.clone()).or_insert(now_ms);
                    let held_ms = now_ms.saturating_sub(since);
                    above_s = (held_ms / 1000) as u32;
                    still.insert(key.clone());
                    let cooled = self
                        .fired
                        .get(&key)
                        .is_none_or(|t| now_ms.saturating_sub(*t) >= cooldown_ms);
                    if held_ms >= sustain_ms && cooled {
                        self.fired.insert(key, now_ms);
                        fires.push(Fire {
                            kind: o.kind,
                            subject: o.subject.clone(),
                            value: v,
                            threshold: limit,
                        });
                    }
                }
            }
            sensors.push(AlertSensor {
                kind: o.kind.into(),
                subject: o.subject.clone(),
                status: o.status.into(),
                value: o.value.unwrap_or(f64::NAN),
                above_s,
            });
        }
        // Below, unread, asleep or switched off: the window starts again.
        self.above.retain(|k, _| still.contains(k));
        (sensors, fires)
    }
}

/// The GPU part of a reading, from the daemon's snapshot.
pub struct GpuRead {
    pub pci: String,
    pub name: String,
    pub asleep: bool,
    pub core: Option<f64>,
    pub hotspot: Option<f64>,
    pub vram: Option<f64>,
}

/// Turns GPU readings into sensors. `seen` remembers which cards were seen
/// reporting a hotspot or memory temperature, so a sleeping card keeps
/// offering what it had (and nothing it never showed). `None`: the service
/// could not be asked.
pub fn gpu_sensors(gpus: Option<&[GpuRead]>, seen: &mut HashSet<String>) -> Vec<Observed> {
    let Some(gpus) = gpus else {
        return vec![Observed::not("gpu_temp", "", "unknown")];
    };
    if gpus.is_empty() {
        return ["gpu_temp", "gpu_hotspot", "gpu_vram"]
            .into_iter()
            .map(|k| Observed::not(k, "", "unsupported"))
            .collect();
    }
    let mut out = Vec::new();
    for g in gpus {
        let subject = if g.name.is_empty() { &g.pci } else { &g.name };
        for (kind, v) in [
            ("gpu_temp", g.core),
            ("gpu_hotspot", g.hotspot),
            ("gpu_vram", g.vram),
        ] {
            let mark = format!("{}/{kind}", g.pci);
            out.push(match (g.asleep, v) {
                (true, _) if kind == "gpu_temp" || seen.contains(&mark) => {
                    Observed::not(kind, subject, "asleep")
                }
                (true, _) => Observed::not(kind, subject, "unknown"),
                (false, Some(c)) => {
                    seen.insert(mark);
                    Observed::ok(kind, subject, c)
                }
                (false, None) => Observed::not(kind, subject, "unsupported"),
            });
        }
    }
    out
}

/// Filesystems worth watching: real storage, not the kernel's own.
const DISK_FS: [&str; 13] = [
    "ext2", "ext3", "ext4", "btrfs", "xfs", "f2fs", "vfat", "exfat", "ntfs", "ntfs3", "fuseblk",
    "zfs", "bcachefs",
];

/// Mounts the disk alert can watch, from `/proc/mounts`: one per device
/// (Btrfs subvolumes of one disk share its space; the shortest path is
/// kept), never the kernel's or a container's.
pub fn mounts(root: &SysRoot) -> Vec<(String, String)> {
    let text = std::fs::read_to_string(root.join("proc/mounts")).unwrap_or_default();
    let mut by_dev: HashMap<String, (String, String)> = HashMap::new();
    let mut order = Vec::new();
    for l in text.lines() {
        let w: Vec<&str> = l.split_whitespace().collect();
        if w.len() < 3 || !DISK_FS.contains(&w[2]) {
            continue;
        }
        // /proc/mounts writes a space in a path as \040.
        let path = w[1].replace("\\040", " ");
        if ["/proc", "/sys", "/dev", "/run", "/snap", "/var/lib/docker"]
            .iter()
            .any(|p| path == *p || path.starts_with(&format!("{p}/")))
        {
            continue;
        }
        match by_dev.get_mut(w[0]) {
            Some(e) if path.len() < e.0.len() => e.0 = path,
            Some(_) => {}
            None => {
                order.push(w[0].to_string());
                by_dev.insert(w[0].to_string(), (path, w[2].to_string()));
            }
        }
    }
    let mut out: Vec<(String, String)> = order.iter().filter_map(|d| by_dev.remove(d)).collect();
    out.sort();
    out
}

/// (percent used, size in bytes) of the filesystem at `path`, as `df`
/// counts it (used / (used + available to users)).
pub fn disk_use(path: &Path) -> Option<(f64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: a valid C string and a zeroed struct for the kernel to fill.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let f = st.f_frsize as u64;
    let used = (st.f_blocks as u64).saturating_sub(st.f_bfree as u64) * f;
    let avail = st.f_bavail as u64 * f;
    let size = st.f_blocks as u64 * f;
    (used + avail > 0).then(|| (used as f64 * 100.0 / (used + avail) as f64, size))
}

/// The disk sensors: each chosen mount, `missing` when it is not mounted.
pub fn disk_sensors(
    chosen: &[String],
    mounted: &[(String, String)],
    usage: impl Fn(&str) -> Option<(f64, u64)>,
) -> Vec<Observed> {
    chosen
        .iter()
        .map(|m| {
            // A path that is not a mount point of its own (a folder, or a
            // Btrfs subvolume hidden from the list) still has its disk's
            // numbers; only one that is gone is missing.
            let listed = mounted.iter().any(|x| &x.0 == m);
            match usage(m) {
                Some((pct, _)) => Observed::ok("disk", m, pct),
                None if listed => Observed::not("disk", m, "unknown"),
                None => Observed::not("disk", m, "missing"),
            }
        })
        .collect()
}

struct Inner {
    settings: AlertSettings,
    recent: Vec<AlertEvent>,
    eval: Evaluator,
    sensors: Vec<AlertSensor>,
    gpu_seen: HashSet<String>,
    /// The CPU package sensor, looked for once.
    cpu: Option<Option<PathBuf>>,
    daemon: Option<zbus::Connection>,
}

pub struct Alerts {
    settings_file: PathBuf,
    recent_file: PathBuf,
    root: SysRoot,
    daemon_session: bool,
    inner: Mutex<Inner>,
    changed: tokio::sync::watch::Sender<()>,
    /// The agent's notification switches: `--quiet`, Settings.
    quiet: bool,
    agent_settings: Arc<Settings>,
    /// Game mode's do not disturb hold is taken (set by the main loop).
    pub dnd: AtomicBool,
}

fn read_json<T: serde::de::DeserializeOwned>(p: &Path) -> Option<T> {
    let t = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(&t)
        .inspect_err(
            |e| tracing::warn!(file = %p.display(), error = %e, "ignoring unreadable file"),
        )
        .ok()
}

impl Alerts {
    pub fn new(
        config_dir: &Path,
        data_dir: &Path,
        root: SysRoot,
        daemon_session: bool,
        agent_settings: Arc<Settings>,
        quiet: bool,
    ) -> Self {
        let settings_file = config_dir.join("alerts.json");
        let recent_file = data_dir.join("alerts-recent.json");
        let settings = read_json::<AlertSettings>(&settings_file)
            .filter(|s| validate(s).is_ok())
            .unwrap_or_default();
        let mut recent: Vec<AlertEvent> = read_json(&recent_file).unwrap_or_default();
        recent.truncate(KEEP);
        Alerts {
            settings_file,
            recent_file,
            root,
            daemon_session,
            inner: Mutex::new(Inner {
                eval: Evaluator::new(&recent),
                settings,
                recent,
                sensors: Vec::new(),
                gpu_seen: HashSet::new(),
                cpu: None,
                daemon: None,
            }),
            changed: tokio::sync::watch::channel(()).0,
            quiet,
            agent_settings,
            dnd: AtomicBool::new(false),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn settings(&self) -> AlertSettings {
        self.lock().settings.clone()
    }

    pub fn set(&self, s: AlertSettings) -> Result<(), String> {
        validate(&s)?;
        let text = serde_json::to_string_pretty(&s).map_err(|e| e.to_string())?;
        ug_core::paths::write_atomic(&self.settings_file, text.as_bytes())
            .map_err(|e| format!("{}: {e}", self.settings_file.display()))?;
        self.lock().settings = s;
        self.changed.send_replace(());
        Ok(())
    }

    pub fn clear(&self) -> u32 {
        let n = {
            let mut i = self.lock();
            let n = i.recent.len();
            i.recent.clear();
            n
        };
        let _ = std::fs::remove_file(&self.recent_file);
        n as u32
    }

    fn record(&self, e: AlertEvent) {
        let text = {
            let mut i = self.lock();
            i.recent.insert(0, e);
            i.recent.truncate(KEEP);
            serde_json::to_string(&i.recent).unwrap_or_default()
        };
        if let Err(e) = ug_core::paths::write_atomic(&self.recent_file, text.as_bytes()) {
            tracing::warn!(error = %e, "could not save the recent alerts");
        }
    }

    /// Why a notification would be held now ("" when it would be shown).
    fn held_reason(&self) -> &'static str {
        if self.quiet || !self.agent_settings.get().notifications {
            "off"
        } else if self.dnd.load(Ordering::Relaxed) {
            "dnd"
        } else {
            ""
        }
    }

    /// Shows (or holds) an alert and records it.
    async fn deliver(
        &self,
        conn: &zbus::Connection,
        kind: &str,
        subject: &str,
        value: f64,
        threshold: u32,
        summary: &str,
        body: &str,
    ) -> AlertEvent {
        let mut held = self.held_reason().to_string();
        if held.is_empty()
            && crate::notify::show_alert(conn, summary, body)
                .await
                .is_none()
        {
            held = "no-server".into();
        }
        tracing::info!(kind, subject, value, threshold, held, "alert");
        let e = AlertEvent {
            ts_unix_ms: crate::iface::now_ms(),
            kind: kind.into(),
            subject: subject.into(),
            value,
            threshold,
            notified: held.is_empty(),
            held,
        };
        self.record(e.clone());
        e
    }

    pub async fn test(&self, conn: &zbus::Connection) -> AlertEvent {
        self.deliver(
            conn,
            "test",
            "",
            f64::NAN,
            0,
            "Test alert",
            "This is how a Unix Guardian alert looks. Set them in Monitoring, Alerts (or: ug alert).",
        )
        .await
    }

    async fn daemon(&self) -> Option<zbus::Connection> {
        if let Some(c) = self.lock().daemon.clone() {
            return Some(c);
        }
        let c = if self.daemon_session {
            zbus::Connection::session().await
        } else {
            zbus::Connection::system().await
        }
        .ok()?;
        self.lock().daemon = Some(c.clone());
        Some(c)
    }

    /// GPU readings from the daemon; `None` when it cannot be asked.
    async fn read_gpus(&self) -> Option<Vec<GpuRead>> {
        let read = async {
            let conn = self.daemon().await?;
            let list = ug_proto::daemon::GpuProxy::new(&conn)
                .await
                .ok()?
                .list()
                .await
                .ok()?;
            let snap = ug_proto::daemon::DaemonProxy::new(&conn)
                .await
                .ok()?
                .snapshot()
                .await
                .ok()?;
            Some(
                snap.gpus
                    .iter()
                    .map(|s| GpuRead {
                        pci: s.pci.clone(),
                        name: list
                            .iter()
                            .find(|g| g.pci == s.pci)
                            .map(|g| crate::tray::short_gpu_name(&g.name))
                            .unwrap_or_default(),
                        asleep: s.asleep,
                        core: s.temp(),
                        hotspot: s.hotspot(),
                        vram: (!s.temp_mem_c.is_nan()).then_some(s.temp_mem_c),
                    })
                    .collect::<Vec<_>>(),
            )
        };
        let r = tokio::time::timeout(Duration::from_secs(1), read)
            .await
            .ok()
            .flatten();
        if r.is_none() {
            self.lock().daemon = None;
        }
        r
    }

    /// Reads every sensor once. The GPU is asked only when `with_gpu`.
    async fn observe(&self, with_gpu: bool) -> Vec<Observed> {
        let s = self.settings();
        let gpus = if with_gpu {
            Some(self.read_gpus().await)
        } else {
            None
        };
        let root = self.root.clone();
        let cpu = self.lock().cpu.clone();
        let local = tokio::task::spawn_blocking(move || {
            let cpu = cpu.unwrap_or_else(|| crate::tray::cpu_sensor(&root).map(|s| s.0));
            let temp = cpu
                .as_ref()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .and_then(|t| t.trim().parse::<f64>().ok())
                .map(|m| m / 1000.0);
            let mem = ug_hw::mem::read(&root);
            let mounted = mounts(&root);
            let disks = disk_sensors(&s.disk_mounts, &mounted, |m| {
                disk_use(&root.join(m.trim_start_matches('/')))
            });
            (cpu, temp, mem, disks)
        })
        .await;
        let Ok((cpu, temp, mem, disks)) = local else {
            return Vec::new();
        };
        let mut out = Vec::new();
        out.push(match (&cpu, temp) {
            (None, _) => Observed::not("cpu_temp", "CPU", "unsupported"),
            (Some(_), Some(t)) => Observed::ok("cpu_temp", "CPU", t),
            (Some(_), None) => Observed::not("cpu_temp", "CPU", "unknown"),
        });
        self.lock().cpu = Some(cpu);
        out.push(if mem.total_kb > 0 {
            Observed::ok("memory", "RAM", mem.used_pct())
        } else {
            Observed::not("memory", "RAM", "unsupported")
        });
        match gpus {
            Some(g) => {
                let mut i = self.lock();
                out.extend(gpu_sensors(g.as_deref(), &mut i.gpu_seen));
            }
            // Not asked: keep what the last full reading said.
            None => {
                let i = self.lock();
                out.extend(
                    i.sensors
                        .iter()
                        .filter(|x| x.kind.starts_with("gpu_"))
                        .map(|x| Observed {
                            kind: match x.kind.as_str() {
                                "gpu_temp" => "gpu_temp",
                                "gpu_hotspot" => "gpu_hotspot",
                                _ => "gpu_vram",
                            },
                            subject: x.subject.clone(),
                            status: if x.status == "unsupported" {
                                "unsupported"
                            } else {
                                "unknown"
                            },
                            value: None,
                        }),
                );
            }
        }
        out.extend(disks);
        out
    }

    /// One round: read, evaluate, alert.
    async fn round(&self, conn: &zbus::Connection) {
        let s = self.settings();
        let seen = self.observe(gpu_on(&s)).await;
        let fires = {
            let mut i = self.lock();
            let (sensors, fires) = i.eval.update(crate::iface::now_ms(), &seen, &s);
            i.sensors = sensors;
            fires
        };
        for f in fires {
            let (summary, body) = text(&f, &s);
            self.deliver(
                conn,
                f.kind,
                &f.subject,
                f.value,
                f.threshold,
                &summary,
                &body,
            )
            .await;
        }
    }

    /// What the Alerts page shows. Reads the sensors now (GPU included)
    /// when the loop is parked or has not read the GPU.
    pub async fn state(&self) -> AlertsState {
        let s = self.settings();
        if !any_on(&s) || !gpu_on(&s) || self.lock().sensors.is_empty() {
            let seen = self.observe(true).await;
            // Not evaluated: only the support and the readings are wanted.
            let sensors: Vec<AlertSensor> = seen
                .iter()
                .map(|o| {
                    let above_s = self
                        .lock()
                        .sensors
                        .iter()
                        .find(|x| x.kind == o.kind && x.subject == o.subject)
                        .map_or(0, |x| x.above_s);
                    AlertSensor {
                        kind: o.kind.into(),
                        subject: o.subject.clone(),
                        status: o.status.into(),
                        value: o.value.unwrap_or(f64::NAN),
                        above_s: if rule(&s, o.kind).0 { above_s } else { 0 },
                    }
                })
                .collect();
            if !any_on(&s) {
                self.lock().sensors = sensors.clone();
            }
            return self.finish_state(s, sensors).await;
        }
        let sensors = self.lock().sensors.clone();
        self.finish_state(s, sensors).await
    }

    async fn finish_state(&self, s: AlertSettings, sensors: Vec<AlertSensor>) -> AlertsState {
        let root = self.root.clone();
        let mounts = tokio::task::spawn_blocking(move || {
            mounts(&root)
                .into_iter()
                .map(|(path, fs)| {
                    let u = disk_use(&root.join(path.trim_start_matches('/')));
                    AlertMount {
                        used_pct: u.map_or(f64::NAN, |u| u.0),
                        size_bytes: u.map_or(0, |u| u.1),
                        path,
                        fs,
                    }
                })
                .collect()
        })
        .await
        .unwrap_or_default();
        AlertsState {
            settings: s,
            sensors,
            mounts,
            recent: self.lock().recent.clone(),
            held: self.dnd.load(Ordering::Relaxed),
        }
    }

    /// Reads every two seconds while an alert is on; parked otherwise.
    pub async fn run(self: Arc<Self>, conn: zbus::Connection) {
        let mut changed = self.changed.subscribe();
        loop {
            if !any_on(&self.settings()) {
                self.lock().sensors.clear();
                if changed.changed().await.is_err() {
                    return;
                }
                continue;
            }
            let mut tick = tokio::time::interval(EVERY);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = tick.tick() => self.round(&conn).await,
                    r = changed.changed() => {
                        if r.is_err() {
                            return;
                        }
                        if !any_on(&self.settings()) {
                            break;
                        }
                    }
                }
            }
        }
    }
}

/// What a kind is called in a notification.
fn what(kind: &str, subject: &str) -> String {
    match kind {
        "cpu_temp" => "The CPU".into(),
        "gpu_temp" => format!("The graphics card ({subject})"),
        "gpu_hotspot" => format!("The graphics card's hotspot ({subject})"),
        "gpu_vram" => format!("The graphics card's memory ({subject})"),
        "memory" => "Memory".into(),
        "disk" => format!("The disk at {subject}"),
        _ => subject.into(),
    }
}

/// The notification for an alert.
pub fn text(f: &Fire, s: &AlertSettings) -> (String, String) {
    let unit = if f.kind == "memory" || f.kind == "disk" {
        " %"
    } else {
        " °C"
    };
    let summary = match f.kind {
        "memory" => format!("Memory is {:.0} % full", f.value),
        "disk" => format!("{} is {:.0} % full", f.subject, f.value),
        _ => format!("{} is at {:.0} °C", what(f.kind, &f.subject), f.value),
    };
    let body = format!(
        "{} has stayed above {}{unit} for {} s. The next alert about this comes after {} min at the earliest. \
         Change alerts in Unix Guardian, Monitoring, Alerts (or: ug alert).",
        what(f.kind, &f.subject),
        f.threshold,
        s.sustain_s,
        s.cooldown_min
    );
    (summary, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cpu(v: f64) -> Vec<Observed> {
        vec![Observed::ok("cpu_temp", "CPU", v)]
    }

    /// Feeds `v` every 2 s from `*t` for `secs` seconds; returns every alert.
    fn feed(e: &mut Evaluator, t: &mut u64, secs: u64, v: f64, s: &AlertSettings) -> Vec<Fire> {
        let mut all = Vec::new();
        for _ in 0..secs / 2 {
            all.extend(e.update(*t, &cpu(v), s).1);
            *t += 2000;
        }
        all
    }

    #[test]
    fn a_reading_must_stay_above_the_limit_for_the_whole_window() {
        let s = AlertSettings::default(); // CPU 90 °C, 10 s
        let mut e = Evaluator::default();
        let mut t = 1_000_000;
        // 8 s hot, one cool reading, 8 s hot: two short spikes, no alert.
        assert!(feed(&mut e, &mut t, 8, 95.0, &s).is_empty());
        assert!(feed(&mut e, &mut t, 2, 70.0, &s).is_empty());
        assert!(feed(&mut e, &mut t, 8, 95.0, &s).is_empty());
        let (sensors, _) = e.update(t, &cpu(95.0), &s);
        assert_eq!(sensors[0].above_s, 8, "the cells show the time so far");
        t += 2000;
        let (_, f) = e.update(t, &cpu(96.0), &s);
        assert_eq!(
            f,
            [Fire {
                kind: "cpu_temp",
                subject: "CPU".into(),
                value: 96.0,
                threshold: 90
            }],
            "10 s above"
        );
    }

    #[test]
    fn the_same_alert_waits_for_the_cooldown_and_others_do_not() {
        let s = AlertSettings {
            cooldown_min: 5,
            sustain_s: 3,
            ..Default::default()
        };
        let mut e = Evaluator::default();
        let mut t = 0;
        assert_eq!(feed(&mut e, &mut t, 6, 95.0, &s).len(), 1);
        // Still hot for 4 more minutes: nothing.
        assert!(feed(&mut e, &mut t, 240, 95.0, &s).is_empty());
        // Memory is another kind: it alerts in the meantime.
        let both = |v: f64| {
            vec![
                Observed::ok("cpu_temp", "CPU", 95.0),
                Observed::ok("memory", "RAM", v),
            ]
        };
        let s2 = AlertSettings {
            mem_on: true,
            ..s.clone()
        };
        let mut f = Vec::new();
        for _ in 0..3 {
            f.extend(e.update(t, &both(97.0), &s2).1);
            t += 2000;
        }
        assert_eq!(f.iter().map(|f| f.kind).collect::<Vec<_>>(), ["memory"]);
        // Past 5 minutes since the CPU alert: it may repeat.
        assert_eq!(feed(&mut e, &mut t, 60, 95.0, &s).len(), 1);
    }

    #[test]
    fn the_cooldown_survives_a_restart_through_the_recent_list() {
        let s = AlertSettings::default();
        let recent = [AlertEvent {
            ts_unix_ms: 1_000_000,
            kind: "cpu_temp".into(),
            subject: "CPU".into(),
            ..Default::default()
        }];
        let mut e = Evaluator::new(&recent);
        let mut t = 1_000_000 + 60_000;
        assert!(feed(&mut e, &mut t, 30, 99.0, &s).is_empty(), "29 min left");
        // A test alert does not start a cooldown.
        let recent = [AlertEvent {
            ts_unix_ms: 1_000_000,
            kind: "test".into(),
            ..Default::default()
        }];
        let mut e = Evaluator::new(&recent);
        let mut t = 1_000_000 + 60_000;
        assert_eq!(feed(&mut e, &mut t, 12, 99.0, &s).len(), 1);
    }

    #[test]
    fn a_switched_off_unsupported_or_sleeping_sensor_never_alerts() {
        let s = AlertSettings {
            sustain_s: 3,
            ..Default::default()
        };
        let mut e = Evaluator::default();
        let seen = vec![
            Observed::ok("memory", "RAM", 99.0), // memory alert is off by default
            Observed::not("gpu_temp", "RTX 3050", "asleep"),
            Observed::not("cpu_temp", "CPU", "unsupported"),
        ];
        for i in 0..10 {
            let (sensors, f) = e.update(i * 2000, &seen, &s);
            assert!(f.is_empty());
            assert!(sensors.iter().all(|x| x.above_s == 0));
            assert_eq!(sensors[1].status, "asleep");
            assert!(sensors[1].reading().is_none());
        }
    }

    #[test]
    fn a_long_gap_between_readings_starts_the_window_again() {
        let s = AlertSettings::default();
        let mut e = Evaluator::default();
        let mut t = 0;
        assert!(feed(&mut e, &mut t, 8, 95.0, &s).is_empty());
        t += 60_000; // suspended for a minute
        assert!(feed(&mut e, &mut t, 8, 95.0, &s).is_empty());
        assert_eq!(feed(&mut e, &mut t, 4, 95.0, &s).len(), 1);
    }

    #[test]
    fn a_sleeping_gpu_is_not_read_and_sub_sensors_appear_only_once_seen() {
        let mut seen = HashSet::new();
        let asleep = GpuRead {
            pci: "0000:01:00.0".into(),
            name: "RTX 3050".into(),
            asleep: true,
            core: None,
            hotspot: None,
            vram: None,
        };
        let o = gpu_sensors(Some(std::slice::from_ref(&asleep)), &mut seen);
        let st: Vec<&str> = o.iter().map(|o| o.status).collect();
        assert_eq!(st, ["asleep", "unknown", "unknown"]);
        // Awake, an NVIDIA laptop card: core only.
        let awake = GpuRead {
            asleep: false,
            core: Some(61.0),
            ..asleep
        };
        let o = gpu_sensors(Some(std::slice::from_ref(&awake)), &mut seen);
        let st: Vec<&str> = o.iter().map(|o| o.status).collect();
        assert_eq!(st, ["ok", "unsupported", "unsupported"]);
        // An AMD card with a junction sensor keeps offering it asleep.
        let amd = GpuRead {
            pci: "0000:03:00.0".into(),
            name: "RX 7600".into(),
            asleep: false,
            core: Some(60.0),
            hotspot: Some(75.0),
            vram: None,
        };
        gpu_sensors(Some(std::slice::from_ref(&amd)), &mut seen);
        let amd = GpuRead {
            asleep: true,
            core: None,
            hotspot: None,
            ..amd
        };
        let o = gpu_sensors(Some(std::slice::from_ref(&amd)), &mut seen);
        let st: Vec<&str> = o.iter().map(|o| o.status).collect();
        assert_eq!(st, ["asleep", "asleep", "unknown"]);
        // No GPU at all; the service not reachable.
        assert!(gpu_sensors(Some(&[]), &mut seen)
            .iter()
            .all(|o| o.status == "unsupported"));
        assert_eq!(gpu_sensors(None, &mut seen)[0].status, "unknown");
    }

    #[test]
    fn the_mount_picker_lists_real_disks_once_each() {
        let fs = ug_testfs::TestFs::new();
        fs.write(
            "proc/mounts",
            "proc /proc proc rw 0 0\n\
             /dev/nvme0n1p2 / btrfs rw,subvol=/@ 0 0\n\
             /dev/nvme0n1p2 /home btrfs rw,subvol=/@home 0 0\n\
             /dev/nvme0n1p1 /boot vfat rw 0 0\n\
             tmpfs /tmp tmpfs rw 0 0\n\
             /dev/sda1 /mnt/Games\\040Disk ext4 rw 0 0\n\
             /dev/sdb1 /run/media/u/USB exfat rw 0 0\n",
        );
        let m = mounts(&SysRoot::at(fs.root()));
        let paths: Vec<&str> = m.iter().map(|m| m.0.as_str()).collect();
        assert_eq!(paths, ["/", "/boot", "/mnt/Games Disk"]);
        let chosen = vec!["/".to_string(), "/home".into(), "/media/gone".into()];
        let o = disk_sensors(&chosen, &m, |p| (p != "/media/gone").then_some((96.5, 1)));
        let st: Vec<(&str, &str)> = o.iter().map(|o| (o.subject.as_str(), o.status)).collect();
        assert_eq!(
            st,
            [("/", "ok"), ("/home", "ok"), ("/media/gone", "missing")]
        );
    }

    #[test]
    fn settings_are_checked_saved_and_read_back() {
        let d = tempfile::tempdir().unwrap();
        let agent = Arc::new(Settings::load(d.path(), None));
        let a = Alerts::new(
            d.path(),
            d.path(),
            SysRoot::at(d.path()),
            true,
            agent.clone(),
            false,
        );
        let s = a.settings();
        assert!(s.cpu_temp_on && s.gpu_temp_on && s.disk_on && !s.mem_on);
        assert_eq!((s.sustain_s, s.cooldown_min), (10, 30));
        let bad = |f: fn(&mut AlertSettings)| {
            let mut s = AlertSettings::default();
            f(&mut s);
            validate(&s).unwrap_err()
        };
        assert!(bad(|s| s.sustain_s = 2).contains("3 to 30"));
        assert!(bad(|s| s.cooldown_min = 10).contains("cooldown"));
        assert!(bad(|s| s.disk_mounts = vec!["home".into()]).contains("mount point"));
        let mine = AlertSettings {
            mem_on: true,
            mem_pct: 80,
            ..Default::default()
        };
        a.set(mine.clone()).unwrap();
        let again = Alerts::new(
            d.path(),
            d.path(),
            SysRoot::at(d.path()),
            true,
            agent,
            false,
        );
        assert_eq!(again.settings(), mine);
    }

    #[test]
    fn recent_alerts_keep_the_newest_fifty_and_clear() {
        let d = tempfile::tempdir().unwrap();
        let agent = Arc::new(Settings::load(d.path(), None));
        let a = Alerts::new(
            d.path(),
            d.path(),
            SysRoot::at(d.path()),
            true,
            agent.clone(),
            false,
        );
        for i in 0..60 {
            a.record(AlertEvent {
                ts_unix_ms: i,
                kind: "disk".into(),
                ..Default::default()
            });
        }
        let again = Alerts::new(
            d.path(),
            d.path(),
            SysRoot::at(d.path()),
            true,
            agent,
            false,
        );
        let r = again.lock().recent.clone();
        assert_eq!((r.len(), r[0].ts_unix_ms, r[49].ts_unix_ms), (50, 59, 10));
        assert_eq!(again.clear(), 50);
        assert!(again.lock().recent.is_empty());
        assert!(!d.path().join("alerts-recent.json").exists());
    }

    #[test]
    fn do_not_disturb_and_the_notification_switch_hold_alerts() {
        let d = tempfile::tempdir().unwrap();
        let agent = Arc::new(Settings::load(d.path(), None));
        let a = Alerts::new(
            d.path(),
            d.path(),
            SysRoot::at(d.path()),
            true,
            agent.clone(),
            false,
        );
        assert_eq!(a.held_reason(), "");
        a.dnd.store(true, Ordering::Relaxed);
        assert_eq!(a.held_reason(), "dnd");
        agent
            .set(ug_proto::AgentSettings {
                notifications: false,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(a.held_reason(), "off");
    }

    #[test]
    fn the_notification_names_the_thing_the_value_and_the_limit() {
        let (sum, body) = text(
            &Fire {
                kind: "disk",
                subject: "/home".into(),
                value: 96.4,
                threshold: 95,
            },
            &AlertSettings::default(),
        );
        assert_eq!(sum, "/home is 96 % full");
        assert!(body.starts_with("The disk at /home has stayed above 95 % for 10 s."));
    }
}
