//! Installed Lutris games, from `pga.db` plus each game's YAML config.
//!
//! `pga.db` lists the games but its `executable` column is usually empty; the
//! executable lives in the per-game config under `game: exe:`. Both files move
//! between Lutris versions and between the native and Flatpak builds, so every
//! known location is tried.

use crate::{db::normalise_exe, InstalledGame, Launcher};
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};

/// `(pga.db, directories that may hold per-game configs)` for each install.
fn installs(home: &Path) -> Vec<(PathBuf, Vec<PathBuf>)> {
    let native = (
        home.join(".local/share/lutris/pga.db"),
        vec![
            home.join(".local/share/lutris/games"),
            home.join(".config/lutris/games"),
        ],
    );
    let flatpak = (
        home.join(".var/app/net.lutris.Lutris/data/lutris/pga.db"),
        vec![
            home.join(".var/app/net.lutris.Lutris/data/lutris/games"),
            home.join(".var/app/net.lutris.Lutris/config/lutris/games"),
        ],
    );
    [native, flatpak]
        .into_iter()
        .filter(|(db, _)| db.is_file())
        .collect()
}

pub fn scan(home: &Path) -> Vec<InstalledGame> {
    let mut out = Vec::new();
    for (db_path, config_dirs) in installs(home) {
        match read_db(&db_path, &config_dirs) {
            Ok(mut games) => out.append(&mut games),
            Err(e) => {
                tracing::warn!(db = %db_path.display(), error = %e, "could not read Lutris library")
            }
        }
    }
    out
}

fn read_db(db_path: &Path, config_dirs: &[PathBuf]) -> rusqlite::Result<Vec<InstalledGame>> {
    // Read-only, and patient: Lutris may be writing to it right now.
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(std::time::Duration::from_millis(500))?;
    let mut stmt =
        conn.prepare("SELECT id, name, directory, configpath FROM games WHERE installed = 1")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
        ))
    })?;

    let mut games = Vec::new();
    for row in rows.flatten() {
        let (id, name, directory, configpath) = row;
        let Some(name) = name.filter(|n| !n.is_empty()) else {
            continue;
        };
        let exe = configpath.as_deref().and_then(|cp| {
            config_dirs
                .iter()
                .find_map(|d| exe_from_config(&d.join(format!("{cp}.yml"))))
        });
        games.push(InstalledGame {
            id: format!("lutris:{id}"),
            name,
            launcher: Launcher::Lutris,
            steam_appid: None,
            exe_names: exe.iter().map(|e| normalise_exe(e)).collect(),
            install_dir: directory.filter(|d| !d.is_empty()).map(PathBuf::from),
        });
    }
    Ok(games)
}

/// Reads `game: exe:` from a per-game config.
fn exe_from_config(path: &Path) -> Option<String> {
    let src = std::fs::read_to_string(path).ok()?;
    let docs = yaml_rust2::YamlLoader::load_from_str(&src).ok()?;
    docs.first()?["game"]["exe"].as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    #[test]
    fn reads_installed_games_and_their_executables() {
        let fs = TestFs::new();
        let home = fs.root().join("home");
        std::fs::create_dir_all(home.join(".local/share/lutris/games")).unwrap();
        let conn = Connection::open(home.join(".local/share/lutris/pga.db")).unwrap();
        // The columns Lutris actually has (abridged to those plus a few extras,
        // so a query that assumed a fixed column order would break).
        conn.execute_batch(
            "CREATE TABLE games (id INTEGER PRIMARY KEY, name TEXT, sortname TEXT, slug TEXT,
                runner TEXT, executable TEXT, directory TEXT, installed INTEGER, configpath TEXT,
                playtime REAL, hidden INTEGER);
             INSERT INTO games VALUES (3, 'Witcher 3', NULL, 'witcher-3', 'wine', '', '/games/w3', 1, 'witcher-3-1712', 0, 0);
             INSERT INTO games VALUES (4, 'Uninstalled Thing', NULL, 'x', 'wine', '', '', 0, 'x-1', 0, 0);",
        )
        .unwrap();
        fs.write(
            "home/.local/share/lutris/games/witcher-3-1712.yml",
            "game:\n  exe: /games/w3/drive_c/GOG Games/The Witcher 3/bin/x64/witcher3.exe\n  prefix: /games/w3\nwine:\n  version: lutris-GE\n",
        );

        let games = scan(&home);
        assert_eq!(games.len(), 1, "uninstalled entries are not offered");
        assert_eq!(games[0].id, "lutris:3");
        assert_eq!(games[0].name, "Witcher 3");
        assert_eq!(games[0].exe_names, vec!["witcher3"]);
    }

    #[test]
    fn no_lutris_means_no_games_not_an_error() {
        let fs = TestFs::new();
        assert!(scan(fs.root()).is_empty());
    }
}
