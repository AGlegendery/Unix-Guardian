//! Global shortcuts through the XDG GlobalShortcuts portal.
//!
//! On Wayland a program cannot grab keys; the desktop owns them, and its
//! portal tells the program when one of the program's shortcuts is pressed.
//! The agent asks for three — open the window, show or hide the overlay, turn
//! game mode on or off — with suggested keys the desktop may change, and the
//! user changes in the desktop's own settings (KDE asks the first time).
//!
//! Off until the user turns them on: binding makes the desktop show a dialog,
//! which must not appear by surprise. Turned on, the agent binds them again
//! at every start (the desktop remembers the keys).
//!
//! The portal talks on a connection of its own: a program without a sandbox
//! has to tell the portal which application it is
//! (`org.freedesktop.host.portal.Registry.Register`) before its first portal
//! call on a connection, and the agent's main connection may have made one.
//!
//! Without the portal (GNOME before 48, some tiling compositors) nothing is
//! bound and the page says so, with the commands to bind in the desktop's own
//! shortcut settings instead (`ug shortcut <action>`).
//!
//! Keys the user picks (`SetKeys`, recorded in the window or the TUI) are the
//! suggestion at binding. The portal has no call to change keys later, and
//! desktops keep the keys they already have, so on KDE the agent sets them
//! in kglobalaccel directly (the portal's own store; the portal then reports
//! the new keys); elsewhere it binds again with the new suggestion and says
//! the desktop may keep its own.

use futures_lite::StreamExt;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use crate::keys::Combo;
use ug_proto::{ShortcutInfo, ShortcutsState};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

/// A shortcut: its id with the portal, what it does, and the suggested keys
/// (the XDG shortcuts format: modifiers CTRL, ALT, SHIFT, NUM, LOGO joined
/// with `+`, then an xkbcommon key name).
pub struct Action {
    pub id: &'static str,
    pub description: &'static str,
    pub preferred: &'static str,
}

pub const ACTIONS: [Action; 5] = [
    Action { id: "open-window", description: "Open Unix Guardian", preferred: "LOGO+SHIFT+u" },
    Action { id: "toggle-overlay", description: "Show or hide the in-game overlay", preferred: "LOGO+SHIFT+o" },
    Action { id: "toggle-gamemode", description: "Turn game mode on or off", preferred: "LOGO+SHIFT+g" },
    Action { id: "toggle-recording", description: "Start or stop recording", preferred: "LOGO+SHIFT+r" },
    Action { id: "save-replay", description: "Save the last seconds (replay)", preferred: "LOGO+SHIFT+s" },
];

/// The application id the portal knows Unix Guardian by: the basename of its
/// desktop file (`unix-guardian.desktop`).
pub const APP_ID: &str = "unix-guardian";

#[zbus::proxy(
    interface = "org.freedesktop.portal.GlobalShortcuts",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop"
)]
trait GlobalShortcuts {
    fn create_session(&self, options: HashMap<&str, Value<'_>>) -> zbus::Result<OwnedObjectPath>;
    fn bind_shortcuts(
        &self,
        session_handle: &ObjectPath<'_>,
        shortcuts: &[(&str, HashMap<&str, Value<'_>>)],
        parent_window: &str,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<OwnedObjectPath>;
    // Lower case on the bus; zbus would ask for "Version".
    #[zbus(property, name = "version")]
    fn version(&self) -> zbus::Result<u32>;
    #[zbus(signal)]
    fn activated(&self, session_handle: OwnedObjectPath, shortcut_id: String, timestamp: u64, options: HashMap<String, OwnedValue>) -> zbus::Result<()>;
    #[zbus(signal)]
    fn shortcuts_changed(&self, session_handle: OwnedObjectPath, shortcuts: Vec<(String, HashMap<String, OwnedValue>)>) -> zbus::Result<()>;
}

#[zbus::proxy(interface = "org.freedesktop.portal.Request", default_service = "org.freedesktop.portal.Desktop")]
trait Request {
    #[zbus(signal)]
    fn response(&self, response: u32, results: HashMap<String, OwnedValue>) -> zbus::Result<()>;
}

#[zbus::proxy(interface = "org.freedesktop.portal.Session", default_service = "org.freedesktop.portal.Desktop")]
trait Session {
    fn close(&self) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.host.portal.Registry",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop"
)]
trait Registry {
    fn register(&self, app_id: &str, options: HashMap<&str, Value<'_>>) -> zbus::Result<()>;
}

/// KDE's shortcut store, where the portal keeps the keys it gives programs
/// (component = the application id, action = the shortcut id).
#[zbus::proxy(interface = "org.kde.KGlobalAccel", default_service = "org.kde.kglobalaccel", default_path = "/kglobalaccel")]
trait KGlobalAccel {
    /// Sets the keys of another program's action and tells that program.
    #[zbus(name = "setForeignShortcutKeys")]
    fn set_foreign_shortcut_keys(&self, action_id: &[&str], keys: &[(Vec<i32>,)]) -> zbus::Result<()>;
    #[zbus(name = "shortcutKeys")]
    fn shortcut_keys(&self, action_id: &[&str]) -> zbus::Result<Vec<(Vec<i32>,)>>;
    /// Who uses a key (match type 0: exactly this key).
    #[zbus(name = "globalShortcutsByKey")]
    #[allow(clippy::type_complexity)]
    fn global_shortcuts_by_key(
        &self,
        key: &(Vec<i32>,),
        match_type: &(i32,),
    ) -> zbus::Result<Vec<(String, String, String, String, String, String, Vec<i32>, Vec<i32>)>>;
}

/// What to bind instead, where there is no portal.
fn fallback() -> String {
    let cmds: Vec<String> = ACTIONS.iter().map(|a| format!("ug shortcut {}", a.id)).collect();
    format!(
        "This desktop does not offer global shortcuts to programs (no GlobalShortcuts portal). Bind these commands to keys in its own shortcut settings instead: {}.",
        cmds.join(", ")
    )
}

#[derive(Default)]
struct Inner {
    enabled: bool,
    /// Keys the user picked (XDG format), by shortcut id.
    chosen: HashMap<String, String>,
    status: String,
    problem: String,
    triggers: HashMap<String, String>,
    /// The portal session and its connection, to close it.
    session: Option<(zbus::Connection, OwnedObjectPath)>,
    task: Option<tokio::task::JoinHandle<()>>,
}

pub struct Shortcuts {
    file: PathBuf,
    inner: Mutex<Inner>,
    /// Shortcut ids pressed, for the agent's main loop to act on.
    pressed: tokio::sync::mpsc::UnboundedSender<String>,
}

impl Shortcuts {
    /// The choice lives in `shortcuts.json` in the configuration directory.
    pub fn new(config_dir: &std::path::Path, pressed: tokio::sync::mpsc::UnboundedSender<String>) -> Arc<Shortcuts> {
        let file = config_dir.join("shortcuts.json");
        let saved = std::fs::read_to_string(&file)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .unwrap_or_default();
        let enabled = saved.get("enabled").and_then(|e| e.as_bool()).unwrap_or(false);
        // Only keys that still read as a shortcut, for shortcuts that exist.
        let chosen = saved
            .get("keys")
            .and_then(|k| k.as_object())
            .map(|k| {
                k.iter()
                    .filter(|(id, _)| ACTIONS.iter().any(|a| a.id == id.as_str()))
                    .filter_map(|(id, v)| Some((id.clone(), Combo::parse(v.as_str()?).ok()?.xdg())))
                    .collect()
            })
            .unwrap_or_default();
        Arc::new(Shortcuts {
            file,
            inner: Mutex::new(Inner { enabled, chosen, status: "off".into(), ..Default::default() }),
            pressed,
        })
    }

    fn save(&self) -> Result<(), String> {
        let text = {
            let i = self.lock();
            serde_json::json!({ "enabled": i.enabled, "keys": i.chosen }).to_string()
        };
        ug_core::paths::write_atomic(&self.file, text.as_bytes()).map_err(|e| format!("{}: {e}", self.file.display()))
    }

    /// The keys to ask for: the user's, or the suggested ones.
    fn wanted(i: &Inner, a: &Action) -> Combo {
        i.chosen
            .get(a.id)
            .and_then(|k| Combo::parse(k).ok())
            .unwrap_or_else(|| Combo::parse(a.preferred).expect("the suggested keys parse"))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn state(&self) -> ShortcutsState {
        let i = self.lock();
        ShortcutsState {
            enabled: i.enabled,
            status: i.status.clone(),
            problem: i.problem.clone(),
            shortcuts: ACTIONS
                .iter()
                .map(|a| ShortcutInfo {
                    id: a.id.into(),
                    description: a.description.into(),
                    trigger: i.triggers.get(a.id).cloned().unwrap_or_default(),
                    keys: Self::wanted(&i, a).label(),
                    chosen: i.chosen.contains_key(a.id),
                })
                .collect(),
            gamemode_by_hand: String::new(),
        }
    }

    /// Binds them if the user turned them on (at start-up).
    pub fn start(self: &Arc<Self>) {
        if self.lock().enabled {
            self.connect();
        }
    }

    fn connect(self: &Arc<Self>) {
        let me = Arc::clone(self);
        let task = tokio::spawn(async move {
            let result = me.run().await;
            let mut i = me.lock();
            match result {
                Ok(()) => {}
                Err((status, problem)) => {
                    tracing::info!(status, problem = %problem, "global shortcuts not bound");
                    i.status = status.into();
                    i.problem = problem;
                }
            }
        });
        let mut i = self.lock();
        i.status = "connecting".into();
        i.problem.clear();
        if let Some(old) = i.task.replace(task) {
            old.abort();
        }
    }

    /// Turns them on or off, and remembers it.
    pub async fn set_enabled(self: &Arc<Self>, on: bool) -> Result<(), String> {
        let was = std::mem::replace(&mut self.lock().enabled, on);
        if let Err(e) = self.save() {
            self.lock().enabled = was;
            return Err(e);
        }
        let (task, session) = {
            let mut i = self.lock();
            (i.task.take(), i.session.take())
        };
        if let Some(t) = task {
            t.abort();
        }
        if let Some((conn, path)) = session {
            close(&conn, path).await;
        }
        if on {
            self.connect();
        } else {
            let mut i = self.lock();
            i.status = "off".into();
            i.problem.clear();
            i.triggers.clear();
        }
        Ok(())
    }

    /// Closes the portal session (agent shutdown).
    pub async fn shutdown(&self) {
        let session = self.lock().session.take();
        if let Some((conn, path)) = session {
            close(&conn, path).await;
        }
    }

    /// Talks to the portal until the session ends. Errors are (status,
    /// problem) for the page.
    async fn run(self: &Arc<Self>) -> Result<(), (&'static str, String)> {
        let failed = |what: &str, e: zbus::Error| ("failed", format!("{what}: {e}"));
        let conn = zbus::Connection::session().await.map_err(|e| failed("no session bus", e))?;
        let portal = GlobalShortcutsProxy::new(&conn).await.map_err(|e| failed("the portal", e))?;
        // No such interface (or no portal at all): say what to do instead.
        if portal.version().await.is_err() {
            return Err(("unavailable", fallback()));
        }
        // Tell the portal which application this is; refused by portals that
        // know already (a desktop-started app) or predate the call. Harmless.
        if let Ok(reg) = RegistryProxy::new(&conn).await {
            if let Err(e) = reg.register(APP_ID, HashMap::new()).await {
                tracing::debug!(error = %e, "portal registry");
            }
        }

        let sender = conn.unique_name().map(|n| n.trim_start_matches(':').replace('.', "_")).unwrap_or_default();
        let token = format!("ug{}", std::process::id());

        let session_token = token.clone();
        let results = request(&conn, &sender, &format!("{token}s"), |opts| {
            let portal = portal.clone();
            async move {
                let mut opts = opts;
                opts.insert("session_handle_token", Value::from(session_token));
                portal.create_session(opts).await
            }
        })
        .await?;
        let session: OwnedObjectPath = results
            .get("session_handle")
            .and_then(|v| {
                String::try_from(v.try_clone().ok()?)
                    .ok()
                    .and_then(|s| OwnedObjectPath::try_from(s).ok())
                    .or_else(|| OwnedObjectPath::try_from(v.try_clone().ok()?).ok())
            })
            .ok_or(("failed", "the portal gave no session".to_string()))?;
        self.lock().session = Some((conn.clone(), session.clone()));

        // Listen before binding, so no press is missed.
        let mut pressed = portal.receive_activated().await.map_err(|e| failed("listening for shortcuts", e))?;
        let mut changed = portal.receive_shortcuts_changed().await.map_err(|e| failed("listening for shortcuts", e))?;

        let wanted: Vec<(&str, HashMap<&str, Value<'_>>)> = {
            let mut i = self.lock();
            i.status = "waiting".into();
            ACTIONS
                .iter()
                .map(|a| {
                    let mut m = HashMap::new();
                    m.insert("description", Value::from(a.description));
                    m.insert("preferred_trigger", Value::from(Self::wanted(&i, a).xdg()));
                    (a.id, m)
                })
                .collect()
        };
        let bound = request(&conn, &sender, &format!("{token}b"), |opts| {
            let (portal, session, wanted) = (portal.clone(), session.clone(), wanted.clone());
            async move { portal.bind_shortcuts(&session, &wanted, "", opts).await }
        })
        .await?;
        self.set_triggers(bound.get("shortcuts"));
        {
            let mut i = self.lock();
            i.status = "active".into();
            i.problem.clear();
        }
        tracing::info!(shortcuts = ?self.lock().triggers, "global shortcuts bound");
        // The desktop keeps keys it had from an earlier binding; the user's
        // choice wins where it can be set (KDE).
        let chosen: Vec<&Action> = {
            let i = self.lock();
            ACTIONS.iter().filter(|a| i.chosen.contains_key(a.id)).collect()
        };
        for a in chosen {
            let keys = Self::wanted(&self.lock(), a);
            match kde_set(&conn, a, &keys).await {
                Ok(true) => {
                    self.lock().triggers.insert(a.id.into(), keys.label());
                }
                Ok(false) => break,
                Err(e) => tracing::info!(shortcut = a.id, error = %e, "keeping the desktop's keys"),
            }
        }

        loop {
            tokio::select! {
                Some(sig) = pressed.next() => {
                    let Ok(args) = sig.args() else { continue };
                    if args.session_handle == session {
                        let _ = self.pressed.send(args.shortcut_id.clone());
                    }
                }
                Some(sig) = changed.next() => {
                    let Ok(args) = sig.args() else { continue };
                    if args.session_handle == session {
                        let list: Vec<(String, HashMap<String, OwnedValue>)> = args.shortcuts.clone();
                        let mut i = self.lock();
                        for (id, props) in list {
                            if let Some(t) = props.get("trigger_description").and_then(|v| String::try_from(v.try_clone().ok()?).ok()) {
                                i.triggers.insert(id, t);
                            }
                        }
                    }
                }
                else => return Err(("failed", "the portal went away".into())),
            }
        }
    }

    /// Uses `keys` for shortcut `id` (any common spelling; "" goes back to
    /// the suggested keys), remembered, and says what happened.
    pub async fn set_keys(self: &Arc<Self>, id: &str, keys: &str) -> Result<String, String> {
        let Some(action) = ACTIONS.iter().find(|a| a.id == id) else {
            let ids: Vec<&str> = ACTIONS.iter().map(|a| a.id).collect();
            return Err(format!("Unknown shortcut `{id}`; known: {}.", ids.join(", ")));
        };
        let combo = if keys.trim().is_empty() {
            Combo::parse(action.preferred).expect("the suggested keys parse")
        } else {
            Combo::parse(keys)?
        };
        let (status, session) = {
            let i = self.lock();
            if let Some(other) = ACTIONS.iter().find(|a| a.id != id && Self::wanted(&i, a) == combo) {
                return Err(format!("{} is already the shortcut for \"{}\".", combo.label(), other.description));
            }
            (i.status.clone(), i.session.clone())
        };
        // Bound on KDE: set the keys there first, so a key another program
        // holds is refused before anything is saved.
        let mut done_in_kde = false;
        if let (Some((conn, _)), "active") = (&session, status.as_str()) {
            done_in_kde = kde_set(conn, action, &combo).await?;
        }
        let previous = {
            let mut i = self.lock();
            let previous = if keys.trim().is_empty() {
                i.chosen.remove(id)
            } else {
                i.chosen.insert(id.into(), combo.xdg())
            };
            if done_in_kde {
                i.triggers.insert(id.into(), combo.label());
            }
            previous
        };
        if let Err(e) = self.save() {
            let mut i = self.lock();
            match previous {
                Some(p) => i.chosen.insert(id.into(), p),
                None => i.chosen.remove(id),
            };
            return Err(e);
        }
        let label = combo.label();
        Ok(match status.as_str() {
            _ if done_in_kde => format!("{label} now does \"{}\".", action.description),
            "active" | "connecting" | "waiting" => {
                // Bind again with the new suggestion.
                let (task, session) = {
                    let mut i = self.lock();
                    (i.task.take(), i.session.take())
                };
                if let Some(t) = task {
                    t.abort();
                }
                if let Some((conn, path)) = session {
                    close(&conn, path).await;
                }
                self.connect();
                format!(
                    "Asked the desktop for {label}. If it keeps the keys it had, change them in its own shortcut settings."
                )
            }
            "unavailable" => format!(
                "Saved. This desktop does not offer shortcuts to programs: bind `ug shortcut {id}` to {label} in its own shortcut settings."
            ),
            _ => format!("Saved: {label}, used when the shortcuts are turned on."),
        })
    }

    fn set_triggers(&self, v: Option<&OwnedValue>) {
        let Some(v) = v.and_then(|v| v.try_clone().ok()) else { return };
        let Ok(list) = <Vec<(String, HashMap<String, OwnedValue>)>>::try_from(v) else { return };
        let mut i = self.lock();
        for (id, props) in list {
            let t = props.get("trigger_description").and_then(|v| String::try_from(v.try_clone().ok()?).ok()).unwrap_or_default();
            i.triggers.insert(id, t);
        }
    }
}

/// Sets shortcut `a`'s keys in KDE's kglobalaccel. `Ok(false)`: not KDE (or
/// the shortcut is not there to change); an error says why KDE did not take
/// them, e.g. another program's shortcut uses the key.
async fn kde_set(conn: &zbus::Connection, a: &Action, keys: &Combo) -> Result<bool, String> {
    let dbus = zbus::fdo::DBusProxy::new(conn).await.map_err(|e| e.to_string())?;
    let name = zbus::names::BusName::try_from("org.kde.kglobalaccel").map_err(|e| e.to_string())?;
    if !dbus.name_has_owner(name).await.unwrap_or(false) {
        return Ok(false);
    }
    let kde = KGlobalAccelProxy::new(conn).await.map_err(|e| e.to_string())?;
    let action_id = [APP_ID, a.id, APP_ID, a.description];
    let now = kde.shortcut_keys(&action_id).await.map_err(|e| format!("KDE: {e}"))?;
    if now.is_empty() {
        return Ok(false);
    }
    let code = keys.qt();
    if now.iter().any(|k| k.0.first() == Some(&code)) {
        return Ok(true);
    }
    let users = kde.global_shortcuts_by_key(&(vec![code],), &(0,)).await.map_err(|e| format!("KDE: {e}"))?;
    if let Some(u) = users.iter().find(|u| !(u.2 == APP_ID && u.4 == a.id)) {
        let what = if u.5.is_empty() { &u.4 } else { &u.5 };
        let who = if u.3.is_empty() { &u.2 } else { &u.3 };
        return Err(format!(
            "{} is already used by {who} (\"{what}\"). Pick other keys, or free them in the desktop's shortcut settings.",
            keys.label()
        ));
    }
    kde.set_foreign_shortcut_keys(&action_id, &[(vec![code],)]).await.map_err(|e| format!("KDE refused the keys: {e}"))?;
    let after = kde.shortcut_keys(&action_id).await.map_err(|e| format!("KDE: {e}"))?;
    if !after.iter().any(|k| k.0.first() == Some(&code)) {
        return Err("KDE did not take the keys. Change them in System Settings, Shortcuts.".into());
    }
    Ok(true)
}

async fn close(conn: &zbus::Connection, path: OwnedObjectPath) {
    let proxy = SessionProxy::builder(conn).path(path).ok().map(|b| b.build());
    if let Some(p) = proxy {
        if let Ok(p) = p.await {
            if let Err(e) = p.close().await {
                tracing::debug!(error = %e, "closing the shortcuts session");
            }
        }
    }
}

/// A portal call that answers through a Request object: subscribes to the
/// request's `Response` first (its path is known from the token), then
/// calls. Response 1 is "the user cancelled", 2 "refused".
async fn request<F, Fut>(
    conn: &zbus::Connection,
    sender: &str,
    token: &str,
    call: F,
) -> Result<HashMap<String, OwnedValue>, (&'static str, String)>
where
    F: FnOnce(HashMap<&'static str, Value<'static>>) -> Fut,
    Fut: std::future::Future<Output = zbus::Result<OwnedObjectPath>>,
{
    let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    let req = RequestProxy::builder(conn)
        .path(path.clone())
        .map_err(|e| ("failed", e.to_string()))?
        .build()
        .await
        .map_err(|e| ("failed", e.to_string()))?;
    let mut responses = req.receive_response().await.map_err(|e| ("failed", e.to_string()))?;
    let mut opts = HashMap::new();
    opts.insert("handle_token", Value::from(token.to_string()));
    let handle = call(opts).await.map_err(|e| ("failed", format!("the portal refused: {e}")))?;
    if handle.as_str() != path {
        tracing::debug!(expected = %path, got = %handle, "portal request path differs");
    }
    let Some(sig) = responses.next().await else {
        return Err(("failed", "the portal did not answer".into()));
    };
    let args = sig.args().map_err(|e| ("failed", e.to_string()))?;
    match args.response {
        0 => Ok(args.results.clone()),
        1 => Err(("failed", "The desktop's confirmation was cancelled. Turn the shortcuts off and on to be asked again.".into())),
        _ => Err(("failed", "The desktop refused the shortcuts.".into())),
    }
}

/// Does what shortcut `id` does and says what happened, in a sentence for a
/// notification or the CLI. Game mode follows at the agent's next tick.
pub async fn perform(
    id: &str,
    core: &crate::iface::Shared,
    overlay: &Arc<crate::overlay::Overlay>,
    recorder: &Arc<crate::recorder::Recorder>,
) -> Result<String, String> {
    let rec = Arc::clone(recorder);
    let blocking = |f: Box<dyn FnOnce(&crate::recorder::Recorder) -> Result<String, String> + Send>| async move {
        tokio::task::spawn_blocking(move || f(&rec)).await.map_err(|e| e.to_string())?
    };
    match id {
        "toggle-recording" => {
            blocking(Box::new(|r| {
                if r.is_recording() {
                    r.stop().map(|f| format!("Recording saved: {f}"))
                } else {
                    r.start().map(|f| format!("Recording to {f}"))
                }
            }))
            .await
        }
        "save-replay" => {
            blocking(Box::new(|r| {
                if r.is_replaying() {
                    r.save_replay().map(|f| format!("Replay saved: {f}"))
                } else {
                    r.start_replay().map(|()| "Keeping the last seconds now; press the shortcut again to save them.".to_string())
                }
            }))
            .await
        }
        "open-window" => crate::tray::open_window().await.map(|()| "Opening Unix Guardian.".to_string()),
        "toggle-overlay" => {
            let o = Arc::clone(overlay);
            let shown = tokio::task::spawn_blocking(move || o.toggle()).await.map_err(|e| e.to_string())??;
            Ok(if shown { "Overlay shown." } else { "Overlay hidden." }.to_string())
        }
        "toggle-gamemode" => {
            let on = crate::iface::lock(core).toggle_gamemode();
            Ok(if on { "Game mode on." } else { "Game mode off." }.to_string())
        }
        other => {
            let ids: Vec<&str> = ACTIONS.iter().map(|a| a.id).collect();
            Err(format!("Unknown shortcut `{other}`; known: {}.", ids.join(", ")))
        }
    }
}
