//! Desktop notifications through `org.freedesktop.Notifications`.
//!
//! Called directly rather than through a notification crate: it is one method
//! and one signal, and this keeps the agent on a single D-Bus stack.

use std::collections::HashMap;
use zbus::zvariant::Value;

#[zbus::proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
pub trait Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;

    #[zbus(signal)]
    fn action_invoked(&self, id: u32, action_key: String) -> zbus::Result<()>;
}

/// Action key for "this is not a game".
pub const NOT_A_GAME: &str = "not-a-game";

/// Shows a notification; `actions` are `(key, label)` pairs. Returns the id,
/// or `None` when no notification server is running (a bare window manager),
/// which is not worth more than a debug line.
pub async fn show(conn: &zbus::Connection, summary: &str, body: &str, actions: &[(&str, &str)]) -> Option<u32> {
    let proxy = NotificationsProxy::new(conn).await.ok()?;
    let flat: Vec<&str> = actions.iter().flat_map(|(k, l)| [*k, *l]).collect();
    let mut hints = HashMap::new();
    hints.insert("desktop-entry", Value::from("unix-guardian"));
    match proxy.notify("Unix Guardian", 0, "input-gaming", summary, body, &flat, hints, 6000).await {
        Ok(id) => Some(id),
        Err(e) => {
            tracing::debug!(error = %e, "no notification server");
            None
        }
    }
}
