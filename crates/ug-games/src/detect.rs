//! Deciding which running processes are games.
//!
//! [`Detector::scan`] is called every couple of seconds with the process table
//! and turns it into [`Event::Started`] / [`Event::Stopped`]. Evidence, from
//! strongest to weakest:
//!
//! 1. **`SteamAppId` in the environment.** Steam sets it on everything it
//!    launches and the whole game process tree inherits it. Certain.
//! 2. **GameMode registration.** Steam, Lutris, Heroic and gamescope call
//!    `RegisterGame` on the GameMode D-Bus interface the agent provides. Fed in
//!    through [`Detector::register`].
//! 3. **Installed library.** The executable matches a game a launcher lists.
//! 4. **Game database.** The executable or command line matches `games.json`.
//! 5. **Heuristic.** A Windows program drawing through DirectX/Vulkan/OpenGL,
//!    or a native program drawing through Vulkan, that also keeps a core busy
//!    for five seconds. Browsers, editors and the like are excluded by name,
//!    and the user can dismiss a wrong guess with [`Detector::ignore_exe`].
//!
//! Classification is cached per process — `(pid, start time)` — so steady
//! state costs a directory listing plus a `stat` read for each process already
//! known to be a game or candidate.

use crate::db::{normalise_exe, GameDb};
use crate::{proc, steam, InstalledGame};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::{Duration, Instant};
use ug_hw::SysRoot;

/// How long a game may have no processes before it counts as stopped.
/// Launchers commonly exit and exec the real game, which would otherwise read
/// as a stop immediately followed by a start.
pub const STOP_DEBOUNCE: Duration = Duration::from_secs(3);
/// How long a heuristic candidate must stay busy before it is called a game.
pub const HEURISTIC_WINDOW: Duration = Duration::from_secs(5);
/// Minimum average CPU over the window, as a percentage of one core. Games
/// run far above this; a video player idling in a corner does not.
pub const HEURISTIC_MIN_CPU_PCT: f64 = 25.0;

/// Why a process was judged to be a game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    SteamAppId,
    GameMode,
    Library,
    Database,
    Heuristic,
}

/// A game that is running now.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunningGame {
    /// Stable identity: `steam:730`, `lutris:3`, `db:minecraft-java`,
    /// `exe:somegame`. Playtime and per-game profiles are keyed on this.
    pub key: String,
    pub name: String,
    pub evidence: Evidence,
    /// Profile to apply unless the user chose another for this game.
    pub profile: String,
    /// Every process currently belonging to the game, ascending. Tuning such
    /// as CPU pinning has to act on all of them, not just the first.
    pub pids: Vec<i32>,
    pub steam_appid: Option<u32>,
    /// Set when the database knows this game's anti-cheat refuses Linux.
    pub blocked_note: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Started(RunningGame),
    Stopped { game: RunningGame, played: Duration },
}

/// Processes that are never games however they behave. Matched on `comm`,
/// which the kernel truncates to 15 bytes, so entries respect that limit.
const NEVER_NATIVE: &[&str] = &[
    // launchers and their helpers
    "steam", "steamwebhelper", "fossilize_repla", "reaper", "pressure-vessel", "srt-bwrap",
    "lutris", "heroic", "bottles", "gamescope", "gamemoded", "wineserver",
    // browsers and Electron
    "firefox", "firefox-bin", "firefox-esr", "chrome", "chromium", "brave", "opera", "vivaldi-bin",
    "electron", "code", "codium", "cursor", "discord", "Discord", "vesktop", "slack", "spotify",
    "zoom", "teams", "obsidian", "signal-desktop", "telegram-deskto", "Telegram",
    // desktop and display
    "Xwayland", "Xorg", "gnome-shell", "kwin_wayland", "kwin_x11", "plasmashell", "mutter",
    "sway", "Hyprland", "niri", "cosmic-comp", "weston", "xdg-desktop-por", "pipewire", "wireplumber",
    // terminals
    "kitty", "alacritty", "wezterm-gui", "ghostty", "foot", "konsole", "gnome-terminal-",
    // creative and media tools that legitimately use the GPU hard
    "blender", "obs", "kdenlive", "davinci", "resolve", "gimp", "krita", "inkscape", "darktable",
    "mpv", "vlc", "celluloid", "totem", "godot", "unityhub", "Unity",
];

/// Windows executables that are part of Wine or of a store client, never the
/// game itself. Compared after [`normalise_exe`].
const NEVER_WINDOWS: &[&str] = &[
    "explorer", "services", "winedevice", "plugplay", "svchost", "rpcss", "conhost", "start",
    "rundll32", "wineboot", "winemenubuilder", "tabtip", "cmd", "steam", "iexplore", "msiexec",
    "crashpad_handler", "unitycrashhandler64", "unitycrashhandler32", "crashreportclient",
    "epicgameslauncher", "epicwebhelper", "galaxyclient", "galaxyclient helper", "battle.net",
    "agent", "riotclientservices", "riotclientux", "upc", "uplaywebcore", "eadesktop",
    "eabackgroundservice", "origin", "qtwebengineprocess", "cefsharp.browsersubprocess",
];

/// A helper process of Chromium, Electron or CEF — the renderer, GPU and
/// utility processes every such app spawns. They map Vulkan and can run hot, so
/// the heuristic would take them for games; no name list can keep up with every
/// Electron app, but all of them mark their helpers with `--type=`. This
/// covers store clients' embedded browsers under Wine too.
fn is_chromium_child(p: &proc::Proc) -> bool {
    // `args()` also splits a rewritten process title, which is the form these
    // processes actually take: one string, no NUL separators.
    p.args().skip(1).any(|a| a.starts_with("--type="))
}

fn is_installer(basename: &str) -> bool {
    let b = normalise_exe(basename);
    b.contains("setup") || b.contains("install") || b.starts_with("unins") || b.contains("redist")
}

/// Who a classified process belongs to.
#[derive(Debug, Clone)]
struct Identity {
    key: String,
    name: String,
    evidence: Evidence,
    profile: String,
    steam_appid: Option<u32>,
    blocked_note: Option<String>,
}

#[derive(Debug)]
enum Class {
    Ignored,
    Game(String),
    Candidate { since: Instant, cpu_at_since: u64, basename: String },
}

#[derive(Debug)]
struct Seen {
    start_ticks: u64,
    class: Class,
}

#[derive(Debug)]
struct Tracked {
    ident: Identity,
    pids: BTreeSet<i32>,
    started_at: Instant,
    empty_since: Option<Instant>,
    announced: bool,
}

impl Tracked {
    fn snapshot(&self) -> RunningGame {
        RunningGame {
            key: self.ident.key.clone(),
            name: self.ident.name.clone(),
            evidence: self.ident.evidence,
            profile: self.ident.profile.clone(),
            pids: self.pids.iter().copied().collect(),
            steam_appid: self.ident.steam_appid,
            blocked_note: self.ident.blocked_note.clone(),
        }
    }
}

/// Watches the process table for games. One per user session.
pub struct Detector {
    db: GameDb,
    installed: Vec<InstalledGame>,
    uid: u32,
    ticks_per_sec: u64,
    seen: HashMap<i32, Seen>,
    games: HashMap<String, Tracked>,
    ignored: HashSet<String>,
    registered: HashMap<i32, Evidence>,
}

impl Detector {
    /// `uid` is the session owner; other users' processes are never games of
    /// theirs, and the daemon must not be asked to tune them.
    pub fn new(db: GameDb, installed: Vec<InstalledGame>, uid: u32) -> Self {
        Detector {
            db,
            installed,
            uid,
            ticks_per_sec: proc::clock_ticks_per_second(),
            seen: HashMap::new(),
            games: HashMap::new(),
            ignored: HashSet::new(),
            registered: HashMap::new(),
        }
    }

    /// Replaces the installed-games list after a rescan. Already-classified
    /// processes keep their identity; new ones use the new list.
    pub fn set_installed(&mut self, installed: Vec<InstalledGame>) {
        self.installed = installed;
    }

    /// Records a GameMode `RegisterGame` call: the process is a game regardless
    /// of heuristics. Applied on the next scan.
    pub fn register(&mut self, pid: i32) {
        self.registered.insert(pid, Evidence::GameMode);
        // Force reclassification in case it was seen before and ignored.
        self.seen.remove(&pid);
    }

    /// "This is not a game": stops treating an executable as one, now and in
    /// future. Returns `Stopped` events for anything it ends, so the caller can
    /// revert tuning immediately rather than waiting for the process to exit.
    pub fn ignore_exe(&mut self, basename: &str, now: Instant) -> Vec<Event> {
        let norm = normalise_exe(basename);
        self.ignored.insert(norm.clone());
        let key = format!("exe:{norm}");
        let mut events = Vec::new();
        if let Some(t) = self.games.remove(&key) {
            if t.announced {
                events.push(Event::Stopped { played: now - t.started_at, game: t.snapshot() });
            }
        }
        for s in self.seen.values_mut() {
            if matches!(&s.class, Class::Game(k) if *k == key)
                || matches!(&s.class, Class::Candidate { basename: b, .. } if normalise_exe(b) == norm)
            {
                s.class = Class::Ignored;
            }
        }
        events
    }

    /// Games running as of the last scan.
    pub fn running(&self) -> Vec<RunningGame> {
        let mut v: Vec<_> = self.games.values().filter(|t| t.announced).map(Tracked::snapshot).collect();
        v.sort_by(|a, b| a.key.cmp(&b.key));
        v
    }

    /// Reads the process table and reports what changed since the last scan.
    pub fn scan(&mut self, root: &SysRoot, now: Instant) -> Vec<Event> {
        let live: HashSet<i32> = proc::pids(root).into_iter().collect();
        self.seen.retain(|pid, _| live.contains(pid));
        self.registered.retain(|pid, _| live.contains(pid));

        for &pid in &live {
            let known = self.seen.get(&pid).map(|s| s.start_ticks);
            match known {
                // Already judged not a game: no reads at all. A PID reused within
                // one scan interval could slip through here, but a game has many
                // processes and a Steam game's others would still be caught.
                Some(_) if matches!(self.seen[&pid].class, Class::Ignored) => {}
                Some(start) => {
                    // Game or candidate: confirm it is the same process.
                    match proc::stat(root, pid) {
                        Some((_, cpu, s)) if s == start => self.advance_candidate(pid, cpu, now),
                        Some(_) => self.classify(root, pid, now),
                        None => {}
                    }
                }
                None => self.classify(root, pid, now),
            }
        }

        self.reconcile(now)
    }

    /// Classifies a process seen for the first time.
    fn classify(&mut self, root: &SysRoot, pid: i32, now: Instant) {
        let Some(p) = proc::read(root, pid) else { return };
        let class = self.judge(root, &p, now);
        self.seen.insert(pid, Seen { start_ticks: p.start_ticks, class });
    }

    fn judge(&mut self, root: &SysRoot, p: &proc::Proc, now: Instant) -> Class {
        if p.uid != self.uid || p.cmdline.is_empty() {
            return Class::Ignored;
        }
        let basename = p.exe_basename();
        let norm = normalise_exe(&basename);
        if self.ignored.contains(&norm) {
            return Class::Ignored;
        }

        if let Some(evidence) = self.registered.get(&p.pid).copied() {
            let mut ident = self.identify(root, p).unwrap_or_else(|| self.exe_identity(&basename, evidence));
            ident.evidence = evidence.min_strength(ident.evidence);
            return self.track(ident, p.pid, now);
        }

        if !p.is_windows_exe() && NEVER_NATIVE.contains(&p.comm.as_str()) {
            return Class::Ignored;
        }
        if is_chromium_child(p) {
            return Class::Ignored;
        }
        if p.is_windows_exe() && (NEVER_WINDOWS.contains(&norm.as_str()) || is_installer(&basename)) {
            return Class::Ignored;
        }

        if let Some(ident) = self.identify(root, p) {
            return self.track(ident, p.pid, now);
        }

        // Heuristic candidates. Native programs must use Vulkan specifically:
        // OpenGL alone is used by too much ordinary software to mean anything.
        let g = proc::graphics(root, p.pid);
        let plausible = if p.is_windows_exe() { g.any() } else { g.vulkan };
        if plausible {
            Class::Candidate { since: now, cpu_at_since: p.cpu_ticks, basename }
        } else {
            Class::Ignored
        }
    }

    /// Strong evidence: Steam App ID, installed library, or database.
    fn identify(&self, root: &SysRoot, p: &proc::Proc) -> Option<Identity> {
        if let Some(appid) = proc::steam_app_id(root, p.pid).filter(|&a| !steam::is_tool_appid(a)) {
            let known = self.db.by_steam_appid(appid);
            let installed = self.installed.iter().find(|g| g.steam_appid == Some(appid));
            return Some(Identity {
                key: format!("steam:{appid}"),
                name: installed
                    .map(|g| g.name.clone())
                    .or_else(|| known.map(|k| k.name.clone()))
                    .unwrap_or_else(|| format!("Steam game {appid}")),
                evidence: Evidence::SteamAppId,
                profile: known.map(|k| k.profile.clone()).unwrap_or_else(|| "balanced".into()),
                steam_appid: Some(appid),
                blocked_note: known.and_then(|k| k.blocked_on_linux().then(|| k.linux_note.clone()).flatten()),
            });
        }

        let norm = normalise_exe(&p.exe_basename());
        if let Some(g) = self.installed.iter().find(|g| g.exe_names.contains(&norm)) {
            let known = self.db.by_exe(&norm);
            return Some(Identity {
                key: g.id.clone(),
                name: g.name.clone(),
                evidence: Evidence::Library,
                profile: known.map(|k| k.profile.clone()).unwrap_or_else(|| "balanced".into()),
                steam_appid: g.steam_appid,
                blocked_note: known.and_then(|k| k.blocked_on_linux().then(|| k.linux_note.clone()).flatten()),
            });
        }

        let known = self.db.by_exe(&norm).or_else(|| self.db.by_cmdline(&p.cmdline))?;
        Some(Identity {
            key: format!("db:{}", known.id),
            name: known.name.clone(),
            evidence: Evidence::Database,
            profile: known.profile.clone(),
            steam_appid: known.steam_appid,
            blocked_note: known.blocked_on_linux().then(|| known.linux_note.clone()).flatten(),
        })
    }

    fn exe_identity(&self, basename: &str, evidence: Evidence) -> Identity {
        let display = basename.strip_suffix(".exe").or_else(|| basename.strip_suffix(".EXE")).unwrap_or(basename);
        Identity {
            key: format!("exe:{}", normalise_exe(basename)),
            name: display.to_string(),
            evidence,
            profile: "balanced".into(),
            steam_appid: None,
            blocked_note: None,
        }
    }

    fn track(&mut self, ident: Identity, pid: i32, now: Instant) -> Class {
        let key = ident.key.clone();
        self.games
            .entry(key.clone())
            .or_insert_with(|| Tracked {
                ident,
                pids: BTreeSet::new(),
                started_at: now,
                empty_since: None,
                announced: false,
            })
            .pids
            .insert(pid);
        Class::Game(key)
    }

    /// Promotes a heuristic candidate that has stayed busy long enough.
    fn advance_candidate(&mut self, pid: i32, cpu: u64, now: Instant) {
        let Some(seen) = self.seen.get_mut(&pid) else { return };
        let Class::Candidate { since, cpu_at_since, basename } = &mut seen.class else { return };
        let elapsed = now.saturating_duration_since(*since);
        if elapsed < HEURISTIC_WINDOW {
            return;
        }
        let secs = cpu.saturating_sub(*cpu_at_since) as f64 / self.ticks_per_sec as f64;
        let pct = secs / elapsed.as_secs_f64() * 100.0;
        if pct >= HEURISTIC_MIN_CPU_PCT {
            let basename = basename.clone();
            let ident = self.exe_identity(&basename, Evidence::Heuristic);
            let class = self.track(ident, pid, now);
            if let Some(seen) = self.seen.get_mut(&pid) {
                seen.class = class;
            }
        } else {
            // Not busy enough yet — maybe a loading screen. Start a new window.
            *since = now;
            *cpu_at_since = cpu;
        }
    }

    /// Updates game membership from the classification table and emits events.
    fn reconcile(&mut self, now: Instant) -> Vec<Event> {
        let mut members: HashMap<&str, BTreeSet<i32>> = HashMap::new();
        for (pid, s) in &self.seen {
            if let Class::Game(key) = &s.class {
                members.entry(key.as_str()).or_default().insert(*pid);
            }
        }

        let mut events = Vec::new();
        let mut finished = Vec::new();
        for (key, t) in self.games.iter_mut() {
            t.pids = members.remove(key.as_str()).unwrap_or_default();
            if t.pids.is_empty() {
                let since = *t.empty_since.get_or_insert(now);
                if now.saturating_duration_since(since) >= STOP_DEBOUNCE {
                    finished.push(key.clone());
                }
            } else {
                t.empty_since = None;
                if !t.announced {
                    t.announced = true;
                    events.push(Event::Started(t.snapshot()));
                }
            }
        }
        for key in finished {
            if let Some(t) = self.games.remove(&key) {
                if t.announced {
                    // Played time ends when the last process vanished, not when
                    // the debounce expired.
                    let ended = t.empty_since.unwrap_or(now);
                    events.push(Event::Stopped { played: ended.saturating_duration_since(t.started_at), game: t.snapshot() });
                }
            }
        }
        events
    }
}

impl Evidence {
    /// When GameMode registers a process we can also identify, keep the more
    /// specific identity's evidence; GameMode only says "a game", not which.
    fn min_strength(self, other: Evidence) -> Evidence {
        match other {
            Evidence::Heuristic => self,
            _ => other,
        }
    }
}

#[cfg(test)]
mod tests;
