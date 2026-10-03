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
//! terminal UI" and "Quit". Quit closes the window and hides the icon; the
//! agent keeps running, because game mode, playtime and the overlay depend on
//! it. `ug tray` (or the next login) brings the icon back.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ug_hw::SysRoot;
use ug_proto::{GpuInfo, GpuSample, MemSample};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, StructureBuilder, Value};

const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/MenuBar";
const WATCHERS: [&str; 2] = ["org.kde.StatusNotifierWatcher", "org.freedesktop.StatusNotifierWatcher"];

/// How often the preview updates while the menu is open.
const LIVE_EVERY: Duration = Duration::from_secs(2);
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
const ID_CPU: i32 = 10;
const ID_RAM: i32 = 11;
const ID_GPU0: i32 = 12;
const ID_BATTERY: i32 = ID_GPU0 + MAX_GPUS as i32;

#[cfg(test)]
const PREVIEW_IDS: [i32; 3 + MAX_GPUS] = [ID_CPU, ID_RAM, ID_GPU0, ID_GPU0 + 1, ID_GPU0 + 2, ID_GPU0 + 3, ID_BATTERY];
const MENU_ORDER: [i32; 6 + 3 + MAX_GPUS] = [
    ID_OPEN, ID_SEP1, ID_CPU, ID_RAM, ID_GPU0, ID_GPU0 + 1, ID_GPU0 + 2, ID_GPU0 + 3, ID_BATTERY, ID_SEP2, ID_TERMINAL,
    ID_SEP3, ID_QUIT,
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
    load_read_at: Option<Instant>,
    gpu_names: Vec<GpuInfo>,
}

/// The tray, shared by its D-Bus interfaces and the agent's `Tray` interface.
#[derive(Clone)]
pub struct Tray {
    conn: zbus::Connection,
    name: String,
    daemon_session: bool,
    state: Arc<Mutex<State>>,
    live: Arc<tokio::sync::Notify>,
}

impl Tray {
    fn st(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Serves the icon and menu on `conn`, shows the icon, and keeps it
    /// registered across panel restarts.
    pub async fn start(conn: &zbus::Connection, daemon_session: bool) -> zbus::Result<Tray> {
        let tray = Tray {
            conn: conn.clone(),
            name: format!("org.kde.StatusNotifierItem-{}-1", std::process::id()),
            daemon_session,
            state: Arc::new(Mutex::new(State {
                visible: false,
                rows: HashMap::new(),
                open: false,
                load: ug_hw::cpu::LoadReader::new(),
                load_read_at: None,
                gpu_names: Vec::new(),
            })),
            live: Arc::default(),
        };
        conn.object_server().at(ITEM_PATH, ItemIface { tray: tray.clone() }).await?;
        conn.object_server().at(MENU_PATH, MenuIface { tray: tray.clone() }).await?;
        tokio::spawn(tray.clone().follow_watcher());
        tokio::spawn(tray.clone().live_preview());
        tray.show().await;
        Ok(tray)
    }

    pub fn visible(&self) -> bool {
        self.st().visible
    }

    /// Puts the icon in the panel (again).
    pub async fn show(&self) {
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

    /// Takes the icon out of the panel. The watcher notices the name going.
    pub async fn hide(&self) {
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

    /// Re-reads usage every two seconds while the menu is open, and never
    /// otherwise.
    async fn live_preview(self) {
        loop {
            self.live.notified().await;
            loop {
                tokio::time::sleep(LIVE_EVERY).await;
                if !self.st().open {
                    break;
                }
                let changed = self.refresh().await;
                self.emit_rows(&changed).await;
            }
        }
    }

    /// Reads usage and updates the rows. Returns the ids whose text changed.
    async fn refresh(&self) -> Vec<i32> {
        let gpus = self.read_gpus().await;
        let state = Arc::clone(&self.state);
        let local = tokio::task::spawn_blocking(move || {
            let root = SysRoot::real();
            let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
            // A load figure needs two reads. After a long gap, the first menu
            // opening would show the average since then: take a short fresh one.
            let stale = st.load_read_at.is_none_or(|t| t.elapsed() > LIVE_EVERY * 3);
            if stale {
                st.load.read(&root);
                drop(st);
                std::thread::sleep(Duration::from_millis(150));
                st = state.lock().unwrap_or_else(|e| e.into_inner());
            }
            let cpu_pct = st.load.read(&root).total_pct;
            st.load_read_at = Some(Instant::now());
            drop(st);
            let chips = ug_hw::hwmon::enumerate(&root);
            let cpu_temp_c = ug_hw::hwmon::cpu_chip(&chips).and_then(|c| c.package_temp()).unwrap_or(f64::NAN);
            let (pct, w, status, on_ac) = ug_hw::extra::battery(&root);
            let battery = (!pct.is_nan()).then_some((pct, w, status, on_ac));
            (cpu_pct, cpu_temp_c, ug_hw::mem::read(&root), battery)
        })
        .await;
        let Ok((cpu_pct, cpu_temp_c, mem, battery)) = local else { return Vec::new() };
        let usage = Usage { cpu_pct, cpu_temp_c, mem, gpus, battery };
        let mut st = self.st();
        let mut changed = Vec::new();
        for (id, text) in preview_rows(&usage) {
            if st.rows.get(&id) != Some(&text) {
                st.rows.insert(id, text);
                changed.push(id);
            }
        }
        changed
    }

    /// GPU readings from the daemon (it owns NVML and never wakes a sleeping
    /// GPU). `None` when it cannot be reached within a second.
    async fn read_gpus(&self) -> Option<Vec<(String, GpuSample)>> {
        let read = async {
            let conn =
                if self.daemon_session { zbus::Connection::session().await } else { zbus::Connection::system().await }.ok()?;
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
        tokio::time::timeout(Duration::from_secs(1), read).await.ok().flatten()
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
            ID_QUIT => {
                label(&mut p, "Quit");
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

    fn layout_of(&self, id: i32, with_children: bool) -> zbus::fdo::Result<Value<'static>> {
        let children: Vec<Value<'static>> = if with_children && id == ID_ROOT {
            MENU_ORDER.iter().map(|&c| self.layout_of(c, false)).collect::<zbus::fdo::Result<_>>()?
        } else {
            Vec::new()
        };
        let children: Vec<OwnedValue> = children.into_iter().map(ov).collect();
        StructureBuilder::new()
            .add_field(id)
            .add_field(self.props(id))
            .add_field(children)
            .build()
            .map(Value::from)
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    fn clicked(&self, id: i32) {
        let tray = self.clone();
        tokio::spawn(async move {
            let result = match id {
                ID_OPEN => open_window().await,
                ID_TERMINAL => open_terminal().await,
                ID_QUIT => {
                    close_windows();
                    tray.hide().await;
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

fn pixmaps() -> Vec<Pixmap> {
    [
        (22, &include_bytes!("../assets/tray-22.argb")[..]),
        (32, &include_bytes!("../assets/tray-32.argb")[..]),
        (48, &include_bytes!("../assets/tray-48.argb")[..]),
        (64, &include_bytes!("../assets/tray-64.argb")[..]),
    ]
    .into_iter()
    .map(|(s, b)| (s, s, b.to_vec()))
    .collect()
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
    /// Empty, so hosts use the pixmap: no icon has to be installed in a theme.
    #[zbus(property)]
    fn icon_name(&self) -> &str {
        ""
    }
    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<Pixmap> {
        pixmaps()
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
        (String::new(), Vec::new(), "Unix Guardian".into(), "Click to open · right-click for usage".into())
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
}

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
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> zbus::fdo::Result<(u32, Value<'static>)> {
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
        let changed = self.tray.refresh().await;
        if !changed.is_empty() {
            let updated = changed.iter().map(|&i| (i, self.tray.props(i))).collect();
            let _ = Self::items_properties_updated(&emitter, updated, Vec::new()).await;
        }
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
                self.tray.live.notify_one();
            }
            (ID_ROOT, "closed") => self.tray.st().open = false,
            (_, "clicked") => self.tray.clicked(id),
            _ => {}
        }
    }
}

/// `org.unixguardian.Agent1.Tray`: brings the icon back after Quit.
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
async fn open_window() -> Result<(), String> {
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

/// Closes this user's Unix Guardian windows.
fn close_windows() {
    if std::env::var_os("UG_TEST_RUN").is_some_and(|t| !t.is_empty()) {
        // A test agent must not close the developer's real window.
        tracing::info!("test run: not closing windows");
        return;
    }
    // SAFETY: getuid cannot fail and has no preconditions.
    let uid = unsafe { libc::getuid() };
    for pid in window_pids(&SysRoot::real(), uid) {
        // SAFETY: plain kill(2) on a pid just read from /proc.
        unsafe { libc::kill(pid, libc::SIGTERM) };
    }
}

/// Processes named `ug-app` owned by `uid`.
fn window_pids(root: &SysRoot, uid: u32) -> Vec<i32> {
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
        if name == Some("ug-app") && owner == Some(uid) {
            pids.push(pid);
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
        assert_eq!(window_pids(&SysRoot::at(fs.root()), 1000), vec![100]);
    }
}
