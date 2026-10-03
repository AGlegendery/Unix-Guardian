//! The programs open now, for "add an app" on the Playtime page.
//!
//! Only programs a person opens are offered: those with a visible desktop
//! entry (the menu's applications, named as the menu names them), and
//! Windows programs under Wine. Measured on the dev laptop's KDE session: of
//! the 70 processes holding a window-system connection, the desktop's own
//! parts (plasmashell, kded6, ksmserver, the portals, Xwayland, …) had no
//! visible desktop entry, and every app but one did. Anything else can still
//! be tracked by typing its executable name.
//!
//! Read only when asked, never on the detection tick.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use ug_games::db::normalise_exe;
use ug_games::proc;
use ug_hw::SysRoot;
use ug_proto::TrackedApp;

/// Launchers and interpreters a desktop entry may start a program through.
/// Their names say nothing about the program, so they never identify one.
const LAUNCHERS: &[&str] = &[
    "sh", "bash", "dash", "zsh", "fish", "env", "python", "python3", "perl", "ruby", "node", "java", "mono",
    "dotnet", "wine", "wine64", "pkexec", "sudo", "xdg-open", "gtk-launch", "kioclient5", "kioclient6",
    "kde-open", "gio", "exo-open", "electron", "snap", "flatpak", "steam",
];

/// Directories that hold unrelated programs side by side: a desktop entry's
/// program there says nothing about others in the same place.
const SHARED_DIRS: &[&str] = &["/usr/bin", "/usr/local/bin", "/bin", "/usr/sbin", "/sbin", "/usr/games", "/usr/libexec"];

/// Unix Guardian itself.
const OURS: &[&str] = &["ug", "ug-app", "ug-agent", "ug-daemon"];

/// Where desktop entries live: `$XDG_DATA_HOME` and `$XDG_DATA_DIRS`, plus
/// Flatpak's exports.
pub fn application_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
    let data_dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    let mut dirs = vec![data_home.clone(), data_home.join("flatpak/exports/share")];
    dirs.extend(std::env::split_paths(&data_dirs));
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share"));
    dirs.into_iter().map(|d| d.join("applications")).collect()
}

/// Desktop entries by what identifies their program: its executable's
/// basename, and — for programs installed in a directory of their own, such
/// as `/opt/google/chrome` — that directory.
#[derive(Default)]
struct Entries {
    by_exe: HashMap<String, String>,
    by_dir: HashMap<PathBuf, String>,
}

fn read_entries(dirs: &[PathBuf]) -> Entries {
    let mut e = Entries::default();
    for dir in dirs {
        let mut stack = vec![dir.clone()];
        while let Some(d) = stack.pop() {
            for f in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let path = f.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|x| x == "desktop") {
                    if let Some((program, name)) = std::fs::read_to_string(&path).ok().and_then(|t| parse_entry(&t)) {
                        add(&mut e, &program, name);
                    }
                }
            }
        }
    }
    e
}

/// An earlier directory wins, as for desktop entries themselves.
fn add(e: &mut Entries, program: &str, name: String) {
    let base = program.rsplit('/').next().unwrap_or(program).to_string();
    if base.is_empty() || LAUNCHERS.contains(&base.as_str()) {
        return;
    }
    if program.starts_with('/') {
        let real = std::fs::canonicalize(program).unwrap_or_else(|_| PathBuf::from(program));
        if let Some(parent) = real.parent().filter(|p| !SHARED_DIRS.iter().any(|s| Path::new(s) == *p)) {
            e.by_dir.entry(parent.to_path_buf()).or_insert_with(|| name.clone());
        }
    }
    e.by_exe.entry(base).or_insert(name);
}

/// `(program, Name)` of a visible application entry. The program is the
/// first word of `Exec` after `env` and variable assignments, or the
/// `--command` of a `flatpak run`.
fn parse_entry(text: &str) -> Option<(String, String)> {
    let mut in_entry = false;
    let (mut name, mut exec, mut hidden, mut app) = (None, None, false, true);
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        match k.trim() {
            "Name" => name = name.or(Some(v.trim().to_string())),
            "Exec" => exec = exec.or(Some(v.trim().to_string())),
            "NoDisplay" | "Hidden" => hidden |= v.trim().eq_ignore_ascii_case("true"),
            "Type" => app = v.trim() == "Application",
            _ => {}
        }
    }
    if hidden || !app {
        return None;
    }
    let words = split_exec(&exec?);
    let mut it = words.iter().skip_while(|w| *w == "env" || (w.contains('=') && !w.starts_with('-')));
    let first = it.next()?.clone();
    let program = if first.rsplit('/').next() == Some("flatpak") {
        words.iter().find_map(|w| w.strip_prefix("--command="))?.to_string()
    } else {
        first
    };
    Some((program, name?))
}

/// Splits an `Exec` value into words, honouring double quotes.
fn split_exec(s: &str) -> Vec<String> {
    let (mut words, mut cur, mut quoted, mut chars) = (Vec::new(), String::new(), false, s.chars());
    while let Some(c) = chars.next() {
        match c {
            '"' => quoted = !quoted,
            '\\' if quoted => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
}

/// Programs open now that could be tracked, by name. `skip` holds
/// normalised executables already tracked.
///
/// A program found through its directory may have helpers there too (a
/// crash handler next to Chrome): per name, a program matched by its own
/// executable wins, then the one with the most processes.
pub fn running(root: &SysRoot, uid: u32, test_run: Option<&str>, app_dirs: &[PathBuf], skip: &[String]) -> Vec<TrackedApp> {
    let entries = read_entries(app_dirs);
    // exe → (name, matched by its own executable, processes)
    let mut found: HashMap<String, (String, bool, usize)> = HashMap::new();
    for pid in proc::pids(root) {
        let Some(p) = proc::read(root, pid) else { continue };
        if p.uid != uid || p.cmdline.is_empty() || proc::test_run(root, pid).as_deref() != test_run {
            continue;
        }
        let basename = p.exe_basename();
        let exe = normalise_exe(&basename);
        if exe.is_empty() || OURS.contains(&exe.as_str()) || skip.contains(&exe) {
            continue;
        }
        if let Some(f) = found.get_mut(&exe) {
            f.2 += 1;
            continue;
        }
        let name = if p.is_windows_exe() {
            (!ug_games::detect::is_wine_helper(&basename)).then(|| {
                let shown = basename.strip_suffix(".exe").or_else(|| basename.strip_suffix(".EXE")).unwrap_or(&basename);
                (shown.to_string(), true)
            })
        } else if let Some(n) = entries.by_exe.get(&basename) {
            Some((n.clone(), true))
        } else {
            let path = std::fs::read_link(root.join(format!("proc/{pid}/exe"))).ok();
            path.as_ref().and_then(|p| p.parent()).and_then(|d| entries.by_dir.get(d)).map(|n| (n.clone(), false))
        };
        if let Some((name, exact)) = name {
            found.insert(exe, (name, exact, 1));
        }
    }
    let mut best: HashMap<String, (String, bool, usize)> = HashMap::new();
    for (exe, (name, exact, count)) in found {
        let better = |b: &(String, bool, usize)| (exact, count, std::cmp::Reverse(&exe)) > (b.1, b.2, std::cmp::Reverse(&b.0));
        match best.get(&name) {
            Some(b) if !better(b) => {}
            _ => {
                best.insert(name, (exe.clone(), exact, count));
            }
        }
    }
    let mut v: Vec<TrackedApp> = best.into_iter().map(|(name, (exe, _, _))| TrackedApp { exe, name }).collect();
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.exe.cmp(&b.exe)));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::{FakeProc, TestFs};

    #[test]
    fn desktop_entries_name_programs_and_hide_launchers() {
        assert_eq!(
            parse_entry("[Desktop Entry]\nType=Application\nName=Firefox ESR\nName[fa]=فایرفاکس\nExec=/usr/lib/firefox-esr/firefox-esr %u\n"),
            Some(("/usr/lib/firefox-esr/firefox-esr".into(), "Firefox ESR".into()))
        );
        assert_eq!(
            parse_entry("[Desktop Entry]\nName=Telegram\nExec=env QT_QPA_PLATFORM=xcb \"/opt/telegram/Telegram\" -- %u\n"),
            Some(("/opt/telegram/Telegram".into(), "Telegram".into()))
        );
        assert_eq!(
            parse_entry("[Desktop Entry]\nName=Steam\nExec=/usr/bin/flatpak run --branch=stable --command=/app/bin/steam-wrapper com.valvesoftware.Steam\n"),
            Some(("/app/bin/steam-wrapper".into(), "Steam".into()))
        );
        assert_eq!(parse_entry("[Desktop Entry]\nName=Portal\nExec=/usr/libexec/portal\nNoDisplay=true\n"), None);
        assert_eq!(parse_entry("[Desktop Entry]\nName=Link\nType=Link\nURL=x\n"), None);
        assert_eq!(parse_entry("[Desktop Action new]\nName=New\nExec=x\n"), None);
    }

    #[test]
    fn programs_open_now_are_offered_by_their_menu_name() {
        let fs = TestFs::new();
        let apps = fs.root().join("share/applications");
        std::fs::create_dir_all(apps.join("kde")).unwrap();
        std::fs::write(apps.join("firefox-esr.desktop"), "[Desktop Entry]\nName=Firefox ESR\nExec=/usr/lib/firefox-esr/firefox-esr %u\n").unwrap();
        std::fs::write(apps.join("kde/org.kde.dolphin.desktop"), "[Desktop Entry]\nName=Dolphin\nExec=dolphin %u\n").unwrap();
        // Chrome's entry starts a wrapper in its own directory, which then
        // runs `chrome` from there.
        std::fs::write(apps.join("google-chrome.desktop"), "[Desktop Entry]\nName=Google Chrome\nExec=/opt/google/chrome/google-chrome %U\n").unwrap();
        std::fs::write(apps.join("helper.desktop"), "[Desktop Entry]\nName=Some Tool\nExec=sh -c \"tool --x\"\n").unwrap();
        let p = |pid, comm, exe, cmd: &'static [&'static str]| FakeProc { pid, comm, exe: Some(exe), cmdline: cmd, ..Default::default() };
        fs.process(&p(10, "firefox-esr", "/usr/lib/firefox-esr/firefox-esr", &["/usr/lib/firefox-esr/firefox-esr"]));
        fs.process(&p(11, "Isolated Web Co", "/usr/lib/firefox-esr/firefox-esr", &["/usr/lib/firefox-esr/firefox-esr", "-contentproc"]));
        fs.process(&p(12, "dolphin", "/usr/bin/dolphin", &["/usr/bin/dolphin"]));
        fs.process(&p(13, "chrome", "/opt/google/chrome/chrome", &["/opt/google/chrome/chrome"]));
        fs.process(&p(20, "chrome", "/opt/google/chrome/chrome", &["/opt/google/chrome/chrome", "--type=renderer"]));
        fs.process(&p(21, "chrome_crashpad", "/opt/google/chrome/chrome_crashpad_handler", &["/opt/google/chrome/chrome_crashpad_handler"]));
        fs.process(&p(22, "crashhelper", "/usr/lib/firefox-esr/crashhelper", &["/usr/lib/firefox-esr/crashhelper"]));
        fs.process(&p(14, "plasmashell", "/usr/bin/plasmashell", &["/usr/bin/plasmashell"]));
        fs.process(&p(15, "bash", "/usr/bin/bash", &["bash"]));
        fs.process(&p(16, "ug-app", "/usr/local/bin/ug-app", &["/usr/local/bin/ug-app"]));
        fs.process(&FakeProc { pid: 17, comm: "Wondershare Fil", cmdline: &[r"C:\W\Wondershare Filmora.exe"], ..Default::default() });
        fs.process(&FakeProc { pid: 18, comm: "services.exe", cmdline: &[r"C:\windows\system32\services.exe"], ..Default::default() });
        fs.process(&FakeProc { pid: 19, uid: 1001, comm: "dolphin", exe: Some("/usr/bin/dolphin"), cmdline: &["dolphin"], ..Default::default() });

        let got = running(&SysRoot::at(fs.root()), 1000, None, &[apps], &["dolphin".to_string()]);
        let pairs: Vec<(&str, &str)> = got.iter().map(|a| (a.exe.as_str(), a.name.as_str())).collect();
        assert_eq!(pairs, vec![("firefox-esr", "Firefox ESR"), ("chrome", "Google Chrome"), ("wondershare filmora", "Wondershare Filmora")]);
    }
}
