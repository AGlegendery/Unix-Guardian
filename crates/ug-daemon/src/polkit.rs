//! Authorising callers through polkit.
//!
//! The daemon runs as root and anyone on the machine can call it, so every
//! method that changes state asks polkit whether *this caller* may perform
//! *this action* before doing anything. The policy file decides the answer —
//! usually "an administrator, with the password cached for a few minutes" —
//! and the desktop's polkit agent shows the password prompt. The daemon never
//! sees a password.
//!
//! On the session bus (`--session`, for development) there is nothing to
//! authorise: every caller is already the user who started the daemon, and the
//! daemon has no privileges to protect. Checks are skipped there, loudly.

use std::collections::HashMap;
use zbus::zvariant::Value;

/// Actions the policy file defines. Kept as constants so a typo is a compile
/// error rather than a silent "not authorised" at runtime.
///
/// An action is added here in the same change as the first method that checks
/// it, never ahead of time: an action in the installed policy that nothing
/// enforces misleads anyone auditing what the daemon can be asked to do.
pub mod action {
    /// Change runtime diagnostics such as the log level.
    pub const DAEMON_DEBUG: &str = "org.unixguardian.daemon.debug";
    /// Apply game mode while a game runs.
    pub const GAMEMODE: &str = "org.unixguardian.gamemode";
    /// Shut down, restart or sleep, now or on a schedule.
    pub const POWER: &str = "org.unixguardian.power.control";
    /// Block an app's internet, or cap its upload or download.
    pub const NET: &str = "org.unixguardian.net.control";
    /// Change a GPU's power limit or clock offsets.
    pub const GPU_TUNE: &str = "org.unixguardian.gpu.tune";
    /// Fan curves and the firmware's fan modes.
    pub const COOLING: &str = "org.unixguardian.cooling";
    /// Graphics driver fixes and their undo (password every time).
    pub const DRIVER: &str = "org.unixguardian.driver.fix";

    /// Every action, for the test that keeps this list and the shipped
    /// policy file in step.
    #[cfg(test)]
    pub const ALL: &[&str] = &[DAEMON_DEBUG, GAMEMODE, POWER, NET, GPU_TUNE, COOLING, DRIVER];
}

/// Bit 0 of CheckAuthorization's flags: allow polkit to prompt the user.
const ALLOW_USER_INTERACTION: u32 = 1;

#[zbus::proxy(
    interface = "org.freedesktop.PolicyKit1.Authority",
    default_service = "org.freedesktop.PolicyKit1",
    default_path = "/org/freedesktop/PolicyKit1/Authority"
)]
trait Authority {
    /// Returns `(is_authorized, is_challenge, details)`.
    fn check_authorization(
        &self,
        subject: &(&str, HashMap<&str, Value<'_>>),
        action_id: &str,
        details: HashMap<&str, &str>,
        flags: u32,
        cancellation_id: &str,
    ) -> zbus::Result<(bool, bool, HashMap<String, String>)>;
}

/// Decides whether a caller may perform an action.
#[derive(Clone)]
pub enum Gate {
    /// System bus: ask polkit.
    Polkit(zbus::Connection),
    /// Session bus development run: allow everything, since nothing privileged
    /// is reachable anyway.
    Development,
}

impl Gate {
    /// Checks `action` for the caller identified by `hdr`, prompting through
    /// the caller's polkit agent if the policy asks for a password.
    ///
    /// Denial and "polkit is unreachable" both refuse. Failing open when the
    /// authority is down would turn a crashed polkit into root for everyone.
    pub async fn authorize(
        &self,
        hdr: &zbus::message::Header<'_>,
        action: &str,
    ) -> zbus::fdo::Result<()> {
        let conn = match self {
            Gate::Development => {
                tracing::debug!(action, "development mode: polkit check skipped");
                return Ok(());
            }
            Gate::Polkit(conn) => conn,
        };

        let sender = hdr
            .sender()
            .ok_or_else(|| zbus::fdo::Error::AccessDenied("message has no sender".into()))?
            .to_string();

        let authority = AuthorityProxy::new(conn).await.map_err(|e| {
            tracing::error!(error = %e, "polkit authority unreachable; refusing");
            zbus::fdo::Error::AccessDenied(
                "polkit is not running, so this change cannot be authorised".into(),
            )
        })?;

        // Identify the caller by bus name, which polkit resolves to a process
        // and session itself. Passing a PID instead would be racy: the caller
        // could exit and the PID be reused between our check and polkit's.
        let mut subject_details = HashMap::new();
        subject_details.insert("name", Value::from(sender.as_str()));
        let subject = ("system-bus-name", subject_details);

        let (authorized, _challenge, _details) = authority
            .check_authorization(&subject, action, HashMap::new(), ALLOW_USER_INTERACTION, "")
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, action, "polkit check failed; refusing");
                zbus::fdo::Error::AccessDenied(format!("authorisation failed: {e}"))
            })?;

        if authorized {
            tracing::info!(action, caller = %sender, "authorised");
            Ok(())
        } else {
            tracing::info!(action, caller = %sender, "denied by polkit");
            Err(zbus::fdo::Error::AccessDenied(format!(
                "not authorised to perform {action}"
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::action;

    const POLICY: &str =
        include_str!("../../../packaging/polkit/org.unixguardian.policy");

    #[test]
    fn every_action_the_daemon_checks_is_defined_in_the_shipped_policy() {
        for a in action::ALL {
            assert!(
                POLICY.contains(&format!("<action id=\"{a}\">")),
                "{a} is checked in code but missing from the polkit policy; \
                 the check would always deny"
            );
        }
    }

    #[test]
    fn the_policy_defines_nothing_the_daemon_does_not_check() {
        let defined = POLICY.matches("<action id=").count();
        assert_eq!(
            defined,
            action::ALL.len(),
            "the policy defines an action no method enforces"
        );
    }
}
