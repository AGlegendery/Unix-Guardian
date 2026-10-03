//! Games: what is installed, what is known about it, and what is running.
//!
//! - [`db`] is the bundled game database: default profiles and anti-cheat notes.
//! - [`steam`], [`lutris`], [`heroic`] and [`bottles`] read each launcher's own
//!   files to list installed games. None of them needs the launcher running.
//! - [`detect`] watches `/proc` and says when a game starts and stops.
//!
//! Everything here reads files only; nothing is launched, and nothing needs
//! privileges. It runs in the per-user agent.

pub mod bottles;
pub mod db;
pub mod detect;
pub mod heroic;
pub mod launch;
pub mod lutris;
pub mod proc;
pub mod steam;
pub mod vdf;

use serde::Serialize;
use std::path::{Path, PathBuf};

/// Which launcher a game belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Launcher {
    Steam,
    Lutris,
    Heroic,
    Bottles,
}

/// A game the user has installed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InstalledGame {
    /// Launcher-qualified and stable across scans, e.g. `steam:730`,
    /// `lutris:12`, `heroic:epic:Fortnite`, `bottles:RedDead:<uuid>`.
    pub id: String,
    pub name: String,
    pub launcher: Launcher,
    pub steam_appid: Option<u32>,
    /// Normalised executable basenames (see [`db::normalise_exe`]), used to
    /// recognise the game when it runs. Empty for Steam games, which are
    /// recognised by App ID instead.
    pub exe_names: Vec<String>,
    pub install_dir: Option<PathBuf>,
}

/// Every installed game from every launcher found under `home`.
///
/// A launcher that is not installed, or whose files do not parse, contributes
/// nothing rather than failing the whole scan.
pub fn scan_installed(home: &Path) -> Vec<InstalledGame> {
    let mut all = steam::scan(home);
    all.extend(lutris::scan(home));
    all.extend(heroic::scan(home));
    all.extend(bottles::scan(home));
    all
}
