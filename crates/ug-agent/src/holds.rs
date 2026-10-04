//! Desktop holds a profile can ask for while game mode is on: the screen
//! saver (screen kept on, no sleep) and notifications (do not disturb).
//!
//! Holds, not settings: the desktop drops them by itself when the agent
//! leaves the session bus, so a crash leaves nothing behind. Notification
//! inhibition is KDE Plasma's extension of the notification spec; other
//! desktops do not offer it and the hold is simply not taken.

use std::collections::HashMap;
use std::time::Duration;
use zbus::proxy::MethodFlags;
use zbus::zvariant::Value;

/// A call that never starts a service just to be told no (the desktop's
/// own is running or the hold is not offered) and never holds up game mode.
async fn call<B, R>(conn: &zbus::Connection, kind: Kind, method: &str, body: &B) -> Result<R, String>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
    R: for<'d> zbus::zvariant::DynamicDeserialize<'d>,
{
    let (name, path) = match kind {
        Kind::ScreenOn => ("org.freedesktop.ScreenSaver", "/org/freedesktop/ScreenSaver"),
        Kind::DoNotDisturb => ("org.freedesktop.Notifications", "/org/freedesktop/Notifications"),
    };
    let p = zbus::Proxy::new(conn, name, path, name).await.map_err(|e| e.to_string())?;
    match tokio::time::timeout(Duration::from_secs(2), p.call_with_flags(method, MethodFlags::NoAutoStart.into(), body)).await {
        Ok(Ok(Some(r))) => Ok(r),
        Ok(Ok(None)) => Err("no reply".into()),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("no answer within 2 s".into()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    ScreenOn,
    DoNotDisturb,
}

/// One hold: taken while wanted, released when not; a refused hold is not
/// asked for again until it stops being wanted.
pub struct Hold {
    kind: Kind,
    cookie: Option<u32>,
    refused: bool,
}

impl Hold {
    pub fn new(kind: Kind) -> Self {
        Hold { kind, cookie: None, refused: false }
    }

    pub fn held(&self) -> bool {
        self.cookie.is_some()
    }

    pub async fn sync(&mut self, conn: &zbus::Connection, wanted: bool, why: &str) {
        if !wanted {
            self.refused = false;
        }
        match (self.cookie, wanted && !self.refused) {
            (None, true) => {
                let r = match self.kind {
                    Kind::ScreenOn => call::<_, u32>(conn, self.kind, "Inhibit", &("Unix Guardian", why)).await,
                    Kind::DoNotDisturb => {
                        let hints: HashMap<&str, Value<'_>> = HashMap::new();
                        call::<_, u32>(conn, self.kind, "Inhibit", &("unix-guardian", why, hints)).await
                    }
                };
                match r {
                    Ok(c) => {
                        tracing::info!(hold = ?self.kind, "desktop hold taken");
                        self.cookie = Some(c);
                    }
                    Err(e) => {
                        tracing::info!(hold = ?self.kind, error = %e, "the desktop does not offer this hold");
                        self.refused = true;
                    }
                }
            }
            (Some(c), false) => {
                if let Err(e) = call::<_, ()>(conn, self.kind, "UnInhibit", &(c,)).await {
                    tracing::warn!(hold = ?self.kind, error = %e, "could not release a desktop hold");
                }
                tracing::info!(hold = ?self.kind, "desktop hold released");
                self.cookie = None;
            }
            _ => {}
        }
    }
}
