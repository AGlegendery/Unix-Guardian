//! Steam launch options written by Unix Guardian: the graphics card a game
//! renders on and the MangoHud overlay, without copying lines by hand.
//!
//! Steam keeps a game's launch options in `userdata/<account>/config/
//! localconfig.vdf`, at `UserLocalConfigStore/Software/Valve/Steam/apps/
//! <appid>/LaunchOptions`. The file is Steam's own: it rewrites it when it
//! exits, so a change made while Steam runs is lost. So writing needs Steam
//! closed, backs the file up first, changes that one value in place (every
//! other byte stays, see [`crate::vdf::set_string`]), checks the result, and
//! remembers the game's own options so the change can be taken back.
//!
//! What Unix Guardian adds goes around the user's own options: its variables
//! first, then the user's own variables, then `mangohud`, then the rest
//! (their wrappers and `%command%`). Inside Flatpak Steam the host's
//! `mangohud` is out of reach, so `MANGOHUD=1` (the Flatpak Vulkan layer) is
//! used instead, and the host's Vulkan driver paths are left out of the GPU
//! variables.

use crate::steam;
use crate::vdf::{self, Vdf};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use ug_hw::gpu_use::{self, Gpu};
use ug_hw::SysRoot;

const APPS: [&str; 5] = ["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"];
const KEY: &str = "LaunchOptions";
/// Backups of `localconfig.vdf` kept; older ones Unix Guardian made are removed.
const BACKUPS_KEPT: usize = 10;

/// The Steam install and account whose launch options are edited: the
/// account used last on this computer (its `localconfig.vdf` was written
/// last).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Steam {
    pub localconfig: PathBuf,
    /// Steam from Flathub, sandboxed: host programs are not visible to games.
    pub flatpak: bool,
}

impl Steam {
    pub fn find(home: &Path) -> Result<Steam, String> {
        let roots = steam::roots(home);
        if roots.is_empty() {
            return Err("Steam is not installed for this user.".into());
        }
        let localconfig = roots
            .iter()
            .flat_map(|r| std::fs::read_dir(r.join("userdata")).into_iter().flatten().flatten())
            .map(|account| account.path().join("config/localconfig.vdf"))
            .filter_map(|f| Some((std::fs::metadata(&f).ok()?.modified().ok()?, f)))
            .max_by_key(|(modified, _)| *modified)
            .map(|(_, f)| f)
            .ok_or("Steam has not been signed in on this computer yet.")?;
        let flatpak = localconfig.components().any(|c| c.as_os_str() == "com.valvesoftware.Steam");
        Ok(Steam { localconfig, flatpak })
    }
}

/// What Unix Guardian adds to a game's launch options.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Additions {
    /// PCI address of the GPU to render on; empty for the system's choice.
    #[serde(default)]
    pub gpu: String,
    /// Load the MangoHud overlay (styled by Unix Guardian's overlay page).
    #[serde(default)]
    pub overlay: bool,
}

impl Additions {
    pub fn is_empty(&self) -> bool {
        self.gpu.is_empty() && !self.overlay
    }
}

/// A game's launch options as Steam has them and as Unix Guardian sees them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct State {
    pub appid: u32,
    /// What Steam has now.
    pub current: String,
    /// The game's own options: `current` without what Unix Guardian added.
    pub own: String,
    /// What Unix Guardian added and is still there.
    pub additions: Additions,
    /// Unix Guardian set them once, but they were changed in Steam since;
    /// the change is then the user's and is built on as it is.
    pub changed_outside: bool,
}

/// The result of a write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Applied {
    pub options: String,
    pub backup: PathBuf,
}

/// What was written for one game, to take it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    own: String,
    ours: String,
    additions: Additions,
    file: PathBuf,
}

/// Launch options for one Steam account, with Unix Guardian's records
/// (`steam-launch.json` in its config directory) and backups (in its data
/// directory).
pub struct Launch {
    pub steam: Steam,
    root: SysRoot,
    home: PathBuf,
    records: PathBuf,
    backups: PathBuf,
}

impl Launch {
    pub fn new(steam: Steam, root: SysRoot, home: &Path, config_dir: &Path, data_dir: &Path) -> Launch {
        Launch {
            steam,
            root,
            home: home.to_path_buf(),
            records: config_dir.join("steam-launch.json"),
            backups: data_dir.join("backups"),
        }
    }

    fn records(&self) -> BTreeMap<u32, Record> {
        std::fs::read_to_string(&self.records).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    fn save_records(&self, r: &BTreeMap<u32, Record>) -> Result<(), String> {
        let text = serde_json::to_string_pretty(r).map_err(|e| e.to_string())?;
        write_replacing(&self.records, text.as_bytes(), None)
    }

    /// The launch options Steam has now for `appid` ("" for none).
    pub fn current(&self, appid: u32) -> Result<String, String> {
        let text = std::fs::read_to_string(&self.steam.localconfig).map_err(|e| format!("{}: {e}", self.steam.localconfig.display()))?;
        let doc = Vdf::parse(&text).ok_or_else(|| format!("{} did not parse", self.steam.localconfig.display()))?;
        let id = appid.to_string();
        let mut path: Vec<&str> = APPS.to_vec();
        path.extend([id.as_str(), KEY]);
        Ok(doc.path(&path).and_then(Vdf::as_str).unwrap_or_default().to_string())
    }

    pub fn state(&self, appid: u32) -> Result<State, String> {
        let current = self.current(appid)?;
        let record = self.records().remove(&appid).filter(|r| r.file == self.steam.localconfig);
        Ok(match record {
            Some(r) if r.ours == current => State { appid, own: r.own, additions: r.additions, current, changed_outside: false },
            Some(_) => State { appid, own: current.clone(), additions: Additions::default(), current, changed_outside: true },
            None => State { appid, own: current.clone(), additions: Additions::default(), current, changed_outside: false },
        })
    }

    /// The options `add` would give the game: its own options with Unix
    /// Guardian's additions (none: the game's own options alone).
    pub fn preview(&self, appid: u32, add: &Additions) -> Result<String, String> {
        let own = self.state(appid)?.own;
        let (env, wrapper) = self.additions_text(add)?;
        Ok(compose(&own, &env, wrapper))
    }

    /// Writes the options `add` gives the game. Empty additions put the
    /// game's own options back.
    pub fn apply(&self, appid: u32, add: &Additions, uid: u32) -> Result<Applied, String> {
        if steam_running(&self.root, uid) {
            return Err("Close Steam first (Steam → Exit): it rewrites its settings when it exits, so the change would be lost.".into());
        }
        let state = self.state(appid)?;
        let options = self.preview(appid, add)?;
        let src = std::fs::read_to_string(&self.steam.localconfig).map_err(|e| format!("{}: {e}", self.steam.localconfig.display()))?;
        let id = appid.to_string();
        let mut path: Vec<&str> = APPS.to_vec();
        path.push(id.as_str());
        // No options at all: no key, as Steam has it for a game never given any.
        let new = if options.is_empty() { vdf::remove_key(&src, &path, KEY)? } else { vdf::set_string(&src, &path, KEY, &options)? };

        let backup = self.backup(&src)?;
        write_replacing(&self.steam.localconfig, new.as_bytes(), Some(&self.steam.localconfig))?;
        if self.current(appid).ok().as_deref() != Some(options.as_str()) {
            // Never leave Steam's file half right: put the backup back.
            let _ = write_replacing(&self.steam.localconfig, src.as_bytes(), Some(&self.steam.localconfig));
            return Err("the launch options did not read back as written; Steam's file was restored from the backup".into());
        }

        let mut records = self.records();
        if add.is_empty() {
            records.remove(&appid);
        } else {
            records.insert(
                appid,
                Record { own: state.own, ours: options.clone(), additions: add.clone(), file: self.steam.localconfig.clone() },
            );
        }
        self.save_records(&records)?;
        tracing::info!(appid, options = %options, backup = %backup.display(), "Steam launch options written");
        Ok(Applied { options, backup })
    }

    /// Takes Unix Guardian's additions back out. Refused when the options
    /// were changed in Steam since (they are the user's then).
    pub fn undo(&self, appid: u32, uid: u32) -> Result<Applied, String> {
        let state = self.state(appid)?;
        if state.changed_outside {
            return Err("These launch options were changed in Steam after Unix Guardian set them; change them there (the game's Properties → General).".into());
        }
        self.apply(appid, &Additions::default(), uid)
    }

    /// Steam is running (see [`steam_running`]).
    pub fn steam_running(&self, uid: u32) -> bool {
        steam_running(&self.root, uid)
    }

    /// Why the overlay cannot be added to this Steam's games, if it cannot.
    pub fn overlay_problem(&self) -> Option<String> {
        overlay_available(&self.root, &self.home, self.steam.flatpak).err()
    }

    /// Unix Guardian's variables and wrapper for `add`, for this Steam.
    fn additions_text(&self, add: &Additions) -> Result<(Vec<(String, String)>, Option<&'static str>), String> {
        let mut env = Vec::new();
        if !add.gpu.is_empty() {
            let gpus = gpu_use::gpus(&self.root);
            let gpu = gpus.iter().find(|g| g.pci == add.gpu).ok_or_else(|| format!("no GPU at {}", add.gpu))?;
            env.extend(launch_env(&self.root, gpu, self.steam.flatpak));
        }
        let mut wrapper = None;
        if add.overlay {
            overlay_available(&self.root, &self.home, self.steam.flatpak)?;
            if self.steam.flatpak {
                env.push(("MANGOHUD".into(), "1".into()));
            } else {
                wrapper = Some("mangohud");
            }
        }
        Ok((env, wrapper))
    }

    /// Copies Steam's file into the backups, keeping the newest few.
    fn backup(&self, src: &str) -> Result<PathBuf, String> {
        let account = self
            .steam
            .localconfig
            .parent()
            .and_then(Path::parent)
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let path = self.backups.join(format!("localconfig-{account}-{ms}.vdf"));
        std::fs::create_dir_all(&self.backups).map_err(|e| format!("{}: {e}", self.backups.display()))?;
        std::fs::write(&path, src).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut ours: Vec<PathBuf> = std::fs::read_dir(&self.backups)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(&format!("localconfig-{account}-"))))
            .collect();
        ours.sort();
        for old in ours.iter().rev().skip(BACKUPS_KEPT) {
            let _ = std::fs::remove_file(old);
        }
        Ok(path)
    }
}

/// The GPU variables for a launch option. Inside Flatpak Steam the host's
/// Vulkan driver list (`VK_DRIVER_FILES`) names files the sandbox does not
/// have, so it is left out there.
pub fn launch_env(root: &SysRoot, gpu: &Gpu, flatpak: bool) -> Vec<(String, String)> {
    gpu_use::offload_env(root, gpu).into_iter().filter(|(k, _)| !(flatpak && k == "VK_DRIVER_FILES")).collect()
}

/// Whether the overlay can be loaded into a game of this Steam, and if not,
/// what to install.
pub fn overlay_available(root: &SysRoot, home: &Path, flatpak: bool) -> Result<(), String> {
    const LAYER: &str = "org.freedesktop.Platform.VulkanLayer.MangoHud";
    if flatpak {
        let system = root.join(format!("var/lib/flatpak/runtime/{LAYER}")).exists();
        let user = home.join(format!(".local/share/flatpak/runtime/{LAYER}")).exists();
        return (system || user)
            .then_some(())
            .ok_or_else(|| format!("Steam is the Flatpak one: MangoHud needs its Flatpak layer (flatpak install flathub {LAYER})."));
    }
    ["usr/bin/mangohud", "usr/local/bin/mangohud"]
        .iter()
        .any(|p| root.join(p).exists())
        .then_some(())
        .ok_or_else(|| "MangoHud is not installed (it is in every distribution's packages, e.g. sudo apt install mangohud).".into())
}

/// Steam is running for this user: its main process or its web helper.
pub fn steam_running(root: &SysRoot, uid: u32) -> bool {
    crate::proc::pids(root).into_iter().any(|pid| {
        let comm = std::fs::read_to_string(root.join(format!("proc/{pid}/comm"))).unwrap_or_default();
        matches!(comm.trim(), "steam" | "steamwebhelper")
            && std::fs::read_to_string(root.join(format!("proc/{pid}/status")))
                .ok()
                .and_then(|s| s.lines().find_map(|l| l.strip_prefix("Uid:")).and_then(|l| l.split_whitespace().next()?.parse::<u32>().ok()))
                == Some(uid)
    })
}

/// The game's own options with Unix Guardian's variables and wrapper added.
/// With nothing to add, the own options come back unchanged.
pub fn compose(own: &str, env: &[(String, String)], wrapper: Option<&str>) -> String {
    let words = words(own);
    let has = |name: &str| words.iter().any(|(w, _)| w.split_once('=').is_some_and(|(k, _)| k == name));
    let env: Vec<String> = env.iter().filter(|(k, _)| !has(k)).map(|(k, v)| format!("{k}={v}")).collect();
    let wrapper = wrapper.filter(|w| !words.iter().any(|(x, _)| x == w));
    if env.is_empty() && wrapper.is_none() {
        return own.to_string();
    }
    // The user's own leading variables stay in front of any wrapper.
    let split = words.iter().find(|(w, _)| !is_assignment(w)).map(|(_, at)| *at).unwrap_or(own.len());
    let (user_env, rest) = (own[..split].trim(), own[split..].trim());
    let mut parts: Vec<&str> = env.iter().map(String::as_str).collect();
    if !user_env.is_empty() {
        parts.push(user_env);
    }
    if let Some(w) = wrapper {
        parts.push(w);
    }
    if rest.contains("%command%") {
        parts.push(rest);
    } else {
        // Plain arguments: Steam adds them after the game; keep them there.
        parts.push("%command%");
        if !rest.is_empty() {
            parts.push(rest);
        }
    }
    parts.join(" ")
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(k, _)| {
        !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !k.starts_with(|c: char| c.is_ascii_digit())
    })
}

/// Shell words with the byte offset each starts at; quotes are kept in the
/// word (only its boundaries matter here).
fn words(s: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut chars = s.char_indices().peekable();
    while let Some(&(i, c)) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        let mut w = String::new();
        let mut quote: Option<char> = None;
        while let Some(&(_, c)) = chars.peek() {
            match (quote, c) {
                (None, c) if c.is_whitespace() => break,
                (None, '"' | '\'') => quote = Some(c),
                (Some(q), c) if c == q => quote = None,
                (_, '\\') => {
                    w.push(c);
                    chars.next();
                    if let Some(&(_, n)) = chars.peek() {
                        w.push(n);
                        chars.next();
                    }
                    continue;
                }
                _ => {}
            }
            w.push(c);
            chars.next();
        }
        out.push((w, i));
    }
    out
}

/// Replaces `path` through a temporary file in the same directory, keeping
/// `mode_of`'s permissions.
fn write_replacing(path: &Path, bytes: &[u8], mode_of: Option<&Path>) -> Result<(), String> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let tmp = path.with_extension(format!("ug-tmp.{}", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    if let Some(perm) = mode_of.and_then(|m| std::fs::metadata(m).ok()).map(|m| m.permissions()) {
        let _ = std::fs::set_permissions(&tmp, perm);
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use ug_testfs::{FakeProc, TestFs};

    const LOCALCONFIG: &str = "\"UserLocalConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"apps\"\n\t\t\t\t{\n\t\t\t\t\t\"730\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"-novid\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n}\n";

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn additions_go_around_the_games_own_options() {
        let nv = env(&[("__NV_PRIME_RENDER_OFFLOAD", "1")]);
        assert_eq!(compose("", &nv, Some("mangohud")), "__NV_PRIME_RENDER_OFFLOAD=1 mangohud %command%");
        assert_eq!(compose("-novid -high", &nv, None), "__NV_PRIME_RENDER_OFFLOAD=1 %command% -novid -high", "plain arguments stay after the game");
        assert_eq!(compose("gamemoderun %command% -x", &[], Some("mangohud")), "mangohud gamemoderun %command% -x");
        assert_eq!(
            compose("PROTON_LOG=1 DXVK_HUD=fps %command%", &nv, Some("mangohud")),
            "__NV_PRIME_RENDER_OFFLOAD=1 PROTON_LOG=1 DXVK_HUD=fps mangohud %command%",
            "a wrapper never comes before the user's variables (it would run them as a program)"
        );
        assert_eq!(compose(r#"FOO="a b" %command%"#, &[], Some("mangohud")), r#"FOO="a b" mangohud %command%"#);
        assert_eq!(compose("mangohud %command%", &[], Some("mangohud")), "mangohud %command%", "never twice");
        assert_eq!(compose("__NV_PRIME_RENDER_OFFLOAD=0 %command%", &nv, None), "__NV_PRIME_RENDER_OFFLOAD=0 %command%", "the user's own setting wins");
        assert_eq!(compose("  -novid ", &[], None), "  -novid ", "nothing to add: exactly the own options");
    }

    struct Setup {
        fs: TestFs,
        launch: Launch,
    }

    /// A Steam account and the dev laptop's GPUs; `flatpak` puts Steam in
    /// Flathub's place.
    fn setup(flatpak: bool) -> Setup {
        let fs = TestFs::new();
        let root = if flatpak { "home/.var/app/com.valvesoftware.Steam/.local/share/Steam" } else { "home/.local/share/Steam" };
        fs.write(&format!("{root}/steamapps/libraryfolders.vdf"), "\"libraryfolders\" {}");
        fs.write(&format!("{root}/userdata/42/config/localconfig.vdf"), LOCALCONFIG);
        for (node, pci, vendor, device, boot) in [("card0", "0000:00:02.0", "0x8086", "0x4692", "1"), ("card1", "0000:01:00.0", "0x10de", "0x25a2", "0")] {
            fs.dir(format!("sys/devices/pci0000:00/{pci}")).dir(format!("sys/class/drm/{node}"));
            fs.write(&format!("sys/devices/pci0000:00/{pci}/vendor"), vendor);
            fs.write(&format!("sys/devices/pci0000:00/{pci}/device"), device);
            fs.write(&format!("sys/devices/pci0000:00/{pci}/boot_vga"), boot);
            symlink(format!("../../../devices/pci0000:00/{pci}"), fs.root().join(format!("sys/class/drm/{node}/device"))).unwrap();
        }
        fs.dir("sys/bus/pci/drivers/nvidia");
        symlink(fs.root().join("sys/bus/pci/drivers/nvidia"), fs.root().join("sys/devices/pci0000:00/0000:01:00.0/driver")).unwrap();
        fs.write("usr/bin/mangohud", "");
        fs.write("var/lib/flatpak/runtime/org.freedesktop.Platform.VulkanLayer.MangoHud/x", "");
        let home = fs.root().join("home");
        let steam = Steam::find(&home).unwrap();
        assert_eq!(steam.flatpak, flatpak);
        let launch = Launch::new(steam, SysRoot::at(fs.root()), &home, &fs.root().join("cfg"), &fs.root().join("data"));
        Setup { fs, launch }
    }

    fn nvidia() -> Additions {
        Additions { gpu: "0000:01:00.0".into(), overlay: true }
    }

    #[test]
    fn applying_shows_first_backs_up_writes_one_value_and_undo_puts_it_back() {
        let s = setup(false);
        let l = &s.launch;
        let before = std::fs::read_to_string(&l.steam.localconfig).unwrap();
        let want = "__NV_PRIME_RENDER_OFFLOAD=1 __GLX_VENDOR_LIBRARY_NAME=nvidia __VK_LAYER_NV_optimus=NVIDIA_only mangohud %command% -novid";
        assert_eq!(l.preview(730, &nvidia()).unwrap(), want);
        assert_eq!(std::fs::read_to_string(&l.steam.localconfig).unwrap(), before, "a preview writes nothing");

        let a = l.apply(730, &nvidia(), 1000).unwrap();
        assert_eq!(a.options, want);
        assert_eq!(std::fs::read_to_string(&a.backup).unwrap(), before, "the backup is the file as it was");
        assert_eq!(l.current(730).unwrap(), want);
        let st = l.state(730).unwrap();
        assert_eq!((st.own.as_str(), st.additions.clone(), st.changed_outside), ("-novid", nvidia(), false));

        // Changing the choice builds on the game's own options, not on ours.
        let overlay_only = Additions { gpu: String::new(), overlay: true };
        assert_eq!(l.apply(730, &overlay_only, 1000).unwrap().options, "mangohud %command% -novid");

        l.undo(730, 1000).unwrap();
        assert_eq!(std::fs::read_to_string(&l.steam.localconfig).unwrap(), before, "exactly as it was");
        assert_eq!(l.state(730).unwrap().additions, Additions::default());
    }

    #[test]
    fn a_game_without_options_gets_them_added_and_undo_leaves_no_trace() {
        let s = setup(false);
        let before = std::fs::read_to_string(&s.launch.steam.localconfig).unwrap();
        let a = s.launch.apply(413150, &Additions { gpu: String::new(), overlay: true }, 1000).unwrap();
        assert_eq!(a.options, "mangohud %command%");
        assert_eq!(s.launch.current(413150).unwrap(), "mangohud %command%");
        assert_eq!(s.launch.current(730).unwrap(), "-novid");
        s.launch.undo(413150, 1000).unwrap();
        let after = std::fs::read_to_string(&s.launch.steam.localconfig).unwrap();
        assert!(after.contains("\"413150\""), "the game's block Steam would keep anyway stays");
        assert!(!after.contains("mangohud"));
        assert_eq!(after.lines().filter(|l| l.contains("LaunchOptions")).count(), before.lines().filter(|l| l.contains("LaunchOptions")).count());
    }

    #[test]
    fn nothing_is_written_while_steam_runs() {
        let s = setup(false);
        s.fs.process(&FakeProc { pid: 50, comm: "steam", cmdline: &["steam"], ..Default::default() });
        let before = std::fs::read_to_string(&s.launch.steam.localconfig).unwrap();
        assert!(s.launch.apply(730, &nvidia(), 1000).unwrap_err().contains("Close Steam"));
        assert_eq!(std::fs::read_to_string(&s.launch.steam.localconfig).unwrap(), before);
        assert!(!steam_running(&SysRoot::at(s.fs.root()), 1001), "another user's Steam does not count");
    }

    #[test]
    fn options_changed_in_steam_afterwards_are_the_users() {
        let s = setup(false);
        s.launch.apply(730, &nvidia(), 1000).unwrap();
        // The user edits them in Steam.
        let p = &s.launch.steam.localconfig;
        let text = std::fs::read_to_string(p).unwrap();
        let mut path = APPS.to_vec();
        path.push("730");
        std::fs::write(p, vdf::set_string(&text, &path, KEY, "-console").unwrap()).unwrap();
        let st = s.launch.state(730).unwrap();
        assert!(st.changed_outside);
        assert_eq!(st.own, "-console");
        assert!(s.launch.undo(730, 1000).unwrap_err().contains("changed in Steam"));
        assert_eq!(s.launch.preview(730, &Additions { gpu: String::new(), overlay: true }).unwrap(), "mangohud %command% -console");
    }

    #[test]
    fn flatpak_steam_uses_the_layer_and_no_host_driver_paths() {
        let s = setup(true);
        s.fs.write("usr/share/vulkan/icd.d/intel_icd.x86_64.json", "{}");
        let intel = Additions { gpu: "0000:00:02.0".into(), overlay: true };
        let opts = s.launch.preview(730, &intel).unwrap();
        assert!(opts.contains("MANGOHUD=1") && !opts.contains("mangohud "), "{opts}");
        assert!(!opts.contains("VK_DRIVER_FILES"), "{opts}");
        assert!(opts.contains("MESA_VK_DEVICE_SELECT=8086:4692!"), "{opts}");
    }

    #[test]
    fn a_missing_overlay_says_what_to_install() {
        let s = setup(false);
        std::fs::remove_file(s.fs.root().join("usr/bin/mangohud")).unwrap();
        assert!(s.launch.preview(730, &nvidia()).unwrap_err().contains("not installed"));
        std::fs::remove_dir_all(s.fs.root().join("var/lib/flatpak")).unwrap();
        assert!(overlay_available(&SysRoot::at(s.fs.root()), &s.fs.root().join("home"), true).unwrap_err().contains("flatpak install"));
    }

    #[test]
    fn backups_are_kept_to_the_newest_ten() {
        let s = setup(false);
        for i in 0..13 {
            let add = if i % 2 == 0 { nvidia() } else { Additions::default() };
            s.launch.apply(730, &add, 1000).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let n = std::fs::read_dir(s.fs.root().join("data/backups")).unwrap().count();
        assert_eq!(n, BACKUPS_KEPT);
    }
}
