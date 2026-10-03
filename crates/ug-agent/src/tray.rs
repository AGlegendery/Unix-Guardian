//! The tray icon: a StatusNotifierItem with a dbusmenu, written directly on
//! the agent's session connection.
//!
//! It lives here rather than in the window because the window closes
//! completely (and during games), while the agent runs anyway. Libraries that
//! draw tray icons pull in GTK; the two D-Bus interfaces below are all KDE,
//! GNOME's AppIndicator extension and most panels need, and cost nothing
//! while the menu is closed: usage is only read while it is open.
//!
//! Left click opens the window. The menu has the usage preview, "Open
//! terminal UI", "Quit tray icon" and "Quit Unix Guardian".
//!
//! "Quit tray icon" only removes the icon, for people it bothers; everything
//! else keeps running, and the choice is remembered (`tray-hidden` in the
//! config directory) so the icon does not come back at the next login until
//! `ug tray` or the window's "Show tray icon" brings it back.
//!
//! "Quit Unix Guardian" closes the windows and terminal UIs and stops the
//! agent: game detection, game mode, playtime and the overlay's FPS log stop
//! with it (the daemon reverts any game-mode change when the agent goes).
//! The root daemon keeps running, parked and idle, because stopping a system
//! service needs an administrator. The agent comes back at the next login,
//! or as soon as the window or `ug` asks for it (D-Bus activation).

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use crate::health::{Health, IconState, Reading, Temp, Thresholds};
use crate::iface::{lock, Shared};
use ug_hw::SysRoot;
use ug_proto::{GpuInfo, GpuSample, MemSample};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, StructureBuilder, Value};

const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/MenuBar";
const WATCHERS: [&str; 2] = ["org.kde.StatusNotifierWatcher", "org.freedesktop.StatusNotifierWatcher"];

/// How often usage is read: for the icon's colour, and for the preview.
const EVERY: Duration = Duration::from_secs(2);
/// GPU rows the menu has room for; more GPUs than this are not shown.
const MAX_GPUS: usize = 4;

// Menu item ids. They never change, so the layout never has to be re-sent:
// the preview rows are updated in place and hidden when they have nothing.
const ID_ROOT: i32 = 0;
const ID_OPEN: i32 = 1;
const ID_SEP1: i32 = 2;
const ID_SEP2: i32 = 3;
const ID_TERMINAL: i32 = 4;
const ID_SEP3: i32 = 5;
const ID_QUIT: i32 = 6;
const ID_HIDE: i32 = 7;
const ID_CPU: i32 = 10;
const ID_RAM: i32 = 11;
const ID_GPU0: i32 = 12;
const ID_BATTERY: i32 = ID_GPU0 + MAX_GPUS as i32;

const PREVIEW_IDS: [i32; 3 + MAX_GPUS] = [ID_CPU, ID_RAM, ID_GPU0, ID_GPU0 + 1, ID_GPU0 + 2, ID_GPU0 + 3, ID_BATTERY];
const MENU_ORDER: [i32; 7 + 3 + MAX_GPUS] = [
    ID_OPEN, ID_SEP1, ID_CPU, ID_RAM, ID_GPU0, ID_GPU0 + 1, ID_GPU0 + 2, ID_GPU0 + 3, ID_BATTERY, ID_SEP2, ID_TERMINAL,
    ID_SEP3, ID_HIDE, ID_QUIT,
];

/// One read of everything the preview shows. NaN where unknown.
#[derive(Debug, Clone, Default)]
pub struct Usage {
    pub cpu_pct: f64,
    pub cpu_temp_c: f64,
    pub mem: MemSample,
    /// `None` when the daemon is not reachable (GPU readings come from it).
    pub gpus: Option<Vec<(String, GpuSample)>>,
    /// (percent, watts draining or negative when charging, status, on AC);
    /// `None` without a battery.
    pub battery: Option<(f64, f64, String, bool)>,
}

/// The preview rows, by id. A row with no text is hidden.
pub fn preview_rows(u: &Usage) -> Vec<(i32, String)> {
    let mut rows = Vec::new();
    let mut cpu = format!("CPU  {:.0} %", u.cpu_pct);
    if !u.cpu_temp_c.is_nan() {
        cpu += &format!("  ·  {:.0} °C", u.cpu_temp_c);
    }
    rows.push((ID_CPU, cpu));
    let gb = |kb: u64| kb as f64 / 1024.0 / 1024.0;
    rows.push((
        ID_RAM,
        if u.mem.total_kb == 0 {
            String::new()
        } else {
            format!("RAM  {:.1} / {:.1} GB  ({:.0} %)", gb(u.mem.used_kb()), gb(u.mem.total_kb), u.mem.used_pct())
        },
    ));
    for i in 0..MAX_GPUS {
        let text = match &u.gpus {
            None if i == 0 => "GPU  —  the Unix Guardian service is not running".to_string(),
            None => String::new(),
            Some(g) => g.get(i).map(|(name, s)| gpu_row(name, s)).unwrap_or_default(),
        };
        rows.push((ID_GPU0 + i as i32, text));
    }
    rows.push((
        ID_BATTERY,
        match &u.battery {
            None => String::new(),
            Some((pct, w, status, on_ac)) => {
                let mut s = format!("Battery  {pct:.0} %");
                match status.as_str() {
                    "Charging" if !w.is_nan() => s += &format!("  ·  charging {:.1} W", w.abs()),
                    "Charging" => s += "  ·  charging",
                    "Discharging" if !w.is_nan() => s += &format!("  ·  {w:.1} W"),
                    _ if *on_ac => s += "  ·  plugged in",
                    _ => {}
                }
                s
            }
        },
    ));
    rows
}

fn gpu_row(name: &str, s: &GpuSample) -> String {
    let name = short_gpu_name(name);
    if s.asleep {
        return format!("{name}  ·  asleep");
    }
    let mut parts = Vec::new();
    if !s.load_pct.is_nan() {
        parts.push(format!("{:.0} %", s.load_pct));
    }
    if !s.temp_c.is_nan() {
        parts.push(format!("{:.0} °C", s.temp_c));
    }
    if !s.power_w.is_nan() {
        parts.push(format!("{:.0} W", s.power_w));
    }
    if parts.is_empty() {
        // Integrated GPUs often report nothing; a bare name reads as a heading.
        format!("{name}  ·  no readings")
    } else {
        format!("{name}  ·  {}", parts.join("  ·  "))
    }
}

/// "NVIDIA GeForce RTX 3050 Laptop GPU" → "RTX 3050".
pub fn short_gpu_name(name: &str) -> String {
    let mut n = name.replace("(R)", "").replace("(TM)", "");
    for prefix in ["NVIDIA GeForce ", "NVIDIA ", "AMD Radeon ", "AMD ", "Intel "] {
        if let Some(rest) = n.strip_prefix(prefix) {
            n = rest.to_string();
            break;
        }
    }
    for suffix in [" Laptop GPU", " Graphics"] {
        if let Some(rest) = n.strip_suffix(suffix) {
            n = rest.to_string();
        }
    }
    let n = n.split_whitespace().collect::<Vec<_>>().join(" ");
    if n.is_empty() {
        "GPU".into()
    } else {
        n
    }
}

/// dbusmenu reads `_` as a mnemonic marker.
fn label_text(s: &str) -> String {
    s.replace('_', "__")
}

fn ov<'a>(v: impl Into<Value<'a>>) -> OwnedValue {
    OwnedValue::try_from(v.into()).expect("plain values carry no file descriptors")
}

struct State {
    visible: bool,
    /// Preview rows by id; empty text means hidden.
    rows: HashMap<i32, String>,
    open: bool,
    load: ug_hw::cpu::LoadReader,
    gpu_names: Vec<GpuInfo>,
    health: Health,
    icon: IconState,
    /// The CPU package sensor and its critical point, found once.
    cpu_sensor: Option<(PathBuf, Option<f64>)>,
    throttle_count: Option<u64>,
    swap_in: Option<(u64, Instant)>,
    /// Kept between reads; `None` after a failure.
    daemon: Option<zbus::Connection>,
}

/// The tray, shared by its D-Bus interfaces and the agent's `Tray` interface.
#[derive(Clone)]
pub struct Tray {
    conn: zbus::Connection,
    name: String,
    daemon_session: bool,
    state: Arc<Mutex<State>>,
    core: Shared,
    /// The status icons are installed in the icon theme, so they can be
    /// loaded by name and follow the panel's colours.
    themed: bool,
    /// Present while the user has chosen "Quit tray icon".
    hidden_flag: PathBuf,
}

impl Tray {
    fn st(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Serves the icon and menu on `conn`, shows the icon, and keeps it
    /// registered across panel restarts.
    pub async fn start(conn: &zbus::Connection, daemon_session: bool, core: Shared) -> zbus::Result<Tray> {
        let tray = Tray {
            conn: conn.clone(),
            name: format!("org.kde.StatusNotifierItem-{}-1", std::process::id()),
            daemon_session,
            state: Arc::new(Mutex::new(State {
                visible: false,
                rows: HashMap::new(),
                open: false,
                load: ug_hw::cpu::LoadReader::new(),
                gpu_names: Vec::new(),
                health: Health::new(Thresholds::load(&ug_core::paths::user_config().join("tray.json"))),
                icon: IconState::Normal,
                cpu_sensor: cpu_sensor(&SysRoot::real()),
                throttle_count: None,
                swap_in: None,
                daemon: None,
            })),
            core,
            themed: icons_installed(),
            hidden_flag: ug_core::paths::user_config().join("tray-hidden"),
        };
        conn.object_server().at(ITEM_PATH, ItemIface { tray: tray.clone() }).await?;
        conn.object_server().at(MENU_PATH, MenuIface { tray: tray.clone() }).await?;
        tokio::spawn(tray.clone().follow_watcher());
        tokio::spawn(tray.clone().watch());
        if tray.hidden_flag.exists() {
            tracing::info!("the tray icon was hidden by the user; `ug tray` shows it");
        } else {
            tray.show().await;
        }
        Ok(tray)
    }

    pub fn visible(&self) -> bool {
        self.st().visible
    }

    /// Puts the icon in the panel (again), and forgets an earlier "Quit
    /// tray icon".
    pub async fn show(&self) {
        if let Err(e) = std::fs::remove_file(&self.hidden_flag) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(error = %e, "could not forget that the tray icon was hidden");
            }
        }
        if self.visible() {
            return;
        }
        match self.conn.request_name_with_flags(self.name.as_str(), zbus::fdo::RequestNameFlags::DoNotQueue.into()).await {
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(error = %e, "could not claim the tray icon's bus name");
                return;
            }
        }
        self.st().visible = true;
        self.register().await;
    }

    /// Takes the icon out of the panel, and keeps it out across logins.
    /// The watcher notices the name going.
    pub async fn hide(&self) {
        if let Err(e) = ug_core::paths::write_atomic(&self.hidden_flag, b"The tray icon stays hidden until `ug tray` shows it.\n") {
            tracing::warn!(error = %e, "could not remember that the tray icon is hidden");
        }
        if !self.visible() {
            return;
        }
        self.st().visible = false;
        let _ = self.conn.release_name(self.name.as_str()).await;
        tracing::info!("tray icon hidden; `ug tray` shows it again");
    }

    async fn register(&self) {
        for w in WATCHERS {
            let r = self
                .conn
                .call_method(Some(w), "/StatusNotifierWatcher", Some(w), "RegisterStatusNotifierItem", &(self.name.as_str(),))
                .await;
            if r.is_ok() {
                tracing::info!(watcher = w, "tray icon shown");
                return;
            }
        }
        // Nothing draws tray icons in this session yet (GNOME without the
        // AppIndicator extension, or the panel is still starting): follow_watcher
        // registers as soon as one appears.
        tracing::debug!("no tray host in this session yet");
    }

    /// Registers again whenever a panel (re)starts.
    async fn follow_watcher(self) {
        use futures_lite::StreamExt;
        let Ok(dbus) = zbus::fdo::DBusProxy::new(&self.conn).await else { return };
        let Ok(mut changes) = dbus.receive_name_owner_changed().await else { return };
        while let Some(sig) = changes.next().await {
            let Ok(args) = sig.args() else { continue };
            if WATCHERS.contains(&args.name.as_str()) && args.new_owner.is_some() && self.visible() {
                self.register().await;
            }
        }
    }

    /// Reads usage every two seconds: the icon changes colour with it, and
    /// an open menu shows it.
    async fn watch(self) {
        let mut tick = tokio::time::interval(EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let changed = self.refresh().await;
            if self.st().open {
                self.emit_rows(&changed).await;
            }
        }
    }

    /// Reads usage, updates the rows and the icon. Returns the ids of the
    /// rows whose text changed.
    async fn refresh(&self) -> Vec<i32> {
        let gpus = self.read_gpus().await;
        let gaming = {
            let core = lock(&self.core);
            core.gamemode_desired().is_some() || !core.running().is_empty()
        };
        let state = Arc::clone(&self.state);
        let local = tokio::task::spawn_blocking(move || {
            let root = SysRoot::real();
            let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
            let cpu_pct = st.load.read(&root).total_pct;
            let (cpu_temp_c, cpu_crit_c) = match &st.cpu_sensor {
                Some((path, crit)) => (read_milli(path).unwrap_or(f64::NAN), *crit),
                None => (f64::NAN, None),
            };
            let count = read_u64(&root.join("sys/devices/system/cpu/cpu0/thermal_throttle/package_throttle_count"));
            let throttling = matches!((st.throttle_count, count), (Some(a), Some(b)) if b > a);
            st.throttle_count = count;
            let now = Instant::now();
            let swap_in = std::fs::read_to_string(root.join("proc/vmstat"))
                .ok()
                .and_then(|v| v.lines().find_map(|l| l.strip_prefix("pswpin ")?.trim().parse::<u64>().ok()));
            let swap_rate = match (st.swap_in, swap_in) {
                (Some((a, t)), Some(b)) => b.saturating_sub(a) as f64 / now.duration_since(t).as_secs_f64().max(0.1),
                _ => 0.0,
            };
            st.swap_in = swap_in.map(|v| (v, now));
            let (pct, w, status, on_ac) = ug_hw::extra::battery(&root);
            let battery = (!pct.is_nan()).then_some((pct, w, status, on_ac));
            let mem = ug_hw::mem::read(&root);
            (cpu_pct, cpu_temp_c, cpu_crit_c, throttling, swap_rate, mem, battery)
        })
        .await;
        let Ok((cpu_pct, cpu_temp_c, cpu_crit_c, throttling, swap_rate, mem, battery)) = local else { return Vec::new() };

        let mut reading = Reading {
            ram_pct: mem.used_pct(),
            throttling,
            // 1000 pages/s read back from swap is about 4 MB/s.
            thrashing: swap_rate >= 1000.0 && mem.used_pct() >= 90.0,
            gaming,
            daemon_ok: gpus.is_some(),
            ..Default::default()
        };
        if !cpu_temp_c.is_nan() {
            reading.temps.push(Temp { what: "CPU".into(), c: cpu_temp_c, crit_c: cpu_crit_c });
        }
        if !cpu_pct.is_nan() {
            reading.usage.push(("CPU".into(), cpu_pct));
        }
        let names = self.st().gpu_names.clone();
        for (name, g) in gpus.iter().flatten() {
            if g.asleep {
                continue;
            }
            let what = short_gpu_name(name);
            if !g.temp_c.is_nan() {
                reading.temps.push(Temp { what: what.clone(), c: g.temp_c, crit_c: None });
            }
            if !g.load_pct.is_nan() {
                reading.usage.push((what.clone(), g.load_pct));
            }
            let total = names.iter().find(|i| i.pci == g.pci).map(|i| i.vram_total_mb).unwrap_or(0);
            if total > 0 && g.vram_used_mb > 0 {
                reading.vram.push((what, g.vram_used_mb as f64 * 100.0 / total as f64));
            }
        }

        let usage = Usage { cpu_pct, cpu_temp_c, mem, gpus, battery };
        let (icon_changed, changed) = {
            let mut st = self.st();
            let before = (st.icon, st.health.reasons().to_vec());
            st.icon = st.health.update(Instant::now(), &reading);
            let icon_changed = before != (st.icon, st.health.reasons().to_vec());
            let mut changed = Vec::new();
            for (id, text) in preview_rows(&usage) {
                if st.rows.get(&id) != Some(&text) {
                    st.rows.insert(id, text);
                    changed.push(id);
                }
            }
            (icon_changed, changed)
        };
        if icon_changed {
            {
                let st = self.st();
                tracing::info!(state = st.icon.name(), reasons = ?st.health.reasons(), "tray icon");
            }
            if let Ok(iface) = self.conn.object_server().interface::<_, ItemIface>(ITEM_PATH).await {
                let e = iface.signal_emitter();
                let _ = ItemIface::new_icon(e).await;
                let _ = ItemIface::new_tool_tip(e).await;
            }
        }
        changed
    }

    /// GPU readings from the daemon (it owns NVML and never wakes a sleeping
    /// GPU). `None` when it cannot be reached within a second.
    async fn read_gpus(&self) -> Option<Vec<(String, GpuSample)>> {
        let read = async {
            let cached = self.st().daemon.clone();
            let conn = match cached {
                Some(c) => c,
                None => {
                    let c = if self.daemon_session { zbus::Connection::session().await } else { zbus::Connection::system().await }
                        .ok()?;
                    self.st().daemon = Some(c.clone());
                    c
                }
            };
            let need_names = self.st().gpu_names.is_empty();
            if need_names {
                let list = ug_proto::daemon::GpuProxy::new(&conn).await.ok()?.list().await.ok()?;
                self.st().gpu_names = list;
            }
            let snap = ug_proto::daemon::DaemonProxy::new(&conn).await.ok()?.snapshot().await.ok()?;
            let names = self.st().gpu_names.clone();
            Some(
                snap.gpus
                    .into_iter()
                    .map(|s| {
                        let name = names.iter().find(|g| g.pci == s.pci).map(|g| g.name.clone()).unwrap_or_default();
                        (name, s)
                    })
                    .collect(),
            )
        };
        let r = tokio::time::timeout(Duration::from_secs(1), read).await.ok().flatten();
        if r.is_none() {
            // A restarted daemon needs a fresh connection; the names may differ too.
            let mut st = self.st();
            st.daemon = None;
            st.gpu_names.clear();
        }
        r
    }

    fn tool_tip_text(&self) -> String {
        let st = self.st();
        match st.icon {
            IconState::Offline => "The Unix Guardian service is not running".into(),
            IconState::Warning | IconState::Critical => st.health.reasons().join(" · "),
            IconState::GameMode => "Game mode is on · right-click for usage".into(),
            IconState::Normal => "Click to open · right-click for usage".into(),
        }
    }

    async fn emit_rows(&self, ids: &[i32]) {
        if ids.is_empty() {
            return;
        }
        let updated: Vec<(i32, HashMap<String, OwnedValue>)> = ids.iter().map(|&id| (id, self.props(id))).collect();
        if let Ok(iface) = self.conn.object_server().interface::<_, MenuIface>(MENU_PATH).await {
            let _ = MenuIface::items_properties_updated(iface.signal_emitter(), updated, Vec::new()).await;
        }
    }

    fn props(&self, id: i32) -> HashMap<String, OwnedValue> {
        let mut p = HashMap::new();
        let label = |p: &mut HashMap<String, OwnedValue>, s: &str| {
            p.insert("label".to_string(), ov(label_text(s)));
        };
        match id {
            ID_ROOT => {
                p.insert("children-display".into(), ov("submenu"));
            }
            ID_OPEN => {
                label(&mut p, "Open Unix Guardian");
                p.insert("icon-name".into(), ov("window-new"));
            }
            ID_TERMINAL => {
                label(&mut p, "Open terminal UI");
                p.insert("icon-name".into(), ov("utilities-terminal"));
            }
            ID_HIDE => {
                label(&mut p, "Quit tray icon");
                p.insert("icon-name".into(), ov("view-hidden"));
            }
            ID_QUIT => {
                label(&mut p, "Quit Unix Guardian");
                p.insert("icon-name".into(), ov("application-exit"));
            }
            ID_SEP1 | ID_SEP2 | ID_SEP3 => {
                p.insert("type".into(), ov("separator"));
            }
            _ => {
                let text = self.st().rows.get(&id).cloned().unwrap_or_default();
                // Read-only rows: shown, but not something to click.
                p.insert("enabled".into(), ov(false));
                p.insert("visible".into(), ov(!text.is_empty()));
                label(&mut p, &text);
            }
        }
        p
    }

    fn layout_of(&self, id: i32, with_children: bool) -> zbus::fdo::Result<Layout> {
        let mut children = Vec::new();
        if with_children && id == ID_ROOT {
            for &c in &MENU_ORDER {
                // Children travel as variants holding the same structure.
                let child = StructureBuilder::new()
                    .add_field(c)
                    .add_field(self.props(c))
                    .add_field(Vec::<OwnedValue>::new())
                    .build()
                    .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
                children.push(ov(child));
            }
        }
        Ok(Layout(id, self.props(id), children))
    }

    fn clicked(&self, id: i32) {
        let tray = self.clone();
        tokio::spawn(async move {
            let result = match id {
                ID_OPEN => open_window().await,
                ID_TERMINAL => open_terminal().await,
                ID_HIDE => {
                    tray.hide().await;
                    Ok(())
                }
                ID_QUIT => {
                    quit_everything();
                    Ok(())
                }
                _ => Ok(()),
            };
            if let Err(e) = result {
                tracing::warn!(error = %e, "tray action failed");
                crate::notify::show(&tray.conn, "Unix Guardian", &e, &[]).await;
            }
        });
    }
}

/// `org.kde.StatusNotifierItem`
struct ItemIface {
    tray: Tray,
}

type Pixmap = (i32, i32, Vec<u8>);

macro_rules! state_pixmaps {
    ($state:literal) => {
        [
            (22, &include_bytes!(concat!("../assets/tray-", $state, "-22.argb"))[..]),
            (32, &include_bytes!(concat!("../assets/tray-", $state, "-32.argb"))[..]),
            (48, &include_bytes!(concat!("../assets/tray-", $state, "-48.argb"))[..]),
            (64, &include_bytes!(concat!("../assets/tray-", $state, "-64.argb"))[..]),
        ]
    };
}

/// The G gauge in each state, for hosts that cannot load the themed icons
/// (made by scripts/gen-tray-icons.py).
fn pixmaps(state: IconState) -> Vec<Pixmap> {
    let sizes = match state {
        IconState::Normal => state_pixmaps!("normal"),
        IconState::Warning => state_pixmaps!("warning"),
        IconState::Critical => state_pixmaps!("critical"),
        IconState::GameMode => state_pixmaps!("gamemode"),
        IconState::Offline => state_pixmaps!("offline"),
    };
    sizes.into_iter().map(|(s, b)| (s, s, b.to_vec())).collect()
}

/// `unix-guardian-normal` and the rest are in a hicolor theme directory.
fn icons_installed() -> bool {
    let home = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share"));
    let dirs = std::env::var_os("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    home.into_iter()
        .chain(std::env::split_paths(&dirs))
        .any(|d| d.join("icons/hicolor/scalable/status/unix-guardian-normal.svg").is_file())
}

fn read_u64(p: &Path) -> Option<u64> {
    std::fs::read_to_string(p).ok()?.trim().parse().ok()
}

fn read_milli(p: &Path) -> Option<f64> {
    read_u64(p).map(|v| v as f64 / 1000.0)
}

/// The CPU package temperature sensor and its `_crit` value (TjMax on Intel).
fn cpu_sensor(root: &SysRoot) -> Option<(PathBuf, Option<f64>)> {
    let chips = ug_hw::hwmon::enumerate(root);
    let chip = ug_hw::hwmon::cpu_chip(&chips)?;
    let sensor = ["Package id 0", "Tctl", "Tdie", "Composite"]
        .iter()
        .find_map(|l| chip.temp_by_label(l))
        .or_else(|| chip.temps.iter().min_by_key(|s| s.index))?;
    let crit = sensor.input_path.to_str().map(|p| PathBuf::from(p.replace("_input", "_crit"))).and_then(|p| read_milli(&p));
    Some((sensor.input_path.clone(), crit.filter(|c| *c > 0.0)))
}

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl ItemIface {
    #[zbus(property)]
    fn category(&self) -> &str {
        "SystemServices"
    }
    #[zbus(property)]
    fn id(&self) -> &str {
        "unix-guardian"
    }
    #[zbus(property)]
    fn title(&self) -> &str {
        "Unix Guardian"
    }
    #[zbus(property)]
    fn status(&self) -> &str {
        "Active"
    }
    #[zbus(property)]
    fn window_id(&self) -> i32 {
        0
    }
    /// The themed icon when installed (its plain parts follow the panel's
    /// text colour); empty otherwise, so hosts use the pixmap.
    #[zbus(property)]
    fn icon_name(&self) -> String {
        if self.tray.themed {
            format!("unix-guardian-{}", self.tray.st().icon.name())
        } else {
            String::new()
        }
    }
    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<Pixmap> {
        pixmaps(self.tray.st().icon)
    }
    #[zbus(property)]
    fn overlay_icon_name(&self) -> &str {
        ""
    }
    #[zbus(property)]
    fn attention_icon_name(&self) -> &str {
        ""
    }
    #[zbus(property)]
    fn tool_tip(&self) -> (String, Vec<Pixmap>, String, String) {
        (String::new(), Vec::new(), "Unix Guardian".into(), self.tray.tool_tip_text())
    }
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn menu(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from(MENU_PATH).expect("valid path")
    }

    /// Left click.
    fn activate(&self, _x: i32, _y: i32) {
        self.tray.clicked(ID_OPEN);
    }
    fn secondary_activate(&self, _x: i32, _y: i32) {}
    fn context_menu(&self, _x: i32, _y: i32) {}
    fn scroll(&self, _delta: i32, _orientation: &str) {}

    #[zbus(signal)]
    async fn new_icon(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn new_tool_tip(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// One menu item and its children, `(ia{sv}av)`.
#[derive(Debug, serde::Serialize, zbus::zvariant::Type)]
struct Layout(i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

/// `com.canonical.dbusmenu`
struct MenuIface {
    tray: Tray,
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl MenuIface {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }
    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }
    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }
    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }

    /// The layout is fixed, so its revision is always 1.
    ///
    /// The reply must be `u(ia{sv}av)` or KDE shows no menu. The return type
    /// is a literal tuple on purpose: zbus sends its fields as two arguments,
    /// but behind a type alias it would send one structure.
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> zbus::fdo::Result<(u32, Layout)> {
        if !(parent_id == ID_ROOT || MENU_ORDER.contains(&parent_id)) {
            return Err(zbus::fdo::Error::InvalidArgs(format!("no menu item {parent_id}")));
        }
        Ok((1, self.tray.layout_of(parent_id, recursion_depth != 0)?))
    }

    fn get_group_properties(&self, ids: Vec<i32>, _property_names: Vec<String>) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        let ids = if ids.is_empty() { std::iter::once(ID_ROOT).chain(MENU_ORDER).collect() } else { ids };
        ids.into_iter().map(|id| (id, self.tray.props(id))).collect()
    }

    fn get_property(&self, id: i32, name: &str) -> zbus::fdo::Result<OwnedValue> {
        self.tray
            .props(id)
            .remove(name)
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("item {id} has no property {name}")))
    }

    fn event(&self, id: i32, event_id: &str, _data: OwnedValue, _timestamp: u32) {
        self.on_event(id, event_id);
    }

    fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> Vec<i32> {
        for (id, event_id, _, _) in &events {
            self.on_event(*id, event_id);
        }
        Vec::new()
    }

    /// Called just before the menu shows: the preview is read now, so it is
    /// current when the menu appears.
    async fn about_to_show(&self, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>, id: i32) -> bool {
        if id != ID_ROOT {
            return false;
        }
        // The rows are at most a couple of seconds old; send them all so a
        // host that cached the menu shows the current ones.
        let updated = PREVIEW_IDS.iter().map(|&i| (i, self.tray.props(i))).collect();
        let _ = Self::items_properties_updated(&emitter, updated, Vec::new()).await;
        false
    }

    async fn about_to_show_group(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        ids: Vec<i32>,
    ) -> (Vec<i32>, Vec<i32>) {
        if ids.contains(&ID_ROOT) {
            self.about_to_show(emitter, ID_ROOT).await;
        }
        (Vec::new(), Vec::new())
    }

    #[zbus(signal)]
    async fn items_properties_updated(
        emitter: &SignalEmitter<'_>,
        updated: Vec<(i32, HashMap<String, OwnedValue>)>,
        removed: Vec<(i32, Vec<String>)>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn layout_updated(emitter: &SignalEmitter<'_>, revision: u32, parent: i32) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn item_activation_requested(emitter: &SignalEmitter<'_>, id: i32, timestamp: u32) -> zbus::Result<()>;
}

impl MenuIface {
    fn on_event(&self, id: i32, event_id: &str) {
        match (id, event_id) {
            (ID_ROOT, "opened") => {
                self.tray.st().open = true;
            }
            (ID_ROOT, "closed") => self.tray.st().open = false,
            (_, "clicked") => self.tray.clicked(id),
            _ => {}
        }
    }
}

/// `org.unixguardian.Agent1.Tray`: shows or hides the icon, and quits.
pub struct TrayControl {
    pub tray: Option<Tray>,
}

#[zbus::interface(name = "org.unixguardian.Agent1.Tray")]
impl TrayControl {
    async fn show(&self) -> zbus::fdo::Result<()> {
        match &self.tray {
            Some(t) => {
                t.show().await;
                Ok(())
            }
            None => Err(zbus::fdo::Error::NotSupported("the agent was started with --no-tray".into())),
        }
    }

    async fn hide(&self) {
        if let Some(t) = &self.tray {
            t.hide().await;
        }
    }

    /// Same as the menu's "Quit Unix Guardian" (`ug quit`).
    async fn quit(&self) {
        // After the reply has gone out.
        tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            quit_everything();
        });
    }

    #[zbus(property)]
    fn visible(&self) -> bool {
        self.tray.as_ref().is_some_and(|t| t.visible())
    }
}

/// A program installed beside the agent, or on `PATH`.
fn find_program(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        let p = PathBuf::from(name);
        return p.is_file().then_some(p);
    }
    let beside = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(name)));
    let path = std::env::var_os("PATH").unwrap_or_default();
    beside.into_iter().chain(std::env::split_paths(&path).map(|d| d.join(name))).find(|p| is_executable(p))
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Starts a program outside the agent's own service. A child of the agent
/// would inherit the unit's `NoNewPrivileges` (so `sudo` in the opened
/// terminal would fail), its `Nice=5`, and its cgroup (so restarting the
/// agent would close the window). `systemd-run --user` makes the user
/// manager start it as its own unit instead.
async fn launch(tag: &str, argv: Vec<OsString>) -> Result<(), String> {
    if let Some(run) = find_program("systemd-run") {
        let mut cmd = tokio::process::Command::new(run);
        cmd.args(["--user", "--quiet", "--collect", "--unit"])
            .arg(format!("unix-guardian-{tag}-{}", crate::iface::now_ms()));
        for key in ["DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY", "XDG_CURRENT_DESKTOP", "XDG_SESSION_TYPE", "DBUS_SESSION_BUS_ADDRESS"]
        {
            if let Ok(v) = std::env::var(key) {
                cmd.arg("-E").arg(format!("{key}={v}"));
            }
        }
        cmd.arg("--").args(&argv).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null());
        match cmd.status().await {
            Ok(s) if s.success() => return Ok(()),
            // No user manager (an XDG-autostarted agent on a non-systemd
            // session): start it directly below.
            other => tracing::debug!(?other, "systemd-run did not start {tag}"),
        }
    }
    let mut cmd = tokio::process::Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = cmd.spawn().map_err(|e| format!("Could not start {}: {e}", argv[0].to_string_lossy()))?;
    tokio::spawn(async move {
        let _ = child.wait().await;
    });
    Ok(())
}

/// Opens the window; if it is already open it comes to the front (the window
/// is single-instance).
pub async fn open_window() -> Result<(), String> {
    let app = find_program("ug-app").ok_or("The Unix Guardian window (ug-app) is not installed. Try `ug tui` in a terminal.")?;
    launch("window", vec![app.into()]).await
}

/// Terminals to try, in order, with the flag that runs a command in them.
pub fn terminal_candidates(env_terminal: Option<&str>) -> Vec<(String, &'static str)> {
    let mut c = Vec::new();
    if let Some(t) = env_terminal.filter(|t| !t.trim().is_empty()) {
        c.push((t.trim().to_string(), "-e"));
    }
    for (t, flag) in [
        ("konsole", "-e"),
        ("gnome-terminal", "--"),
        ("kgx", "--"),
        ("xfce4-terminal", "-x"),
        ("kitty", "--"),
        ("alacritty", "-e"),
        ("wezterm", "start"),
        ("foot", "--"),
        ("x-terminal-emulator", "-e"),
        ("xterm", "-e"),
    ] {
        c.push((t.to_string(), flag));
    }
    c
}

/// Opens `ug tui` in the user's terminal.
async fn open_terminal() -> Result<(), String> {
    let ug: OsString = find_program("ug").map(Into::into).unwrap_or_else(|| "ug".into());
    let terminal = std::env::var("TERMINAL").ok();
    for (term, flag) in terminal_candidates(terminal.as_deref()) {
        let Some(path) = find_program(&term) else { continue };
        return launch("terminal", vec![path.into(), flag.into(), ug, "tui".into()]).await;
    }
    Err("No terminal program was found. Open a terminal and run: ug tui".into())
}

/// "Quit Unix Guardian": closes the windows and terminal UIs, then stops
/// this agent the way systemd would, through SIGTERM, so it shuts down in
/// order (game mode ends, playtime is saved) and exits 0. The unit is
/// `Restart=on-failure`, so a clean exit stays stopped.
pub fn quit_everything() {
    close_uis();
    tracing::info!("quitting at the user's request");
    // SAFETY: kill(2) on our own pid; the SIGTERM handler in main ends the loop.
    unsafe { libc::kill(libc::getpid(), libc::SIGTERM) };
}

/// Closes this user's Unix Guardian windows and terminal UIs.
fn close_uis() {
    close(true);
}

/// Closes this user's Unix Guardian windows (not the terminal UI, which costs
/// a game nothing): the Settings page's "close the window when a game starts".
pub fn close_windows() {
    close(false);
}

fn close(with_tui: bool) {
    if std::env::var_os("UG_TEST_RUN").is_some_and(|t| !t.is_empty()) {
        // A test agent must not close the developer's real window.
        tracing::info!("test run: not closing windows");
        return;
    }
    // SAFETY: getuid cannot fail and has no preconditions.
    let uid = unsafe { libc::getuid() };
    for (pid, tui) in ui_pids(&SysRoot::real(), uid) {
        if tui && !with_tui {
            continue;
        }
        // SAFETY: plain kill(2) on a pid just read from /proc.
        unsafe { libc::kill(pid, libc::SIGTERM) };
    }
}

/// Processes owned by `uid` that are a Unix Guardian window (`ug-app`) or
/// terminal UI (`ug tui`, marked `true`). Other `ug` commands are left alone.
fn ui_pids(root: &SysRoot, uid: u32) -> Vec<(i32, bool)> {
    let mut pids = Vec::new();
    for e in std::fs::read_dir(root.join("proc")).into_iter().flatten().flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) else { continue };
        let Ok(status) = std::fs::read_to_string(e.path().join("status")) else { continue };
        let name = status.lines().find_map(|l| l.strip_prefix("Name:")).map(str::trim);
        let owner = status
            .lines()
            .find_map(|l| l.strip_prefix("Uid:"))
            .and_then(|l| l.split_whitespace().next())
            .and_then(|u| u.parse::<u32>().ok());
        if owner != Some(uid) {
            continue;
        }
        let tui = name == Some("ug")
            && std::fs::read(e.path().join("cmdline")).is_ok_and(|c| c.split(|&b| b == 0).skip(1).any(|a| a == b"tui"));
        if name == Some("ug-app") || tui {
            pids.push((pid, tui));
        }
    }
    pids
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    fn gpu(load: f64, temp: f64, power: f64, asleep: bool) -> GpuSample {
        GpuSample {
            pci: String::new(),
            load_pct: load,
            temp_c: temp,
            power_w: power,
            asleep,
            temp_hotspot_c: f64::NAN,
            temp_mem_c: f64::NAN,
            power_limit_w: f64::NAN,
            core_clock_mhz: 0,
            mem_clock_mhz: 0,
            vram_used_mb: 0,
            fan_pct: f64::NAN,
            fan_rpm: 0,
        }
    }

    fn row(rows: &[(i32, String)], id: i32) -> &str {
        &rows.iter().find(|(i, _)| *i == id).unwrap().1
    }

    #[test]
    fn preview_shows_cpu_ram_gpu_and_battery() {
        let u = Usage {
            cpu_pct: 23.4,
            cpu_temp_c: 54.0,
            mem: MemSample { total_kb: 16 * 1024 * 1024, available_kb: 10 * 1024 * 1024, ..Default::default() },
            gpus: Some(vec![
                ("Intel(R) UHD Graphics".into(), gpu(5.0, f64::NAN, f64::NAN, false)),
                ("NVIDIA GeForce RTX 3050 Laptop GPU".into(), gpu(0.0, f64::NAN, f64::NAN, true)),
            ]),
            battery: Some((80.0, 12.3, "Discharging".into(), false)),
        };
        let r = preview_rows(&u);
        assert_eq!(row(&r, ID_CPU), "CPU  23 %  ·  54 °C");
        assert_eq!(row(&r, ID_RAM), "RAM  6.0 / 16.0 GB  (38 %)");
        assert_eq!(row(&r, ID_GPU0), "UHD  ·  5 %");
        assert_eq!(row(&r, ID_GPU0 + 1), "RTX 3050  ·  asleep");
        assert_eq!(row(&r, ID_GPU0 + 2), "", "rows without a GPU are hidden");
        assert_eq!(row(&r, ID_BATTERY), "Battery  80 %  ·  12.3 W");
    }

    #[test]
    fn preview_without_daemon_or_battery() {
        let u = Usage { cpu_pct: 3.0, cpu_temp_c: f64::NAN, gpus: None, battery: None, ..Default::default() };
        let r = preview_rows(&u);
        assert_eq!(row(&r, ID_CPU), "CPU  3 %");
        assert!(row(&r, ID_GPU0).contains("service is not running"));
        assert_eq!(row(&r, ID_GPU0 + 1), "");
        assert_eq!(row(&r, ID_BATTERY), "");
        let charging = Usage { battery: Some((40.0, -20.0, "Charging".into(), true)), ..u.clone() };
        assert_eq!(row(&preview_rows(&charging), ID_BATTERY), "Battery  40 %  ·  charging 20.0 W");
        let full = Usage { battery: Some((100.0, f64::NAN, "Full".into(), true)), ..u };
        assert_eq!(row(&preview_rows(&full), ID_BATTERY), "Battery  100 %  ·  plugged in");
    }

    /// Qt's dbusmenu importer (KDE) only accepts `u(ia{sv}av)`; a reply typed
    /// `uv` is dropped, and right-clicking the icon then shows nothing.
    #[test]
    fn layout_reply_has_the_dbusmenu_signature() {
        use zbus::zvariant::Type;
        assert_eq!(<Layout as Type>::SIGNATURE.to_string(), "(ia{sv}av)");
    }

    #[test]
    fn gpu_names_are_shortened() {
        assert_eq!(short_gpu_name("NVIDIA GeForce RTX 3050 Laptop GPU"), "RTX 3050");
        assert_eq!(short_gpu_name("AMD Radeon RX 7800 XT"), "RX 7800 XT");
        assert_eq!(short_gpu_name("Intel(R) UHD Graphics"), "UHD");
        assert_eq!(short_gpu_name(""), "GPU");
    }

    #[test]
    fn every_preview_row_is_in_the_menu() {
        for id in PREVIEW_IDS {
            assert!(MENU_ORDER.contains(&id));
        }
        assert_eq!(label_text("a_b"), "a__b");
    }

    #[test]
    fn terminal_from_env_comes_first() {
        let c = terminal_candidates(Some("foot"));
        assert_eq!(c[0], ("foot".to_string(), "-e"));
        assert_eq!(c[1].0, "konsole");
        assert_eq!(terminal_candidates(Some(" "))[0].0, "konsole");
        assert!(terminal_candidates(None).iter().any(|(t, _)| t == "x-terminal-emulator"));
    }

    #[test]
    fn only_this_users_windows_are_closed() {
        let fs = TestFs::new();
        fs.write("proc/100/status", "Name:\tug-app\nUid:\t1000\t1000\t1000\t1000\n")
            .write("proc/101/status", "Name:\tug-app\nUid:\t1001\t1001\t1001\t1001\n")
            .write("proc/102/status", "Name:\tkonsole\nUid:\t1000\t1000\t1000\t1000\n")
            .write("proc/self/status", "Name:\tug-app\nUid:\t1000\n");
        assert_eq!(ui_pids(&SysRoot::at(fs.root()), 1000), vec![(100, false)]);
    }

    #[test]
    fn terminal_uis_are_closed_but_other_ug_commands_are_not() {
        let fs = TestFs::new();
        fs.write("proc/200/status", "Name:\tug\nUid:\t1000\t1000\t1000\t1000\n")
            .write("proc/200/cmdline", "/usr/bin/ug\0tui\0")
            .write("proc/201/status", "Name:\tug\nUid:\t1000\t1000\t1000\t1000\n")
            .write("proc/201/cmdline", "ug\0monitor\0")
            .write("proc/202/status", "Name:\tug\nUid:\t1000\t1000\t1000\t1000\n")
            .write("proc/202/cmdline", "ug\0--session\0tui\0");
        let mut pids = ui_pids(&SysRoot::at(fs.root()), 1000);
        pids.sort();
        assert_eq!(pids, vec![(200, true), (202, true)], "marked as terminal UIs, which a game start leaves open");
    }
}
