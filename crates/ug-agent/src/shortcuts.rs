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

use futures_lite::StreamExt;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
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

pub const ACTIONS: [Action; 3] = [
    Action { id: "open-window", description: "Open Unix Guardian", preferred: "LOGO+SHIFT+u" },
    Action { id: "toggle-overlay", description: "Show or hide the in-game overlay", preferred: "LOGO+SHIFT+o" },
    Action { id: "toggle-gamemode", description: "Turn game mode on or off", preferred: "LOGO+SHIFT+g" },
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
    #[zbus(property)]
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
        let enabled = std::fs::read_to_string(&file)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v.get("enabled").and_then(|e| e.as_bool()))
            .unwrap_or(false);
        Arc::new(Shortcuts {
            file,
            inner: Mutex::new(Inner { enabled, status: "off".into(), ..Default::default() }),
            pressed,
        })
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
        let text = serde_json::json!({ "enabled": on }).to_string();
        ug_core::paths::write_atomic(&self.file, text.as_bytes()).map_err(|e| format!("{}: {e}", self.file.display()))?;
        let (task, session) = {
            let mut i = self.lock();
            i.enabled = on;
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

        self.lock().status = "waiting".into();
        let wanted: Vec<(&str, HashMap<&str, Value<'_>>)> = ACTIONS
            .iter()
            .map(|a| {
                let mut m = HashMap::new();
                m.insert("description", Value::from(a.description));
                m.insert("preferred_trigger", Value::from(a.preferred));
                (a.id, m)
            })
            .collect();
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
pub async fn perform(id: &str, core: &crate::iface::Shared, overlay: &Arc<crate::overlay::Overlay>) -> Result<String, String> {
    match id {
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
