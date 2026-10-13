//! Installed Heroic games: Epic (via legendary), GOG, and sideloaded apps.

use crate::{db::normalise_exe, InstalledGame, Launcher};
use serde_json::Value;
use std::path::{Path, PathBuf};

fn config_roots(home: &Path) -> Vec<PathBuf> {
    [
        ".config/heroic",
        ".var/app/com.heroicgameslauncher.hgl/config/heroic",
    ]
    .iter()
    .map(|r| home.join(r))
    .filter(|p| p.is_dir())
    .collect()
}

pub fn scan(home: &Path) -> Vec<InstalledGame> {
    let mut out = Vec::new();
    for root in config_roots(home) {
        out.extend(epic(&root));
        out.extend(gog(&root));
        out.extend(sideload(&root));
    }
    out
}

fn read_json(p: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

/// `legendaryConfig/legendary/installed.json`: an object keyed by app name.
fn epic(root: &Path) -> Vec<InstalledGame> {
    let Some(Value::Object(map)) =
        read_json(&root.join("legendaryConfig/legendary/installed.json"))
    else {
        return Vec::new();
    };
    map.iter()
        .filter_map(|(app, g)| {
            let install = g
                .get("install_path")
                .and_then(Value::as_str)
                .map(PathBuf::from);
            Some(InstalledGame {
                id: format!("heroic:epic:{app}"),
                name: g.get("title")?.as_str()?.to_string(),
                launcher: Launcher::Heroic,
                steam_appid: None,
                exe_names: g
                    .get("executable")
                    .and_then(Value::as_str)
                    .filter(|e| !e.is_empty())
                    .map(|e| vec![normalise_exe(e)])
                    .unwrap_or_default(),
                install_dir: install,
            })
        })
        .collect()
}

/// `gog_store/installed.json` lists installs without titles; titles come from
/// `gog_store/library.json`, falling back to the install folder's name.
fn gog(root: &Path) -> Vec<InstalledGame> {
    let Some(installed) = read_json(&root.join("gog_store/installed.json")) else {
        return Vec::new();
    };
    let library = read_json(&root.join("gog_store/library.json"));
    let title_for = |app: &str| -> Option<String> {
        library
            .as_ref()?
            .get("games")?
            .as_array()?
            .iter()
            .find_map(|g| {
                (g.get("app_name")?.as_str()? == app)
                    .then(|| g.get("title")?.as_str().map(str::to_string))?
            })
    };

    installed
        .get("installed")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter(|g| !g.get("is_dlc").and_then(Value::as_bool).unwrap_or(false))
                .filter_map(|g| {
                    let app = g.get("appName")?.as_str()?;
                    let dir = g
                        .get("install_path")
                        .and_then(Value::as_str)
                        .map(PathBuf::from);
                    let name = title_for(app)
                        .or_else(|| dir.as_ref()?.file_name()?.to_str().map(str::to_string))?;
                    Some(InstalledGame {
                        id: format!("heroic:gog:{app}"),
                        name,
                        launcher: Launcher::Heroic,
                        steam_appid: None,
                        exe_names: Vec::new(),
                        install_dir: dir,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `sideload_apps/library.json`: apps the user added by hand.
fn sideload(root: &Path) -> Vec<InstalledGame> {
    let Some(doc) = read_json(&root.join("sideload_apps/library.json")) else {
        return Vec::new();
    };
    doc.get("games")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter(|g| {
                    g.get("is_installed")
                        .and_then(Value::as_bool)
                        .unwrap_or(true)
                })
                .filter_map(|g| {
                    let app = g.get("app_name")?.as_str()?;
                    let exe = g.pointer("/install/executable").and_then(Value::as_str);
                    Some(InstalledGame {
                        id: format!("heroic:sideload:{app}"),
                        name: g.get("title")?.as_str()?.to_string(),
                        launcher: Launcher::Heroic,
                        steam_appid: None,
                        exe_names: exe.map(|e| vec![normalise_exe(e)]).unwrap_or_default(),
                        install_dir: g
                            .get("folder_name")
                            .and_then(Value::as_str)
                            .map(PathBuf::from),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    #[test]
    fn reads_epic_gog_and_sideloaded_games() {
        let fs = TestFs::new();
        fs.write(
            "home/.config/heroic/legendaryConfig/legendary/installed.json",
            r#"{"Fortnite": {"title": "Fortnite", "executable": "FortniteGame/Binaries/Win64/FortniteClient-Win64-Shipping.exe", "install_path": "/g/Fortnite", "platform": "Windows"}}"#,
        );
        fs.write(
            "home/.config/heroic/gog_store/installed.json",
            r#"{"installed": [
                {"appName": "1207664663", "install_path": "/g/Witcher 3", "platform": "windows", "is_dlc": false},
                {"appName": "999", "install_path": "/g/Some DLC", "platform": "windows", "is_dlc": true},
                {"appName": "1234", "install_path": "/g/Folder Named Game", "platform": "windows"}
            ]}"#,
        );
        fs.write(
            "home/.config/heroic/gog_store/library.json",
            r#"{"games": [{"app_name": "1207664663", "title": "The Witcher 3: Wild Hunt"}]}"#,
        );
        fs.write(
            "home/.config/heroic/sideload_apps/library.json",
            r#"{"games": [{"app_name": "abc", "title": "My Indie Game", "install": {"executable": "/g/indie/Indie.exe"}, "folder_name": "/g/indie", "is_installed": true}]}"#,
        );

        let mut games = scan(&fs.root().join("home"));
        games.sort_by(|a, b| a.id.cmp(&b.id));
        let ids: Vec<&str> = games.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "heroic:epic:Fortnite",
                "heroic:gog:1207664663",
                "heroic:gog:1234",
                "heroic:sideload:abc"
            ]
        );
        assert_eq!(games[0].exe_names, vec!["fortniteclient-win64-shipping"]);
        assert_eq!(games[1].name, "The Witcher 3: Wild Hunt");
        assert_eq!(
            games[2].name, "Folder Named Game",
            "title falls back to the install folder"
        );
        assert_eq!(games[3].exe_names, vec!["indie"]);
    }
}
