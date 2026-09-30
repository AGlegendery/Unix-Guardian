//! Game profiles: what game mode changes while a game runs.
//!
//! Three built-in profiles cover most people. Any of them can be overridden,
//! and new ones added, by dropping a TOML file in
//! `~/.config/unix-guardian/profiles/<id>.toml`; which profile each game uses
//! lives in `~/.config/unix-guardian/games.toml`. Everything a profile does is
//! temporary: it applies when a game starts and is reverted when the last game
//! stops, including after a crash.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// One profile. Every field has a default, so a user file can set only what it
/// changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    /// Filled from the file name; not read from the file.
    #[serde(skip)]
    pub id: String,
    /// CPU frequency governor `performance` and energy preference `performance`.
    pub cpu_performance: bool,
    /// Hold CPU idle states off (via `/dev/cpu_dma_latency`) to stop clocks
    /// dropping between frames. Costs power; worth it for competitive play.
    pub cpu_no_idle_states: bool,
    /// Keep the game on the CPU's preferred cores: the V-Cache CCD on AMD X3D,
    /// the P-cores on Intel hybrid. Ignored on CPUs without such cores.
    pub cpu_preferred_cores: bool,
    /// Raise the game's scheduling priority (nice -10).
    pub cpu_priority: bool,
    /// Memory settings games benefit from: low swappiness, a large
    /// `vm.max_map_count`, and no proactive compaction.
    pub memory_tuning: bool,
    /// Pause background services that cause stutter (package updates, file
    /// indexers) for the duration of the game.
    pub pause_services: bool,
    /// Applications to pause (SIGSTOP) while playing, by executable name, e.g.
    /// `["discord", "steamwebhelper"]`. Resumed exactly as they were after.
    pub pause_apps: Vec<String>,
}

impl Default for Profile {
    /// The "balanced" behaviour, which is also what an unlisted field falls
    /// back to.
    fn default() -> Self {
        Profile {
            id: String::new(),
            cpu_performance: true,
            cpu_no_idle_states: false,
            cpu_preferred_cores: false,
            cpu_priority: true,
            memory_tuning: true,
            pause_services: true,
            pause_apps: Vec::new(),
        }
    }
}

/// The built-in profiles.
pub fn builtin() -> Vec<Profile> {
    vec![
        Profile {
            id: "competitive".into(),
            cpu_no_idle_states: true,
            ..Profile::default()
        },
        Profile {
            id: "balanced".into(),
            ..Profile::default()
        },
        Profile {
            id: "quiet".into(),
            cpu_performance: false,
            cpu_no_idle_states: false,
            memory_tuning: false,
            ..Profile::default()
        },
    ]
}

/// All profiles and the per-game choices, as loaded from disk.
#[derive(Debug, Clone)]
pub struct Profiles {
    profiles: Vec<Profile>,
    /// Game key (`steam:730`) → profile id.
    per_game: BTreeMap<String, String>,
}

#[derive(Deserialize, Serialize, Default)]
struct GamesFile {
    #[serde(default)]
    games: BTreeMap<String, String>,
}

impl Profiles {
    /// Loads built-ins, then user profiles from `config_dir/profiles/*.toml`
    /// (a file named like a built-in replaces it), then `config_dir/games.toml`.
    ///
    /// A file that does not parse is skipped and reported, never fatal: a typo
    /// in one profile must not stop game detection working.
    pub fn load(config_dir: &Path) -> (Self, Vec<String>) {
        let mut problems = Vec::new();
        let mut profiles = builtin();

        if let Ok(entries) = std::fs::read_dir(config_dir.join("profiles")) {
            let mut paths: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
            paths.sort();
            for path in paths.into_iter().filter(|p| p.extension().is_some_and(|e| e == "toml")) {
                let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(str::to_string) else {
                    continue;
                };
                match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|s| {
                    toml::from_str::<Profile>(&s).map_err(|e| e.to_string())
                }) {
                    Ok(mut p) => {
                        p.id = id.clone();
                        profiles.retain(|q| q.id != id);
                        profiles.push(p);
                    }
                    Err(e) => problems.push(format!("{}: {e}", path.display())),
                }
            }
        }

        let per_game = match std::fs::read_to_string(config_dir.join("games.toml")) {
            Ok(s) => match toml::from_str::<GamesFile>(&s) {
                Ok(f) => f.games,
                Err(e) => {
                    problems.push(format!("games.toml: {e}"));
                    BTreeMap::new()
                }
            },
            Err(_) => BTreeMap::new(),
        };

        (Profiles { profiles, per_game }, problems)
    }

    pub fn get(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.id == id)
    }

    /// The profile a game runs with: the user's choice for it, else the
    /// database's suggestion, else `balanced`. A choice naming a profile that
    /// no longer exists falls through rather than leaving the game untuned.
    pub fn for_game(&self, key: &str, suggested: &str) -> &Profile {
        self.per_game
            .get(key)
            .and_then(|id| self.get(id))
            .or_else(|| self.get(suggested))
            .or_else(|| self.get("balanced"))
            .expect("the balanced profile is built in")
    }

    pub fn all(&self) -> &[Profile] {
        &self.profiles
    }

    /// The user's per-game choices: game key → profile id.
    pub fn per_game(&self) -> &BTreeMap<String, String> {
        &self.per_game
    }
}

/// Whether `id` names one of the built-in profiles (possibly overridden by a
/// user file). Built-ins cannot be deleted, only changed.
pub fn is_builtin(id: &str) -> bool {
    builtin().iter().any(|p| p.id == id)
}

/// Records which profile a game runs with in `config_dir/games.toml`, or with
/// `None` removes the choice so the game falls back to the database's
/// suggestion. Every other entry is kept.
///
/// An existing file that does not parse is left alone and reported: rewriting
/// it would silently throw away the user's hand-made choices. The new file is
/// written beside the old one and renamed over it, so a crash mid-write cannot
/// leave a truncated file that resets every game to its default.
pub fn set_game_profile(config_dir: &Path, key: &str, profile: Option<&str>) -> Result<(), String> {
    let path = config_dir.join("games.toml");
    let mut file = match std::fs::read_to_string(&path) {
        Ok(s) => toml::from_str::<GamesFile>(&s)
            .map_err(|e| format!("{} does not parse, so it was not changed: {e}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => GamesFile::default(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    match profile {
        Some(id) => file.games.insert(key.to_string(), id.to_string()),
        None => file.games.remove(key),
    };
    let text = toml::to_string(&file).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(config_dir).map_err(|e| format!("{}: {e}", config_dir.display()))?;
    let tmp = config_dir.join(".games.toml.tmp");
    std::fs::write(&tmp, text)
        .and_then(|()| std::fs::rename(&tmp, &path))
        .map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn builtins_are_available_with_no_config_at_all() {
        let d = dir();
        let (p, problems) = Profiles::load(d.path());
        assert!(problems.is_empty());
        assert!(p.get("competitive").unwrap().cpu_no_idle_states);
        assert!(!p.get("quiet").unwrap().cpu_performance);
        assert_eq!(p.for_game("steam:730", "competitive").id, "competitive");
    }

    #[test]
    fn a_user_file_overrides_only_what_it_sets() {
        let d = dir();
        std::fs::create_dir_all(d.path().join("profiles")).unwrap();
        std::fs::write(
            d.path().join("profiles/balanced.toml"),
            "cpu_preferred_cores = true\npause_apps = [\"discord\"]\n",
        )
        .unwrap();
        let (p, _) = Profiles::load(d.path());
        let b = p.get("balanced").unwrap();
        assert!(b.cpu_preferred_cores);
        assert_eq!(b.pause_apps, vec!["discord"]);
        assert!(b.cpu_performance, "unset fields keep their defaults");
    }

    #[test]
    fn per_game_choice_wins_and_a_dangling_one_falls_through() {
        let d = dir();
        std::fs::write(
            d.path().join("games.toml"),
            "[games]\n\"steam:730\" = \"quiet\"\n\"steam:570\" = \"deleted-profile\"\n",
        )
        .unwrap();
        let (p, _) = Profiles::load(d.path());
        assert_eq!(p.for_game("steam:730", "competitive").id, "quiet");
        assert_eq!(p.for_game("steam:570", "competitive").id, "competitive");
        assert_eq!(p.for_game("exe:unknown", "nonsense").id, "balanced");
    }

    #[test]
    fn choosing_a_profile_for_a_game_keeps_the_other_choices() {
        let d = dir();
        std::fs::write(d.path().join("games.toml"), "[games]\n\"steam:570\" = \"quiet\"\n").unwrap();
        set_game_profile(d.path(), "steam:730", Some("competitive")).unwrap();
        let (p, problems) = Profiles::load(d.path());
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(p.for_game("steam:730", "balanced").id, "competitive");
        assert_eq!(p.for_game("steam:570", "balanced").id, "quiet");
    }

    #[test]
    fn clearing_a_choice_returns_the_game_to_its_suggested_profile() {
        let d = dir();
        set_game_profile(d.path(), "steam:730", Some("quiet")).unwrap();
        set_game_profile(d.path(), "steam:730", None).unwrap();
        let (p, _) = Profiles::load(d.path());
        assert_eq!(p.for_game("steam:730", "competitive").id, "competitive");
        assert!(p.per_game().is_empty());
    }

    #[test]
    fn the_first_choice_creates_the_config_directory() {
        let d = dir();
        let cfg = d.path().join("not/there/yet");
        set_game_profile(&cfg, "lutris:3", Some("quiet")).unwrap();
        assert_eq!(Profiles::load(&cfg).0.for_game("lutris:3", "balanced").id, "quiet");
    }

    #[test]
    fn a_hand_edited_file_that_does_not_parse_is_not_overwritten() {
        let d = dir();
        let broken = "[games]\n\"steam:730\" = quiet  # forgot the quotes\n";
        std::fs::write(d.path().join("games.toml"), broken).unwrap();
        let err = set_game_profile(d.path(), "steam:570", Some("quiet")).unwrap_err();
        assert!(err.contains("not changed"), "{err}");
        assert_eq!(std::fs::read_to_string(d.path().join("games.toml")).unwrap(), broken);
    }

    #[test]
    fn only_the_three_shipped_profiles_are_builtin() {
        assert!(is_builtin("competitive") && is_builtin("balanced") && is_builtin("quiet"));
        assert!(!is_builtin("mine"));
    }

    #[test]
    fn a_broken_file_is_reported_and_skipped_not_fatal() {
        let d = dir();
        std::fs::create_dir_all(d.path().join("profiles")).unwrap();
        std::fs::write(d.path().join("profiles/mine.toml"), "cpu_performance = \"yes please\"").unwrap();
        std::fs::write(d.path().join("profiles/typo.toml"), "cpu_performanse = true").unwrap();
        let (p, problems) = Profiles::load(d.path());
        assert_eq!(problems.len(), 2, "wrong type and unknown key both reported: {problems:?}");
        assert!(p.get("mine").is_none());
        assert!(p.get("balanced").is_some());
    }
}
