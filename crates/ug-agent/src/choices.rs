//! What the user decided about programs, kept across sessions in
//! `playtime.json` in the configuration directory: which programs are
//! tracked as apps, and which executables are not games.
//!
//! "Not a game" used to live only in the detector's memory, so a dismissed
//! program was taken for a game again after the next login.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use ug_games::db::normalise_exe;
use ug_proto::TrackedApp;

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Choices {
    #[serde(default)]
    pub apps: Vec<TrackedApp>,
    /// Normalised executable names.
    #[serde(default)]
    pub not_games: Vec<String>,
}

fn file(dir: &Path) -> PathBuf {
    dir.join("playtime.json")
}

impl Choices {
    /// The saved choices; none when the file is missing. A file that does not
    /// parse is left alone (and logged) rather than overwritten on the next
    /// save, which would lose the user's choices.
    pub fn load(dir: &Path) -> Result<Choices, String> {
        match std::fs::read_to_string(file(dir)) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file(dir).display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Choices::default()),
            Err(e) => Err(format!("{}: {e}", file(dir).display())),
        }
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        ug_core::paths::write_atomic(&file(dir), text.as_bytes()).map_err(|e| format!("{}: {e}", file(dir).display()))
    }

    /// Adds (or renames) a tracked app. Returns its normalised executable.
    pub fn track(&mut self, exe: &str, name: &str) -> Result<String, String> {
        let exe = normalise_exe(exe.trim());
        if exe.is_empty() {
            return Err("give a program's executable name, e.g. firefox or Filmora.exe".into());
        }
        let name = if name.trim().is_empty() { exe.clone() } else { name.trim().to_string() };
        match self.apps.iter_mut().find(|a| a.exe == exe) {
            Some(a) => a.name = name,
            None => self.apps.push(TrackedApp { exe: exe.clone(), name }),
        }
        Ok(exe)
    }

    /// Returns whether it was tracked.
    pub fn untrack(&mut self, exe: &str) -> bool {
        let exe = normalise_exe(exe.trim());
        let before = self.apps.len();
        self.apps.retain(|a| a.exe != exe);
        self.apps.len() != before
    }

    /// Returns whether this is new.
    pub fn not_a_game(&mut self, exe: &str) -> bool {
        let exe = normalise_exe(exe.trim());
        if exe.is_empty() || self.not_games.contains(&exe) {
            return false;
        }
        self.not_games.push(exe);
        true
    }

    pub fn tracked_pairs(&self) -> Vec<(String, String)> {
        self.apps.iter().map(|a| (a.exe.clone(), a.name.clone())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_survive_a_restart_and_names_are_normalised() {
        let d = tempfile::tempdir().unwrap();
        let mut c = Choices::default();
        assert_eq!(c.track("Wondershare Filmora.exe", "Filmora").unwrap(), "wondershare filmora");
        c.track("firefox-esr", "").unwrap();
        assert!(c.not_a_game("Render.EXE"));
        assert!(!c.not_a_game("render"), "already there");
        c.save(d.path()).unwrap();

        let back = Choices::load(d.path()).unwrap();
        assert_eq!(back, c);
        assert_eq!(back.apps[1], TrackedApp { exe: "firefox-esr".into(), name: "firefox-esr".into() });
        assert_eq!(back.not_games, vec!["render"]);
    }

    #[test]
    fn a_broken_file_is_reported_not_replaced() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("playtime.json"), "{ not json").unwrap();
        assert!(Choices::load(d.path()).is_err());
        assert!(Choices::load(&d.path().join("missing")).unwrap().apps.is_empty());
    }

    #[test]
    fn untracking_and_empty_names() {
        let mut c = Choices::default();
        assert!(c.track("  ", "x").is_err());
        c.track("dolphin", "Dolphin").unwrap();
        c.track("dolphin", "Files").unwrap();
        assert_eq!(c.apps.len(), 1, "tracking again renames");
        assert_eq!(c.apps[0].name, "Files");
        assert!(c.untrack("Dolphin"));
        assert!(!c.untrack("dolphin"));
    }
}
