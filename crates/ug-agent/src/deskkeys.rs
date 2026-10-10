//! The global shortcuts written into the desktop's own keyboard settings,
//! where there is no GlobalShortcuts portal: GNOME (custom keybindings in
//! gsettings) and XFCE (`xfce4-keyboard-shortcuts` in xfconf). Each runs
//! `ug shortcut <id>`.
//!
//! Only what Unix Guardian added is undone: GNOME entries live under paths
//! of our own (`…/custom-keybindings/unix-guardian-<id>/`); on XFCE a key
//! that already runs something else is left alone and reported, and undo
//! removes a key only while it still runs our command. What was written is
//! kept in `desktop-shortcuts.json`.
//!
//! Nothing session-wide is touched (no environment, no logout): both
//! desktops pick the change up at once.

use crate::keys::Combo;
use std::path::{Path, PathBuf};

const GNOME_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys";
const GNOME_ONE: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
const GNOME_BASE: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/";
const XFCE_CHANNEL: &str = "xfce4-keyboard-shortcuts";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Desk {
    Gnome,
    Xfce,
}

impl Desk {
    pub fn name(self) -> &'static str {
        match self {
            Desk::Gnome => "GNOME",
            Desk::Xfce => "XFCE",
        }
    }

    fn tool(self) -> &'static str {
        match self {
            Desk::Gnome => "gsettings",
            Desk::Xfce => "xfconf-query",
        }
    }
}

/// The desktop from `XDG_CURRENT_DESKTOP` (`ubuntu:GNOME`, `XFCE`).
pub fn desk_from(current_desktop: &str) -> Option<Desk> {
    let parts: Vec<String> = current_desktop.split(':').map(|p| p.trim().to_ascii_uppercase()).collect();
    if parts.iter().any(|p| p == "GNOME" || p == "GNOME-CLASSIC" || p == "GNOME-FLASHBACK") {
        Some(Desk::Gnome)
    } else if parts.iter().any(|p| p == "XFCE") {
        Some(Desk::Xfce)
    } else {
        None
    }
}

/// GTK's accelerator format, which both desktops store: `<Super><Shift>u`.
pub fn accel(c: &Combo) -> String {
    let mut s = String::new();
    for (on, name) in [(c.ctrl, "<Primary>"), (c.alt, "<Alt>"), (c.shift, "<Shift>"), (c.logo, "<Super>")] {
        if on {
            s.push_str(name);
        }
    }
    s + &c.key
}

/// A GVariant string literal.
fn gv(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// `@as []` or `['/a/', '/b/']` → the paths.
pub fn parse_list(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('\'') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('\'') else { break };
        out.push(after[..end].to_string());
        rest = &after[end + 1..];
    }
    out
}

fn format_list(v: &[String]) -> String {
    if v.is_empty() {
        "@as []".into()
    } else {
        format!("[{}]", v.iter().map(|p| gv(p)).collect::<Vec<_>>().join(", "))
    }
}

fn gnome_path(id: &str) -> String {
    format!("{GNOME_BASE}unix-guardian-{id}/")
}

fn xfce_prop(accel: &str) -> String {
    format!("/commands/custom/{accel}")
}

/// Runs the desktop's settings tool; the output, or why it failed.
pub trait Runner {
    fn run(&self, argv: &[String]) -> Result<String, String>;
}

pub struct RealRunner;

impl Runner for RealRunner {
    fn run(&self, argv: &[String]) -> Result<String, String> {
        let out = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("could not run {}: {e}", argv[0]))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            Err(if err.is_empty() { format!("{} failed ({})", argv[0], out.status) } else { err })
        }
    }
}

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// One shortcut to write: its id, keys and command.
pub struct Binding {
    pub id: String,
    pub name: String,
    pub keys: Combo,
    pub command: String,
}

/// What was written, to undo exactly that.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Written {
    pub desktop: String,
    /// GNOME: our keybinding paths. XFCE: our properties.
    pub entries: Vec<String>,
    /// The command each XFCE property runs (to remove it only while it is ours).
    #[serde(default)]
    pub commands: Vec<String>,
}

/// Writes the bindings. Returns what was written and the keys left alone
/// because the desktop already uses them for something else. On an error
/// nothing of ours is left in the desktop's settings.
pub fn set_up(r: &dyn Runner, desk: Desk, bindings: &[Binding], before: &Written) -> Result<(Written, Vec<String>), String> {
    // Keys may have changed since the last set-up: start from a clean slate.
    if !before.entries.is_empty() {
        undo(r, desk, before)?;
    }
    let mut w = Written { desktop: desk.name().into(), ..Default::default() };
    let mut skipped = Vec::new();
    if let Err(e) = write(r, desk, bindings, &mut w, &mut skipped) {
        let _ = undo(r, desk, &w);
        return Err(e);
    }
    Ok((w, skipped))
}

fn write(r: &dyn Runner, desk: Desk, bindings: &[Binding], w: &mut Written, skipped: &mut Vec<String>) -> Result<(), String> {
    match desk {
        Desk::Gnome => {
            let mut list = parse_list(&r.run(&argv(&["gsettings", "get", GNOME_SCHEMA, "custom-keybindings"]))?);
            for b in bindings {
                let path = gnome_path(&b.id);
                let schema = format!("{GNOME_ONE}:{path}");
                w.entries.push(path.clone());
                for (key, value) in [("name", &b.name), ("command", &b.command), ("binding", &accel(&b.keys))] {
                    r.run(&argv(&["gsettings", "set", &schema, key, &gv(value)]))?;
                }
                if !list.contains(&path) {
                    list.push(path);
                }
            }
            r.run(&argv(&["gsettings", "set", GNOME_SCHEMA, "custom-keybindings", &format_list(&list)]))?;
        }
        Desk::Xfce => {
            for b in bindings {
                let prop = xfce_prop(&accel(&b.keys));
                match r.run(&argv(&["xfconf-query", "-c", XFCE_CHANNEL, "-p", &prop])) {
                    Ok(v) if v != b.command => {
                        skipped.push(format!("{} already runs {v}", b.keys.label()));
                        continue;
                    }
                    Ok(_) => r.run(&argv(&["xfconf-query", "-c", XFCE_CHANNEL, "-p", &prop, "-s", &b.command]))?,
                    Err(_) => r.run(&argv(&["xfconf-query", "-c", XFCE_CHANNEL, "-p", &prop, "-n", "-t", "string", "-s", &b.command]))?,
                };
                w.entries.push(prop);
                w.commands.push(b.command.clone());
            }
        }
    }
    Ok(())
}

/// Removes what `set_up` wrote.
pub fn undo(r: &dyn Runner, desk: Desk, w: &Written) -> Result<(), String> {
    match desk {
        Desk::Gnome => {
            let list = parse_list(&r.run(&argv(&["gsettings", "get", GNOME_SCHEMA, "custom-keybindings"]))?);
            let kept: Vec<String> = list.into_iter().filter(|p| !w.entries.contains(p)).collect();
            r.run(&argv(&["gsettings", "set", GNOME_SCHEMA, "custom-keybindings", &format_list(&kept)]))?;
            for path in &w.entries {
                r.run(&argv(&["gsettings", "reset-recursively", &format!("{GNOME_ONE}:{path}")]))?;
            }
        }
        Desk::Xfce => {
            for (prop, cmd) in w.entries.iter().zip(&w.commands) {
                // Changed by the user since: theirs now.
                if r.run(&argv(&["xfconf-query", "-c", XFCE_CHANNEL, "-p", prop])).is_ok_and(|v| v == *cmd) {
                    r.run(&argv(&["xfconf-query", "-c", XFCE_CHANNEL, "-p", prop, "-r"]))?;
                }
            }
        }
    }
    Ok(())
}

/// The record of what was written, in the configuration directory.
pub struct DeskKeys {
    file: PathBuf,
}

impl DeskKeys {
    pub fn new(config_dir: &Path) -> DeskKeys {
        DeskKeys { file: config_dir.join("desktop-shortcuts.json") }
    }

    pub fn written(&self) -> Written {
        std::fs::read_to_string(&self.file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, w: &Written) -> Result<(), String> {
        if w.entries.is_empty() {
            return match std::fs::remove_file(&self.file) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{}: {e}", self.file.display())),
                _ => Ok(()),
            };
        }
        let text = serde_json::to_string_pretty(w).map_err(|e| e.to_string())?;
        ug_core::paths::write_atomic(&self.file, text.as_bytes()).map_err(|e| format!("{}: {e}", self.file.display()))
    }

    /// This session's desktop, if its settings tool is there.
    pub fn desk() -> Option<Desk> {
        let d = desk_from(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default())?;
        on_path(d.tool()).then_some(d)
    }
}

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file()))
}

/// `ug` next to the agent (the installed or the development one), else on PATH.
pub fn ug_command() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("ug")))
        .filter(|p| p.is_file())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "ug".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    /// gsettings and xfconf-query over a map, as far as `set_up` uses them.
    #[derive(Default)]
    struct Fake {
        store: RefCell<BTreeMap<String, String>>,
    }

    impl Runner for Fake {
        fn run(&self, a: &[String]) -> Result<String, String> {
            let mut s = self.store.borrow_mut();
            let a: Vec<&str> = a.iter().map(String::as_str).collect();
            match a.as_slice() {
                ["gsettings", "get", schema, key] => Ok(s.get(&format!("{schema} {key}")).cloned().unwrap_or_else(|| "@as []".into())),
                ["gsettings", "set", schema, key, value] => {
                    s.insert(format!("{schema} {key}"), value.to_string());
                    Ok(String::new())
                }
                ["gsettings", "reset-recursively", schema] => {
                    s.retain(|k, _| !k.starts_with(&format!("{schema} ")));
                    Ok(String::new())
                }
                ["xfconf-query", "-c", c, "-p", p] => s.get(&format!("{c}{p}")).cloned().ok_or_else(|| "no such property".into()),
                ["xfconf-query", "-c", c, "-p", p, "-r"] => {
                    s.remove(&format!("{c}{p}"));
                    Ok(String::new())
                }
                ["xfconf-query", "-c", c, "-p", p, "-s", v] | ["xfconf-query", "-c", c, "-p", p, "-n", "-t", "string", "-s", v] => {
                    s.insert(format!("{c}{p}"), v.to_string());
                    Ok(String::new())
                }
                other => Err(format!("unexpected {other:?}")),
            }
        }
    }

    fn bindings() -> Vec<Binding> {
        [("open-window", "LOGO+SHIFT+u"), ("toggle-gamemode", "CTRL+ALT+g")]
            .into_iter()
            .map(|(id, keys)| Binding {
                id: id.into(),
                name: format!("Unix Guardian: {id}"),
                keys: Combo::parse(keys).unwrap(),
                command: format!("/usr/local/bin/ug shortcut {id}"),
            })
            .collect()
    }

    #[test]
    fn desktops_are_read_from_xdg_current_desktop() {
        assert_eq!(desk_from("ubuntu:GNOME"), Some(Desk::Gnome));
        assert_eq!(desk_from("GNOME"), Some(Desk::Gnome));
        assert_eq!(desk_from("XFCE"), Some(Desk::Xfce));
        assert_eq!(desk_from("KDE"), None);
        assert_eq!(desk_from("Hyprland"), None);
        assert_eq!(desk_from(""), None);
    }

    #[test]
    fn keys_are_written_as_gtk_accelerators() {
        assert_eq!(accel(&Combo::parse("LOGO+SHIFT+u").unwrap()), "<Shift><Super>u");
        assert_eq!(accel(&Combo::parse("Ctrl+Alt+F5").unwrap()), "<Primary><Alt>F5");
        assert_eq!(accel(&Combo::parse("Meta+Page_Up").unwrap()), "<Super>Page_Up");
    }

    #[test]
    fn gsettings_lists_round_trip() {
        assert!(parse_list("@as []").is_empty());
        let v = parse_list("['/a/custom0/', '/b/unix-guardian-x/']");
        assert_eq!(v, ["/a/custom0/", "/b/unix-guardian-x/"]);
        assert_eq!(format_list(&v), "['/a/custom0/', '/b/unix-guardian-x/']");
        assert_eq!(format_list(&[]), "@as []");
        assert_eq!(gv("it's"), "'it\\'s'");
    }

    #[test]
    fn gnome_set_up_keeps_the_users_own_bindings_and_undo_restores_them() {
        let f = Fake::default();
        let list_key = format!("{GNOME_SCHEMA} custom-keybindings");
        f.store.borrow_mut().insert(list_key.clone(), "['/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/']".into());
        let (w, skipped) = set_up(&f, Desk::Gnome, &bindings(), &Written::default()).unwrap();
        assert!(skipped.is_empty());
        assert_eq!(w.entries.len(), 2);
        let list = parse_list(&f.store.borrow()[&list_key]);
        assert_eq!(list.len(), 3);
        assert!(list[0].ends_with("custom0/"), "the user's own binding stays first");
        let one = format!("{GNOME_ONE}:{GNOME_BASE}unix-guardian-open-window/");
        assert_eq!(f.store.borrow()[&format!("{one} binding")], "'<Shift><Super>u'");
        assert_eq!(f.store.borrow()[&format!("{one} command")], "'/usr/local/bin/ug shortcut open-window'");

        // Again (new keys): no duplicates in the list.
        let (w, _) = set_up(&f, Desk::Gnome, &bindings(), &w).unwrap();
        assert_eq!(parse_list(&f.store.borrow()[&list_key]).len(), 3);

        undo(&f, Desk::Gnome, &w).unwrap();
        let s = f.store.borrow();
        assert_eq!(parse_list(&s[&list_key]), ["/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/"]);
        assert!(!s.keys().any(|k| k.contains("unix-guardian")), "{s:?}");
    }

    #[test]
    fn xfce_set_up_leaves_keys_that_run_something_else_and_undo_removes_only_ours() {
        let f = Fake::default();
        let taken = format!("{XFCE_CHANNEL}/commands/custom/<Primary><Alt>g");
        f.store.borrow_mut().insert(taken.clone(), "gimp".into());
        let (w, skipped) = set_up(&f, Desk::Xfce, &bindings(), &Written::default()).unwrap();
        assert_eq!(w.entries, ["/commands/custom/<Shift><Super>u"]);
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped, ["Ctrl+Alt+G already runs gimp"]);
        let ours = format!("{XFCE_CHANNEL}/commands/custom/<Shift><Super>u");
        assert_eq!(f.store.borrow()[&ours], "/usr/local/bin/ug shortcut open-window");

        undo(&f, Desk::Xfce, &w).unwrap();
        assert!(!f.store.borrow().contains_key(&ours));
        assert_eq!(f.store.borrow()[&taken], "gimp", "someone else's key is never touched");

        // Changed by the user after the set-up: theirs, kept by undo.
        let (w, _) = set_up(&f, Desk::Xfce, &bindings(), &Written::default()).unwrap();
        f.store.borrow_mut().insert(ours.clone(), "xfce4-terminal".into());
        undo(&f, Desk::Xfce, &w).unwrap();
        assert_eq!(f.store.borrow()[&ours], "xfce4-terminal");
    }
}
