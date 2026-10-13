//! The bundled game database, `data/games.json`.
//!
//! What it adds on top of what launchers already say: the profile a game
//! should start with, and whether its anti-cheat refuses Linux — so the UI can
//! tell a user why a game will not start instead of leaving them to find out.
//! Steam App IDs in it are checked against the Steam store API by
//! `scripts/verify-games-db.py`.

use serde::Deserialize;

/// The copy compiled into the binary. A newer copy downloaded at runtime (P10)
/// overrides it; this one guarantees there is always something to match on.
const BUNDLED: &str = include_str!("../../../data/games.json");

/// One game the project knows about.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct KnownGame {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub steam_appid: Option<u32>,
    /// Executable basenames without `.exe`, compared case-insensitively.
    #[serde(default)]
    pub exe: Vec<String>,
    /// Substrings of the command line that identify the game when the
    /// executable is generic — Minecraft is `java`, and so is half the world.
    #[serde(default)]
    pub cmdline: Vec<String>,
    /// Profile applied when the user has not chosen one: `competitive`,
    /// `balanced` or `quiet`.
    #[serde(default = "default_profile")]
    pub profile: String,
    /// `"blocked"` when anti-cheat refuses to run under Linux.
    #[serde(default)]
    pub linux: Option<String>,
    /// Why, in a sentence the UI can show.
    #[serde(default)]
    pub linux_note: Option<String>,
}

fn default_profile() -> String {
    "balanced".into()
}

impl KnownGame {
    pub fn blocked_on_linux(&self) -> bool {
        self.linux.as_deref() == Some("blocked")
    }
}

#[derive(Deserialize)]
struct Document {
    version: u32,
    games: Vec<KnownGame>,
}

/// The game database, indexed for the two lookups detection makes.
#[derive(Debug, Clone, Default)]
pub struct GameDb {
    games: Vec<KnownGame>,
}

impl GameDb {
    /// The database compiled into this build.
    pub fn bundled() -> Self {
        Self::parse(BUNDLED).expect("bundled games.json is validated by tests")
    }

    /// Parses a `games.json` document. Unknown versions are refused rather
    /// than half-understood.
    pub fn parse(json: &str) -> Result<Self, String> {
        let doc: Document = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if doc.version != 1 {
            return Err(format!("unsupported games.json version {}", doc.version));
        }
        Ok(GameDb { games: doc.games })
    }

    pub fn by_steam_appid(&self, appid: u32) -> Option<&KnownGame> {
        self.games.iter().find(|g| g.steam_appid == Some(appid))
    }

    /// Matches an executable basename (with or without `.exe`, any case).
    pub fn by_exe(&self, basename: &str) -> Option<&KnownGame> {
        let want = normalise_exe(basename);
        self.games
            .iter()
            .find(|g| g.exe.iter().any(|e| normalise_exe(e) == want))
    }

    /// Matches on a command-line substring, for games run by a generic binary.
    pub fn by_cmdline(&self, args: &[String]) -> Option<&KnownGame> {
        self.games.iter().find(|g| {
            !g.cmdline.is_empty()
                && g.cmdline
                    .iter()
                    .any(|needle| args.iter().any(|a| a.contains(needle.as_str())))
        })
    }

    pub fn by_id(&self, id: &str) -> Option<&KnownGame> {
        self.games.iter().find(|g| g.id == id)
    }

    pub fn len(&self) -> usize {
        self.games.len()
    }

    pub fn is_empty(&self) -> bool {
        self.games.is_empty()
    }
}

/// Lowercases a basename and strips a trailing `.exe`, so `Cyberpunk2077.exe`,
/// `cyberpunk2077` and a Windows-style path's tail all compare equal.
pub fn normalise_exe(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let lower = base.to_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_database_parses_and_is_consistent() {
        let db = GameDb::bundled();
        assert!(db.len() >= 60);

        let mut ids = std::collections::HashSet::new();
        let mut appids = std::collections::HashSet::new();
        for g in &db.games {
            assert!(ids.insert(&g.id), "duplicate id {}", g.id);
            if let Some(a) = g.steam_appid {
                assert!(appids.insert(a), "duplicate steam_appid {a}");
            }
            assert!(
                matches!(g.profile.as_str(), "competitive" | "balanced" | "quiet"),
                "{} has unknown profile {}",
                g.id,
                g.profile
            );
            if g.blocked_on_linux() {
                assert!(
                    g.linux_note.is_some(),
                    "{} is blocked but says not why",
                    g.id
                );
            }
        }
    }

    #[test]
    fn lookups_by_appid_exe_and_cmdline() {
        let db = GameDb::bundled();
        assert_eq!(db.by_steam_appid(730).unwrap().name, "Counter-Strike 2");
        assert_eq!(db.by_exe("Cyberpunk2077.exe").unwrap().id, "cyberpunk2077");
        assert_eq!(
            db.by_exe(r"C:\Games\Cyberpunk 2077\bin\x64\CYBERPUNK2077.EXE")
                .unwrap()
                .id,
            "cyberpunk2077"
        );
        let args = vec![
            "java".into(),
            "-Xmx4G".into(),
            "net.minecraft.client.main.Main".into(),
        ];
        assert_eq!(db.by_cmdline(&args).unwrap().id, "minecraft-java");
        assert!(db
            .by_cmdline(&["java".into(), "-jar".into(), "server.jar".into()])
            .is_none());
    }

    #[test]
    fn riot_games_are_flagged_with_a_reason() {
        let db = GameDb::bundled();
        let v = db.by_id("valorant").unwrap();
        assert!(v.blocked_on_linux());
        assert!(v.linux_note.as_deref().unwrap().contains("Vanguard"));
    }

    #[test]
    fn a_future_format_is_refused() {
        assert!(GameDb::parse(r#"{"version": 2, "games": []}"#).is_err());
    }
}
