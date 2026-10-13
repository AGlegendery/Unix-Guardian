//! Installed Steam games, from `libraryfolders.vdf` and `appmanifest_*.acf`.

use crate::vdf::Vdf;
use crate::{InstalledGame, Launcher};
use std::path::{Path, PathBuf};

/// Where Steam keeps its data, in the order a user is most likely to have it:
/// the native package, the `~/.steam` compatibility link, Flatpak, then Snap.
pub fn roots(home: &Path) -> Vec<PathBuf> {
    [
        ".local/share/Steam",
        ".steam/steam",
        ".var/app/com.valvesoftware.Steam/.local/share/Steam",
        "snap/steam/common/.local/share/Steam",
    ]
    .iter()
    .map(|rel| home.join(rel))
    .filter(|p| p.join("steamapps").is_dir())
    .collect()
}

/// Every fully installed game across every library folder of every Steam root.
///
/// `~/.steam/steam` is normally a symlink to `~/.local/share/Steam`, so roots
/// are canonicalised and de-duplicated; without that each game appears twice.
pub fn scan(home: &Path) -> Vec<InstalledGame> {
    let mut libraries: Vec<PathBuf> = Vec::new();
    for root in roots(home) {
        let root = std::fs::canonicalize(&root).unwrap_or(root);
        if !libraries.contains(&root) {
            libraries.push(root.clone());
        }
        for lib in library_folders(&root) {
            let lib = std::fs::canonicalize(&lib).unwrap_or(lib);
            if !libraries.contains(&lib) {
                libraries.push(lib);
            }
        }
    }

    let mut games: Vec<InstalledGame> = Vec::new();
    for lib in libraries {
        let steamapps = lib.join("steamapps");
        let Ok(entries) = std::fs::read_dir(&steamapps) else {
            continue;
        };
        for e in entries.filter_map(|e| e.ok()) {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if !(name.starts_with("appmanifest_") && name.ends_with(".acf")) {
                continue;
            }
            if let Some(g) = read_manifest(&e.path(), &steamapps) {
                if !games.iter().any(|x| x.id == g.id) {
                    games.push(g);
                }
            }
        }
    }
    games.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    games
}

/// Library folder paths listed in `steamapps/libraryfolders.vdf`.
fn library_folders(root: &Path) -> Vec<PathBuf> {
    let Ok(src) = std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) else {
        return Vec::new();
    };
    let Some(doc) = Vdf::parse(&src) else {
        tracing::warn!(root = %root.display(), "libraryfolders.vdf did not parse");
        return Vec::new();
    };
    doc.get("libraryfolders")
        .map(|lf| {
            lf.entries()
                .iter()
                .filter_map(|(_, folder)| folder.get("path")?.as_str().map(PathBuf::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Steam's own tools, which live in the library like games but are not ones.
fn is_tool(appid: u32, name: &str) -> bool {
    is_tool_appid(appid)
        || name.starts_with("Proton ")
        || name.starts_with("Steam Linux Runtime")
        || name.starts_with("Steamworks")
}

/// App IDs of Steam runtimes and compatibility tools. These run with a
/// `SteamAppId` of their own while a game starts, and must not be mistaken
/// for the game.
pub fn is_tool_appid(appid: u32) -> bool {
    const TOOLS: &[u32] = &[
        228980,  // Steamworks Common Redistributables
        1070560, // Steam Linux Runtime 1.0 (scout)
        1391110, // Steam Linux Runtime 2.0 (soldier)
        1628350, // Steam Linux Runtime 3.0 (sniper)
        1493710, // Proton Experimental
        2180100, // Proton Hotfix
        1161040, // Proton BattlEye Runtime
        1826330, // Proton EasyAntiCheat Runtime
    ];
    TOOLS.contains(&appid)
}

/// What Steam itself counted for one app, on any computer the account used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamPlaytime {
    pub appid: u32,
    pub minutes: u64,
    /// Unix seconds; 0 when Steam has no date.
    pub last_played_unix: u64,
}

/// Steam's playtime for the account last used on this computer.
///
/// Steam keeps it per account in `userdata/<account id>/config/localconfig.vdf`,
/// under `UserLocalConfigStore/Software/Valve/Steam/apps/<appid>`: `Playtime`
/// is the total in minutes, `LastPlayed` a Unix time in seconds (both written
/// by Steam only for apps that were played). With several accounts, the file
/// written most recently wins. The file is only read.
///
/// Steam's tools and non-Steam shortcuts are left out: shortcuts carry ids of
/// 2³¹ and up and are named only in the binary `shortcuts.vdf`.
pub fn playtime(home: &Path) -> Result<Vec<SteamPlaytime>, String> {
    let roots = roots(home);
    if roots.is_empty() {
        return Err("Steam is not installed for this user.".into());
    }
    let newest = roots
        .iter()
        .flat_map(|r| {
            std::fs::read_dir(r.join("userdata"))
                .into_iter()
                .flatten()
                .flatten()
        })
        .map(|account| account.path().join("config/localconfig.vdf"))
        .filter_map(|f| Some((std::fs::metadata(&f).ok()?.modified().ok()?, f)))
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, f)| f)
        .ok_or("Steam has not been signed in on this computer yet.")?;
    let src = std::fs::read_to_string(&newest).map_err(|e| format!("{}: {e}", newest.display()))?;
    let doc = Vdf::parse(&src).ok_or_else(|| format!("{} did not parse", newest.display()))?;
    Ok(apps_playtime(&doc))
}

fn apps_playtime(doc: &Vdf) -> Vec<SteamPlaytime> {
    let Some(apps) = doc.path(&["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"])
    else {
        return Vec::new();
    };
    let num = |app: &Vdf, key: &str| {
        app.get(key)
            .and_then(Vdf::as_str)
            .and_then(|v| v.trim().parse::<u64>().ok())
    };
    let mut v: Vec<SteamPlaytime> = apps
        .entries()
        .iter()
        .filter_map(|(id, app)| {
            let appid: u32 = id.parse().ok()?;
            if appid >= 0x8000_0000 || is_tool_appid(appid) {
                return None;
            }
            let minutes = num(app, "Playtime").filter(|&m| m > 0)?;
            Some(SteamPlaytime {
                appid,
                minutes,
                last_played_unix: num(app, "LastPlayed").unwrap_or(0),
            })
        })
        .collect();
    v.sort_by(|a, b| b.minutes.cmp(&a.minutes).then(a.appid.cmp(&b.appid)));
    v
}

fn read_manifest(path: &Path, steamapps: &Path) -> Option<InstalledGame> {
    let doc = Vdf::parse(&std::fs::read_to_string(path).ok()?)?;
    let app = doc.get("AppState")?;
    let appid: u32 = app.get("appid")?.as_str()?.parse().ok()?;
    let name = app.get("name")?.as_str()?.to_string();

    // StateFlags is a bitfield; 4 is "fully installed". A game mid-download or
    // mid-update has other bits and is not something the user can launch.
    let flags: u32 = app.get("StateFlags").and_then(Vdf::as_str)?.parse().ok()?;
    if flags & 4 == 0 || is_tool(appid, &name) {
        return None;
    }

    let install_dir = app
        .get("installdir")
        .and_then(Vdf::as_str)
        .map(|d| steamapps.join("common").join(d));

    Some(InstalledGame {
        id: format!("steam:{appid}"),
        name,
        launcher: Launcher::Steam,
        steam_appid: Some(appid),
        exe_names: Vec::new(),
        install_dir,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    fn manifest(appid: u32, name: &str, flags: u32) -> String {
        format!(
            "\"AppState\"\n{{\n\t\"appid\"\t\t\"{appid}\"\n\t\"name\"\t\t\"{name}\"\n\t\"StateFlags\"\t\t\"{flags}\"\n\t\"installdir\"\t\t\"{name}\"\n}}\n"
        )
    }

    #[test]
    fn finds_games_across_two_library_folders_and_skips_tools() {
        let fs = TestFs::new();
        let home = fs.root().join("home");
        let native = home.join(".local/share/Steam");
        let extra = fs.root().join("mnt/games/SteamLibrary");
        fs.write(
            "home/.local/share/Steam/steamapps/libraryfolders.vdf",
            &format!(
                "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}",
                native.display(),
                extra.display()
            ),
        );
        fs.write(
            "home/.local/share/Steam/steamapps/appmanifest_730.acf",
            &manifest(730, "Counter-Strike 2", 4),
        );
        fs.write(
            "home/.local/share/Steam/steamapps/appmanifest_1628350.acf",
            &manifest(1628350, "Steam Linux Runtime 3.0 (sniper)", 4),
        );
        fs.write(
            "home/.local/share/Steam/steamapps/appmanifest_1493710.acf",
            &manifest(1493710, "Proton Experimental", 4),
        );
        fs.write(
            "mnt/games/SteamLibrary/steamapps/appmanifest_1091500.acf",
            &manifest(1091500, "Cyberpunk 2077", 4),
        );

        let games = scan(&home);
        let names: Vec<&str> = games.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, vec!["Counter-Strike 2", "Cyberpunk 2077"]);
        assert_eq!(games[1].id, "steam:1091500");
        assert!(games[1]
            .install_dir
            .as_ref()
            .unwrap()
            .ends_with("steamapps/common/Cyberpunk 2077"));
    }

    #[test]
    fn a_game_still_downloading_is_not_offered() {
        let fs = TestFs::new();
        fs.write(
            "home/.local/share/Steam/steamapps/appmanifest_570.acf",
            &manifest(570, "Dota 2", 1026),
        );
        assert!(scan(&fs.root().join("home")).is_empty());
    }

    #[test]
    fn the_steam_symlink_does_not_duplicate_games() {
        let fs = TestFs::new();
        fs.write(
            "home/.local/share/Steam/steamapps/appmanifest_730.acf",
            &manifest(730, "Counter-Strike 2", 4),
        );
        std::fs::create_dir_all(fs.root().join("home/.steam")).unwrap();
        std::os::unix::fs::symlink(
            fs.root().join("home/.local/share/Steam"),
            fs.root().join("home/.steam/steam"),
        )
        .unwrap();
        assert_eq!(scan(&fs.root().join("home")).len(), 1);
    }

    /// The layout of a real `localconfig.vdf` (Flatpak Steam, 2026-10): the
    /// apps without play only have `cloud`/`BadgeData` blocks. `Playtime` and
    /// `LastPlayed` are added as Steam writes them for played apps.
    const LOCALCONFIG: &str = r#""UserLocalConfigStore"
{
	"friends"
	{
		"672194005"
		{
			"NameHistory" { "0" "someone" }
		}
	}
	"Software"
	{
		"Valve"
		{
			"Steam"
			{
				"apps"
				{
					"7"
					{
						"cloud"
						{
							"last_sync_state"		"synchronized"
						}
					}
					"730"
					{
						"LastPlayed"		"1790000000"
						"Playtime2wks"		"95"
						"Playtime"		"5521"
						"cloud" { "last_sync_state" "synchronized" }
					}
					"413150"
					{
						"LastPlayed"		"1780000000"
						"Playtime"		"610"
					}
					"1628350"
					{
						"Playtime"		"12"
					}
					"3169765128"
					{
						"BadgeData"		"020000000800"
						"Playtime"		"30"
					}
				}
				"LastPlayedTimesSyncTime"		"10"
				"PlayerLevel"		"0"
			}
		}
	}
}
"#;

    #[test]
    fn steam_playtime_is_read_for_played_games_only() {
        let fs = TestFs::new();
        fs.write(
            "home/.local/share/Steam/steamapps/libraryfolders.vdf",
            "\"libraryfolders\" {}",
        );
        fs.write(
            "home/.local/share/Steam/userdata/672194005/config/localconfig.vdf",
            LOCALCONFIG,
        );
        let p = playtime(&fs.root().join("home")).unwrap();
        assert_eq!(
            p,
            vec![
                SteamPlaytime {
                    appid: 730,
                    minutes: 5521,
                    last_played_unix: 1_790_000_000
                },
                SteamPlaytime {
                    appid: 413150,
                    minutes: 610,
                    last_played_unix: 1_780_000_000
                },
            ],
            "no unplayed apps, no runtimes, no non-Steam shortcuts"
        );
    }

    #[test]
    fn no_steam_and_no_account_are_said_plainly() {
        let fs = TestFs::new();
        assert!(playtime(&fs.root().join("home"))
            .unwrap_err()
            .contains("not installed"));
        fs.write(
            "home/.local/share/Steam/steamapps/libraryfolders.vdf",
            "\"libraryfolders\" {}",
        );
        assert!(playtime(&fs.root().join("home"))
            .unwrap_err()
            .contains("signed in"));
    }

    #[test]
    fn flatpak_steam_is_found() {
        let fs = TestFs::new();
        fs.write(
            "home/.var/app/com.valvesoftware.Steam/.local/share/Steam/steamapps/appmanifest_413150.acf",
            &manifest(413150, "Stardew Valley", 4),
        );
        assert_eq!(scan(&fs.root().join("home"))[0].name, "Stardew Valley");
    }
}
