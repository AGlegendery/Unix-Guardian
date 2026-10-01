//! The system's power profile (power-profiles-daemon), held at
//! "performance" while a game that wants it runs.
//!
//! A hold, not a switch: power-profiles-daemon restores the user's own choice
//! when the hold is released, and releases it by itself if the agent leaves
//! the bus (crash, kill), so nothing is left behind.

#[zbus::proxy(
    interface = "org.freedesktop.UPower.PowerProfiles",
    default_service = "org.freedesktop.UPower.PowerProfiles",
    default_path = "/org/freedesktop/UPower/PowerProfiles"
)]
pub trait PowerProfiles {
    fn hold_profile(&self, profile: &str, reason: &str, application_id: &str) -> zbus::Result<u32>;
    fn release_profile(&self, cookie: u32) -> zbus::Result<()>;
    #[zbus(property)]
    fn active_profile(&self) -> zbus::Result<String>;
}

/// Whether to hold, given the current hold and whether a game wants it.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Hold,
    Release(u32),
    Nothing,
}

pub fn step(held: Option<u32>, wanted: bool) -> Step {
    match (held, wanted) {
        (None, true) => Step::Hold,
        (Some(c), false) => Step::Release(c),
        _ => Step::Nothing,
    }
}

pub struct Holder {
    pub cookie: Option<u32>,
    /// Do not retry a refused hold every tick.
    pub refused: bool,
}

impl Holder {
    pub fn new() -> Self {
        Holder { cookie: None, refused: false }
    }

    pub async fn sync(&mut self, system: Option<&zbus::Connection>, wanted: bool, game: &str) {
        if !wanted {
            self.refused = false;
        }
        let Some(conn) = system else { return };
        match step(self.cookie, wanted && !self.refused) {
            Step::Nothing => {}
            Step::Hold => {
                let Ok(p) = PowerProfilesProxy::new(conn).await else { return };
                match p.hold_profile("performance", &format!("Playing {game}"), "Unix Guardian").await {
                    Ok(c) => {
                        tracing::info!(game, "performance power profile held");
                        self.cookie = Some(c);
                    }
                    Err(e) => {
                        tracing::info!(error = %e, "power-profiles-daemon refused or is absent; profile left as it is");
                        self.refused = true;
                    }
                }
            }
            Step::Release(c) => {
                if let Ok(p) = PowerProfilesProxy::new(conn).await {
                    let _ = p.release_profile(c).await;
                }
                tracing::info!("performance power profile released");
                self.cookie = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_once_and_releases_when_the_game_ends() {
        assert_eq!(step(None, true), Step::Hold);
        assert_eq!(step(Some(7), true), Step::Nothing);
        assert_eq!(step(Some(7), false), Step::Release(7));
        assert_eq!(step(None, false), Step::Nothing);
    }
}
