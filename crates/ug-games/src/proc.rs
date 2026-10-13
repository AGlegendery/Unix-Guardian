//! Just enough of `/proc` to recognise games.
//!
//! Detection scans the process table every couple of seconds, so everything
//! here is shaped around doing as little as possible per scan: a directory
//! listing, and the small files of processes not seen before. `maps` — the
//! one large file — is read only for processes that are already candidates.

use std::path::PathBuf;
use ug_hw::SysRoot;

/// What detection needs to know about one process.
#[derive(Debug, Clone, PartialEq)]
pub struct Proc {
    pub pid: i32,
    pub ppid: i32,
    pub uid: u32,
    /// `comm`: the executable name, truncated by the kernel to 15 bytes.
    pub comm: String,
    pub cmdline: Vec<String>,
    /// Start time since boot in clock ticks. A PID is reused; (pid, start) is not.
    pub start_ticks: u64,
    /// User + system CPU time in clock ticks.
    pub cpu_ticks: u64,
    /// Basename of `/proc/<pid>/exe`: the binary actually running. Unlike the
    /// command line, a process cannot rewrite it.
    pub exe_name: Option<String>,
}

impl Proc {
    /// The executable's basename as a user would name it.
    ///
    /// Under Wine, `argv[0]` is the Windows path of the game (`C:\...\Game.exe`)
    /// while `comm` is truncated and the real `exe` link points at the Wine
    /// preloader, so `argv[0]` is the only place the game's own name survives.
    ///
    /// Everything else is named by its `exe` link. Its command line is not
    /// trustworthy for this: Chromium and Electron overwrite theirs with one
    /// long space-separated string, whose last `/`-separated piece is some
    /// argument's path, not the program.
    pub fn exe_basename(&self) -> String {
        let first = self.cmdline.first().map(String::as_str).unwrap_or("");
        if self.is_windows_exe() {
            return first
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(first)
                .to_string();
        }
        if let Some(e) = self.exe_name.as_deref().filter(|e| !e.is_empty()) {
            return e.to_string();
        }
        let base = first.rsplit('/').next().unwrap_or(first);
        if !base.is_empty() && !base.contains(char::is_whitespace) {
            base.to_string()
        } else {
            self.comm.clone()
        }
    }

    /// Command-line arguments, including those packed into a rewritten
    /// process title (see [`Proc::exe_basename`]).
    pub fn args(&self) -> impl Iterator<Item = &str> {
        self.cmdline.iter().flat_map(|a| a.split_whitespace())
    }

    /// True when this looks like a Windows program running under Wine/Proton.
    pub fn is_windows_exe(&self) -> bool {
        self.cmdline
            .first()
            .is_some_and(|a| a.to_ascii_lowercase().ends_with(".exe"))
    }
}

/// All numeric entries of `/proc`.
pub fn pids(root: &SysRoot) -> Vec<i32> {
    let Ok(entries) = std::fs::read_dir(root.join("proc")) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse::<i32>().ok())
        .collect()
}

fn dir(root: &SysRoot, pid: i32) -> PathBuf {
    root.join(format!("proc/{pid}"))
}

/// Reads `stat` alone — PID identity and CPU time. The only per-scan read for
/// processes already classified.
pub fn stat(root: &SysRoot, pid: i32) -> Option<(i32, u64, u64)> {
    let s = std::fs::read_to_string(dir(root, pid).join("stat")).ok()?;
    // `comm` is parenthesised and may itself contain spaces and parentheses,
    // so fields are counted from the *last* closing parenthesis.
    let rest = &s[s.rfind(')')? + 2..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    let ppid = f.get(1)?.parse().ok()?;
    let cpu = f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?;
    let start = f.get(19)?.parse().ok()?;
    Some((ppid, cpu, start))
}

/// Reads everything detection needs about a process. `None` if it exited
/// mid-read, which is routine.
pub fn read(root: &SysRoot, pid: i32) -> Option<Proc> {
    let d = dir(root, pid);
    let (ppid, cpu_ticks, start_ticks) = stat(root, pid)?;
    let status = std::fs::read_to_string(d.join("status")).ok()?;
    let uid = status
        .lines()
        .find_map(|l| l.strip_prefix("Uid:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let comm = std::fs::read_to_string(d.join("comm"))
        .ok()?
        .trim_end()
        .to_string();
    // Kernel threads have an empty cmdline; that is fine, they are never games.
    let cmdline = std::fs::read(d.join("cmdline"))
        .map(|b| {
            b.split(|&c| c == 0)
                .filter(|a| !a.is_empty())
                .map(|a| String::from_utf8_lossy(a).into_owned())
                .collect()
        })
        .unwrap_or_default();
    // Readable for the user's own processes, which are the only ones
    // detection considers; a deleted binary reads as "name (deleted)".
    let exe_name = std::fs::read_link(d.join("exe")).ok().and_then(|p| {
        let name = p.file_name()?.to_string_lossy().into_owned();
        Some(
            name.strip_suffix(" (deleted)")
                .map(str::to_string)
                .unwrap_or(name),
        )
    });
    Some(Proc {
        pid,
        ppid,
        uid,
        comm,
        cmdline,
        start_ticks,
        cpu_ticks,
        exe_name,
    })
}

/// The Steam App ID a process was launched with, if any.
///
/// Steam sets `SteamAppId` (and `SteamGameId`, which carries the 64-bit id of
/// non-Steam shortcuts) in the environment of everything it launches, and it is
/// inherited by the whole game process tree. `0` means "not a game".
pub fn steam_app_id(root: &SysRoot, pid: i32) -> Option<u32> {
    let env = std::fs::read(dir(root, pid).join("environ")).ok()?;
    let mut game_id = None;
    for var in env.split(|&c| c == 0) {
        if let Some(v) = var.strip_prefix(b"SteamAppId=") {
            let id: u32 = std::str::from_utf8(v).ok()?.parse().ok()?;
            return (id != 0).then_some(id);
        }
        if let Some(v) = var.strip_prefix(b"SteamGameId=") {
            game_id = std::str::from_utf8(v)
                .ok()
                .and_then(|s| s.parse::<u64>().ok());
        }
    }
    // Non-Steam shortcuts: SteamGameId's high bits hold the shortcut id and the
    // App ID field is unused, so the low 32 bits identify it well enough.
    game_id
        .filter(|&g| g != 0)
        .map(|g| (g >> 32) as u32)
        .filter(|&id| id != 0)
}

/// The `UG_TEST_RUN` token a process was started with, if any. Set by the
/// end-to-end scripts on everything they start, so fake games stay invisible
/// to any agent outside that test run.
pub fn test_run(root: &SysRoot, pid: i32) -> Option<String> {
    let env = std::fs::read(dir(root, pid).join("environ")).ok()?;
    env.split(|&c| c == 0)
        .find_map(|var| var.strip_prefix(b"UG_TEST_RUN="))
        .map(|v| String::from_utf8_lossy(v).into_owned())
}

/// Which graphics APIs a process has loaded, from its memory map.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Graphics {
    pub vulkan: bool,
    pub opengl: bool,
    /// DirectX through Wine/DXVK/VKD3D (`d3d*.dll`, `dxgi.dll`).
    pub direct3d: bool,
}

impl Graphics {
    pub fn any(&self) -> bool {
        self.vulkan || self.opengl || self.direct3d
    }
}

pub fn graphics(root: &SysRoot, pid: i32) -> Graphics {
    let Ok(maps) = std::fs::read_to_string(dir(root, pid).join("maps")) else {
        return Graphics::default();
    };
    let mut g = Graphics::default();
    for line in maps.lines() {
        let Some(path) = line.split_whitespace().nth(5) else {
            continue;
        };
        let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
        if name.starts_with("libvulkan") || name == "vulkan-1.dll" || name.starts_with("winevulkan")
        {
            g.vulkan = true;
        } else if name.starts_with("libgl") || name.starts_with("libegl") || name == "opengl32.dll"
        {
            g.opengl = true;
        } else if (name.starts_with("d3d") && name.ends_with(".dll")) || name == "dxgi.dll" {
            g.direct3d = true;
        }
    }
    g
}

/// Clock ticks per second, for turning CPU ticks into a percentage.
pub fn clock_ticks_per_second() -> u64 {
    // SAFETY: sysconf has no preconditions and does not touch memory we own.
    let v = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if v > 0 {
        v as u64
    } else {
        100
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::{FakeProc, TestFs};

    #[test]
    fn reads_a_process_whose_name_contains_parentheses() {
        let fs = TestFs::new();
        fs.process(&FakeProc {
            pid: 42,
            comm: "a (weird) name",
            cpu_ticks: 250,
            start_ticks: 900,
            ..Default::default()
        });
        let p = read(&SysRoot::at(fs.root()), 42).unwrap();
        assert_eq!(p.comm, "a (weird) name");
        assert_eq!(p.cpu_ticks, 250);
        assert_eq!(p.start_ticks, 900);
        assert_eq!(p.uid, 1000);
    }

    #[test]
    fn a_wine_process_is_named_by_its_windows_path() {
        let fs = TestFs::new();
        fs.process(&FakeProc {
            pid: 7,
            comm: "Cyberpunk2077.e",
            cmdline: &[
                r"C:\Games\Cyberpunk 2077\bin\x64\Cyberpunk2077.exe",
                "--launcher-skip",
            ],
            ..Default::default()
        });
        let p = read(&SysRoot::at(fs.root()), 7).unwrap();
        assert!(p.is_windows_exe());
        assert_eq!(p.exe_basename(), "Cyberpunk2077.exe");
    }

    #[test]
    fn steam_app_id_comes_from_the_environment() {
        let fs = TestFs::new();
        fs.process(&FakeProc {
            pid: 1,
            environ: &[("HOME", "/h"), ("SteamAppId", "730")],
            ..Default::default()
        });
        fs.process(&FakeProc {
            pid: 2,
            environ: &[("SteamAppId", "0")],
            ..Default::default()
        });
        fs.process(&FakeProc {
            pid: 3,
            environ: &[("PATH", "/usr/bin")],
            ..Default::default()
        });
        let root = SysRoot::at(fs.root());
        assert_eq!(steam_app_id(&root, 1), Some(730));
        assert_eq!(steam_app_id(&root, 2), None, "0 is Steam's 'not a game'");
        assert_eq!(steam_app_id(&root, 3), None);
    }

    #[test]
    fn graphics_apis_are_read_from_the_memory_map() {
        let fs = TestFs::new();
        fs.process(&FakeProc {
            pid: 9,
            maps: &[
                "/usr/lib/libc.so.6",
                "/pfx/drive_c/windows/system32/d3d11.dll",
                "/usr/lib/libvulkan.so.1",
            ],
            ..Default::default()
        });
        fs.process(&FakeProc {
            pid: 10,
            maps: &["/usr/lib/libc.so.6"],
            ..Default::default()
        });
        let root = SysRoot::at(fs.root());
        let g = graphics(&root, 9);
        assert!(g.direct3d && g.vulkan && !g.opengl);
        assert!(!graphics(&root, 10).any());
    }
}
