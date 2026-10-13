//! Programs added to Bottles, from each bottle's `bottle.yml`.

use crate::{db::normalise_exe, InstalledGame, Launcher};
use std::path::{Path, PathBuf};
use yaml_rust2::{Yaml, YamlLoader};

fn bottle_dirs(home: &Path) -> Vec<PathBuf> {
    [
        ".local/share/bottles/bottles",
        ".var/app/com.usebottles.bottles/data/bottles/bottles",
    ]
    .iter()
    .map(|r| home.join(r))
    .filter(|p| p.is_dir())
    .collect()
}

/// Every program the user added to any bottle.
///
/// Bottles of every environment are included — a program in an "Application"
/// bottle can still be a game — but the environment rides along in the id so a
/// caller can weigh it.
pub fn scan(home: &Path) -> Vec<InstalledGame> {
    let mut out = Vec::new();
    for dir in bottle_dirs(home) {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for bottle in entries.filter_map(|e| e.ok()).map(|e| e.path()) {
            for g in read_bottle(&bottle) {
                // Bottles happily stores the same executable twice when a user
                // adds it again; to the user that is one game.
                let dup = out.iter().any(|o: &InstalledGame| {
                    o.exe_names == g.exe_names && o.install_dir == g.install_dir
                });
                if !dup {
                    out.push(g);
                }
            }
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

fn read_bottle(bottle: &Path) -> Vec<InstalledGame> {
    let Ok(src) = std::fs::read_to_string(bottle.join("bottle.yml")) else {
        return Vec::new();
    };
    let Some(doc) = YamlLoader::load_from_str(&src)
        .ok()
        .and_then(|d| d.into_iter().next())
    else {
        tracing::warn!(bottle = %bottle.display(), "bottle.yml did not parse");
        return Vec::new();
    };
    // The directory, not the `Name:` field, identifies a bottle: Bottles
    // appends `__149`-style suffixes to keep directories unique while leaving
    // two bottles with the same display name.
    let bottle_dir = bottle
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let Yaml::Hash(programs) = &doc["External_Programs"] else {
        return Vec::new();
    };

    programs
        .iter()
        .filter_map(|(key, p)| {
            let uuid = key.as_str()?;
            let exe = p["executable"].as_str()?;
            let name = p["name"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| exe.trim_end_matches(".exe").to_string());
            Some(InstalledGame {
                id: format!("bottles:{bottle_dir}:{uuid}"),
                name,
                launcher: Launcher::Bottles,
                steam_appid: None,
                exe_names: vec![normalise_exe(exe)],
                install_dir: p["folder"].as_str().map(PathBuf::from),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    #[test]
    fn reads_external_programs_including_wrapped_yaml_lines() {
        let fs = TestFs::new();
        // Shape copied from a real bottle.yml: the line wrap Bottles writes for
        // long paths, and the same program added twice.
        fs.write(
            "home/.var/app/com.usebottles.bottles/data/bottles/bottles/RedDead__149/bottle.yml",
            "Arch: win64\nEnvironment: Gaming\nExternal_Programs:\n    7151074f-0340-480f-a6f9-641c779faeb0:\n        arguments: null\n        executable: Fears to Fathom - Ironbark Lookout.exe\n        folder: /home/u/.var/app/com.usebottles.bottles/data/bottles/bottles/RedDead__149/drive_c/Program\n            Files (x86)/Fears to Fathom Ironbark Lookout\n        name: Fears to Fathom - Ironbark Lookout\n    d9b6c790-247c-496d-ad1a-b24671144b17:\n        arguments: null\n        executable: Fears to Fathom - Ironbark Lookout.exe\n        folder: /home/u/.var/app/com.usebottles.bottles/data/bottles/bottles/RedDead__149/drive_c/Program\n            Files (x86)/Fears to Fathom Ironbark Lookout\n        name: Fears to Fathom - Ironbark Lookout\nName: RedDead\nRunner: soda-11.0-5\n",
        );
        fs.write(
            "home/.var/app/com.usebottles.bottles/data/bottles/bottles/OpenDesign/bottle.yml",
            "Arch: win64\nEnvironment: Application\nExternal_Programs: {}\nName: OpenDesign\n",
        );

        let games = scan(&fs.root().join("home"));
        assert_eq!(games.len(), 1);
        let g = &games[0];
        assert_eq!(g.launcher, Launcher::Bottles);
        assert_eq!(g.name, "Fears to Fathom - Ironbark Lookout");
        assert_eq!(
            g.id,
            "bottles:RedDead__149:7151074f-0340-480f-a6f9-641c779faeb0"
        );
        assert_eq!(g.exe_names, vec!["fears to fathom - ironbark lookout"]);
        assert!(g
            .install_dir
            .as_ref()
            .unwrap()
            .ends_with("Program Files (x86)/Fears to Fathom Ironbark Lookout"));
    }
}
