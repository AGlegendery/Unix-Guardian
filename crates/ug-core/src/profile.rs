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
    /// Network settings for low latency while playing: Wi-Fi power saving
    /// off, TCP Fast Open and a smaller send queue, BBR congestion control.
    pub network_tuning: bool,
    /// Ask the system's power profile service (power-profiles-daemon) for
    /// its performance profile while playing: on laptops that also means the
    /// firmware's performance fan and power mode.
    pub power_profile: bool,
    /// Keep the screen on and the computer awake while playing (a hold on
    /// the desktop's screen saver, released when game mode ends).
    pub keep_screen_on: bool,
    /// Hold the desktop's notifications while playing (KDE Plasma; other
    /// desktops ignore it), Unix Guardian's own included.
    pub do_not_disturb: bool,
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
            network_tuning: true,
            power_profile: true,
            keep_screen_on: true,
            do_not_disturb: false,
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
            do_not_disturb: true,
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
            network_tuning: false,
            power_profile: false,
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
            for path in paths
                .into_iter()
                .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            {
                let Some(id) = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(str::to_string)
                else {
                    continue;
                };
                match std::fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|s| toml::from_str::<Profile>(&s).map_err(|e| e.to_string()))
                {
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

/// The on/off settings a profile has, by their file and wire names.
pub const OPTIONS: [&str; 10] = [
    "cpu_performance",
    "cpu_no_idle_states",
    "cpu_preferred_cores",
    "cpu_priority",
    "memory_tuning",
    "pause_services",
    "network_tuning",
    "power_profile",
    "keep_screen_on",
    "do_not_disturb",
];

/// A profile name a file can carry: letters, digits, `-` and `_`.
fn valid_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 40
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "{id:?} is not a valid profile name (letters, digits, - and _ only)"
        ));
    }
    Ok(())
}

/// Reads the user's file for a profile as a table (empty when there is
/// none), changes it, checks it still loads, and writes it atomically.
fn edit_file(
    config_dir: &Path,
    id: &str,
    change: impl FnOnce(&mut toml::Table),
) -> Result<(), String> {
    valid_id(id)?;
    let dir = config_dir.join("profiles");
    let path = dir.join(format!("{id}.toml"));
    let mut table = match std::fs::read_to_string(&path) {
        Ok(s) => s.parse::<toml::Table>().map_err(|e| {
            format!(
                "{} does not parse, so it was not changed: {e}",
                path.display()
            )
        })?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => toml::Table::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    change(&mut table);
    let text = toml::to_string(&table).map_err(|e| e.to_string())?;
    // Never write a file the loader would then skip as broken.
    toml::from_str::<Profile>(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let tmp = dir.join(format!(".{id}.toml.tmp"));
    std::fs::write(&tmp, text)
        .and_then(|()| std::fs::rename(&tmp, &path))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// The apps a profile pauses while playing, by executable name; empty
/// names and duplicates are dropped.
pub fn set_pause_apps(config_dir: &Path, id: &str, apps: &[String]) -> Result<(), String> {
    let mut list: Vec<String> = Vec::new();
    for a in apps.iter().map(|a| a.trim()).filter(|a| !a.is_empty()) {
        if a.contains('/') {
            return Err(format!("{a:?}: give the program's name, not a path"));
        }
        if !list.iter().any(|x| x == a) {
            list.push(a.to_string());
        }
    }
    edit_file(config_dir, id, |t| {
        t.insert(
            "pause_apps".into(),
            toml::Value::Array(list.into_iter().map(toml::Value::String).collect()),
        );
    })
}

/// A new profile of the user's own, starting as a copy of `from`.
pub fn create_profile(config_dir: &Path, id: &str, from: &Profile) -> Result<(), String> {
    valid_id(id)?;
    let (existing, _) = Profiles::load(config_dir);
    if existing.get(id).is_some() {
        return Err(format!("there is already a profile named {id:?}"));
    }
    let mut copy = from.clone();
    copy.id = id.to_string();
    let table = toml::Table::try_from(&copy).map_err(|e| e.to_string())?;
    edit_file(config_dir, id, |t| *t = table)
}

/// Deletes one of the user's own profiles; games that used it go back to
/// the profile the game database suggests. Built-in profiles stay.
pub fn delete_profile(config_dir: &Path, id: &str) -> Result<(), String> {
    valid_id(id)?;
    if is_builtin(id) {
        return Err(format!(
            "{id} is built in and cannot be deleted; its settings can be changed"
        ));
    }
    let path = config_dir.join("profiles").join(format!("{id}.toml"));
    std::fs::remove_file(&path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => format!("there is no profile of yours named {id:?}"),
        _ => format!("{}: {e}", path.display()),
    })?;
    let (p, _) = Profiles::load(config_dir);
    for (game, _) in p.per_game().iter().filter(|(_, pid)| *pid == id) {
        set_game_profile(config_dir, game, None)?;
    }
    Ok(())
}

/// Turns one setting of a profile on or off by writing it into the user's
/// `profiles/<id>.toml`; everything else already in that file (`pause_apps`,
/// other settings) is kept. For a built-in profile the file becomes an
/// override of just what the user changed.
///
/// Same safety rules as [`set_game_profile`]: a user file that does not parse
/// is reported and left alone, and the write is atomic.
pub fn set_profile_option(
    config_dir: &Path,
    id: &str,
    option: &str,
    value: bool,
) -> Result<(), String> {
    if !OPTIONS.contains(&option) {
        return Err(format!(
            "unknown setting {option:?}; settings are: {}",
            OPTIONS.join(", ")
        ));
    }
    edit_file(config_dir, id, |t| {
        t.insert(option.to_string(), toml::Value::Boolean(value));
    })
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
        Ok(s) => toml::from_str::<GamesFile>(&s).map_err(|e| {
            format!(
                "{} does not parse, so it was not changed: {e}",
                path.display()
            )
        })?,
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
        std::fs::write(
            d.path().join("games.toml"),
            "[games]\n\"steam:570\" = \"quiet\"\n",
        )
        .unwrap();
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
        assert_eq!(
            Profiles::load(&cfg).0.for_game("lutris:3", "balanced").id,
            "quiet"
        );
    }

    #[test]
    fn a_hand_edited_file_that_does_not_parse_is_not_overwritten() {
        let d = dir();
        let broken = "[games]\n\"steam:730\" = quiet  # forgot the quotes\n";
        std::fs::write(d.path().join("games.toml"), broken).unwrap();
        let err = set_game_profile(d.path(), "steam:570", Some("quiet")).unwrap_err();
        assert!(err.contains("not changed"), "{err}");
        assert_eq!(
            std::fs::read_to_string(d.path().join("games.toml")).unwrap(),
            broken
        );
    }

    #[test]
    fn changing_a_builtin_setting_overrides_only_that_setting() {
        let d = dir();
        set_profile_option(d.path(), "quiet", "cpu_priority", false).unwrap();
        let (p, problems) = Profiles::load(d.path());
        assert!(problems.is_empty(), "{problems:?}");
        let q = p.get("quiet").unwrap();
        assert!(!q.cpu_priority);
        assert!(q.pause_services, "untouched settings keep the default");
    }

    #[test]
    fn changing_a_setting_keeps_the_rest_of_the_users_file() {
        let d = dir();
        std::fs::create_dir_all(d.path().join("profiles")).unwrap();
        std::fs::write(
            d.path().join("profiles/mine.toml"),
            "pause_apps = [\"discord\"]\ncpu_performance = false\n",
        )
        .unwrap();
        set_profile_option(d.path(), "mine", "cpu_preferred_cores", true).unwrap();
        let (p, _) = Profiles::load(d.path());
        let m = p.get("mine").unwrap();
        assert_eq!(m.pause_apps, vec!["discord"]);
        assert!(!m.cpu_performance && m.cpu_preferred_cores);
    }

    #[test]
    fn an_unknown_setting_or_a_path_like_name_is_refused() {
        let d = dir();
        assert!(set_profile_option(d.path(), "quiet", "overclock", true)
            .unwrap_err()
            .contains("cpu_performance"));
        assert!(set_profile_option(d.path(), "../../etc/x", "cpu_priority", true).is_err());
        assert!(!d.path().join("profiles").exists());
    }

    #[test]
    fn a_profile_file_that_does_not_parse_is_not_overwritten() {
        let d = dir();
        std::fs::create_dir_all(d.path().join("profiles")).unwrap();
        let broken = "cpu_performance = maybe\n";
        std::fs::write(d.path().join("profiles/mine.toml"), broken).unwrap();
        assert!(set_profile_option(d.path(), "mine", "cpu_priority", true)
            .unwrap_err()
            .contains("not changed"));
        assert_eq!(
            std::fs::read_to_string(d.path().join("profiles/mine.toml")).unwrap(),
            broken
        );
    }

    #[test]
    fn a_profile_of_ones_own_is_made_from_another_and_deleted_with_its_games() {
        let d = dir();
        let (p, _) = Profiles::load(d.path());
        create_profile(d.path(), "night", p.get("quiet").unwrap()).unwrap();
        set_game_profile(d.path(), "steam:730", Some("night")).unwrap();
        set_pause_apps(
            d.path(),
            "night",
            &[
                "discord".into(),
                " discord ".into(),
                "".into(),
                "steamwebhelper".into(),
            ],
        )
        .unwrap();
        let (p, problems) = Profiles::load(d.path());
        assert!(problems.is_empty(), "{problems:?}");
        let n = p.get("night").unwrap();
        assert!(!n.cpu_performance, "a copy of quiet");
        assert_eq!(n.pause_apps, vec!["discord", "steamwebhelper"]);
        assert!(create_profile(d.path(), "night", n)
            .unwrap_err()
            .contains("already"));
        assert!(create_profile(d.path(), "quiet", n)
            .unwrap_err()
            .contains("already"));
        assert!(set_pause_apps(d.path(), "night", &["/usr/bin/x".into()]).is_err());
        assert!(delete_profile(d.path(), "balanced")
            .unwrap_err()
            .contains("built in"));
        delete_profile(d.path(), "night").unwrap();
        let (p, _) = Profiles::load(d.path());
        assert!(p.get("night").is_none());
        assert!(
            p.per_game().is_empty(),
            "its games went back to their suggestion"
        );
        assert!(delete_profile(d.path(), "night")
            .unwrap_err()
            .contains("no profile"));
    }

    #[test]
    fn the_new_settings_have_defaults_and_can_be_changed() {
        let d = dir();
        let (p, _) = Profiles::load(d.path());
        assert!(
            p.get("balanced").unwrap().keep_screen_on && !p.get("balanced").unwrap().do_not_disturb
        );
        assert!(p.get("competitive").unwrap().do_not_disturb);
        set_profile_option(d.path(), "balanced", "do_not_disturb", true).unwrap();
        assert!(
            Profiles::load(d.path())
                .0
                .get("balanced")
                .unwrap()
                .do_not_disturb
        );
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
        std::fs::write(
            d.path().join("profiles/mine.toml"),
            "cpu_performance = \"yes please\"",
        )
        .unwrap();
        std::fs::write(
            d.path().join("profiles/typo.toml"),
            "cpu_performanse = true",
        )
        .unwrap();
        let (p, problems) = Profiles::load(d.path());
        assert_eq!(
            problems.len(),
            2,
            "wrong type and unknown key both reported: {problems:?}"
        );
        assert!(p.get("mine").is_none());
        assert!(p.get("balanced").is_some());
    }
}
