//! The agent's own settings, in `settings.json` in the user's configuration
//! directory, changed from the Settings page (window and terminal UI) or
//! `ug settings`.
//!
//! The main loop reads them on every use, so a change applies at once; the
//! scan interval also wakes the loop to rebuild its timer.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use ug_proto::AgentSettings;

pub struct Settings {
    file: PathBuf,
    current: Mutex<AgentSettings>,
    /// `--scan-interval` given on the command line wins over the file.
    scan_fixed: Option<u32>,
    changed: tokio::sync::watch::Sender<()>,
}

/// Checks a value before it is saved.
fn validate(s: &AgentSettings) -> Result<(), String> {
    if !(1..=10).contains(&s.scan_interval_s) {
        return Err(format!("scan interval must be 1 to 10 seconds, not {}", s.scan_interval_s));
    }
    Ok(())
}

impl Settings {
    /// Reads `settings.json`; a missing or unreadable file gives the
    /// defaults (and is not overwritten until the user saves).
    pub fn load(config_dir: &Path, scan_fixed: Option<u32>) -> Settings {
        let file = config_dir.join("settings.json");
        let current = std::fs::read_to_string(&file)
            .ok()
            .and_then(|t| match serde_json::from_str::<AgentSettings>(&t) {
                Ok(s) if validate(&s).is_ok() => Some(s),
                other => {
                    tracing::warn!(file = %file.display(), ?other, "ignoring unreadable settings");
                    None
                }
            })
            .unwrap_or_default();
        Settings { file, current: Mutex::new(current), scan_fixed, changed: tokio::sync::watch::channel(()).0 }
    }

    pub fn get(&self) -> AgentSettings {
        let mut s = self.current.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(i) = self.scan_fixed {
            s.scan_interval_s = i;
        }
        s
    }

    pub fn scan_fixed(&self) -> bool {
        self.scan_fixed.is_some()
    }

    pub fn set(&self, s: AgentSettings) -> Result<(), String> {
        validate(&s)?;
        let text = serde_json::to_string_pretty(&s).map_err(|e| e.to_string())?;
        ug_core::paths::write_atomic(&self.file, text.as_bytes()).map_err(|e| format!("{}: {e}", self.file.display()))?;
        *self.current.lock().unwrap_or_else(|e| e.into_inner()) = s;
        self.changed.send_replace(());
        Ok(())
    }

    /// Fires after every saved change.
    pub fn watch(&self) -> tokio::sync::watch::Receiver<()> {
        self.changed.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_are_saved_validated_and_read_back() {
        let d = tempfile::tempdir().unwrap();
        let s = Settings::load(d.path(), None);
        assert_eq!(s.get(), AgentSettings::default(), "no file: defaults");
        let w = s.watch();
        let quiet = AgentSettings { notifications: false, scan_interval_s: 5, close_window_on_game: true };
        s.set(quiet.clone()).unwrap();
        assert!(w.has_changed().unwrap(), "the loop is told");
        assert_eq!(Settings::load(d.path(), None).get(), quiet, "kept for the next start");
        let bad = AgentSettings { scan_interval_s: 0, ..quiet.clone() };
        assert!(s.set(bad).unwrap_err().contains("1 to 10"));
        assert_eq!(s.get(), quiet, "a refused value changes nothing");
    }

    #[test]
    fn a_broken_file_gives_the_defaults_and_the_command_line_interval_wins() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("settings.json"), "{ not json").unwrap();
        assert_eq!(Settings::load(d.path(), None).get(), AgentSettings::default());
        std::fs::write(d.path().join("settings.json"), r#"{"notifications":true,"scan_interval_s":9,"close_window_on_game":false}"#).unwrap();
        let s = Settings::load(d.path(), Some(1));
        assert_eq!(s.get().scan_interval_s, 1);
        assert!(s.scan_fixed());
    }
}
