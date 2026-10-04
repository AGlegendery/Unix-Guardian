//! The GPU programs start on: the dashboard's card switch.
//!
//! A hybrid laptop cannot move its desktop to the other card without a MUX
//! (the panel is wired to the integrated GPU), so the switch only changes
//! where the programs Unix Guardian starts render, with PRIME render
//! offload: `ug run` without `--gpu`, and the per-app launchers built from
//! it. The choice is a one-line file in Unix Guardian's own configuration
//! (`~/.config/unix-guardian/default-gpu`) and takes effect at once.
//!
//! **Never session-wide.** The first version wrote the offload variables
//! into `~/.config/environment.d`, and that broke KDE on the next login
//! (2026-10-04, see `claude-project/05-findings.md`): systemd's user
//! manager reads `environment.d`, KWin and plasmashell are user services,
//! and `__NV_PRIME_RENDER_OFFLOAD=1` steers **EGL** as well as GLX and
//! Vulkan, so the compositor itself rendered on the NVIDIA card while the
//! panel hangs off the Intel one: a black Wayland desktop, garbled X11.
//! No GPU variable may be written to `environment.d`, `/etc/environment`,
//! `plasma-workspace/env`, `~/.profile` or anything else the whole session
//! reads. Per program only: `ug run`, a `.desktop` entry with
//! `PrefersNonDefaultGPU` (see [`desktop_override`]), Steam launch options.
//!
//! [`retire_legacy`] moves the old version's file out of `environment.d`.

use crate::session_env::Retired;
#[cfg(test)]
use crate::session_env::SESSION_WIDE;
use std::path::{Path, PathBuf};
use ug_hw::gpu_use::Gpu;

/// The variable the old file also carried, so a session could tell which
/// choice it started with.
pub const MARK: &str = "UG_DEFAULT_GPU";

/// The file holding the choice, in Unix Guardian's configuration directory.
pub fn file(ug_config: &Path) -> PathBuf {
    ug_config.join("default-gpu")
}

/// The PCI address chosen; `None` is the system's default.
pub fn chosen(ug_config: &Path) -> Option<String> {
    let text = std::fs::read_to_string(file(ug_config)).ok()?;
    Some(text.trim().to_string()).filter(|v| !v.is_empty())
}

/// Makes `gpu` the default. The boot GPU is the system's own default, so
/// choosing it removes the file instead of writing one.
pub fn set(ug_config: &Path, gpu: &Gpu) -> std::io::Result<()> {
    let path = file(ug_config);
    if gpu.boot_vga {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    crate::paths::write_atomic(&path, format!("{}\n", gpu.pci).as_bytes())
}

/// The switch's state, for the window, the TUI and the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    /// The chosen GPU's PCI address (the boot GPU when nothing is chosen).
    pub chosen: String,
}

pub fn state(ug_config: &Path, gpus: &[Gpu]) -> State {
    let boot = gpus.iter().find(|g| g.boot_vga).or(gpus.first()).map(|g| g.pci.clone()).unwrap_or_default();
    State { chosen: chosen(ug_config).unwrap_or(boot) }
}

/// `~/.config`, the directory `environment.d` and `applications` overrides
/// are found under.
pub fn config_home() -> PathBuf {
    crate::paths::user_config().parent().map(Path::to_path_buf).unwrap_or_default()
}

/// The old version's session-wide file.
pub fn legacy_file(config_home: &Path) -> PathBuf {
    config_home.join("environment.d").join(crate::session_env::LEGACY_FILES[0])
}

/// Moves the old version's `environment.d` file aside (see
/// [`crate::session_env::retire`]) and keeps the choice it held as the
/// per-program default.
pub fn retire_legacy(config_home: &Path, ug_config: &Path, stamp: &str) -> std::io::Result<Option<Retired>> {
    let Some(r) = crate::session_env::retire(config_home, ug_config, crate::session_env::LEGACY_FILES[0], stamp)? else {
        return Ok(None);
    };
    let pci = r.vars.iter().find(|(k, _)| k == MARK).map(|(_, v)| v.as_str()).unwrap_or("");
    if !pci.is_empty() && chosen(ug_config).is_none() {
        crate::paths::write_atomic(&file(ug_config), format!("{pci}\n").as_bytes())?;
    }
    Ok(Some(r))
}

/// Marks an application entry Unix Guardian wrote, so only those are ever
/// removed again.
const DESKTOP_MARK: &str = "X-UnixGuardian-Gpu=true";

/// A per-user copy of an application's `.desktop` entry that asks the
/// desktop to start it on the discrete GPU (`PrefersNonDefaultGPU`, which
/// KDE and GNOME honour through switcheroo-control; KDE's older key too).
/// `None` when the entry has no `[Desktop Entry]` group.
pub fn desktop_override(original: &str) -> Option<String> {
    let mut out = String::new();
    let mut in_main = false;
    let mut seen_main = false;
    for line in original.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            if in_main {
                out.push_str(&format!("PrefersNonDefaultGPU=true\nX-KDE-RunOnDiscreteGpu=true\n{DESKTOP_MARK}\n"));
            }
            in_main = t == "[Desktop Entry]";
            seen_main |= in_main;
        } else if in_main
            && ["PrefersNonDefaultGPU", "X-KDE-RunOnDiscreteGpu", "X-UnixGuardian-Gpu"]
                .iter()
                .any(|k| t.split_once('=').is_some_and(|(key, _)| key.trim() == *k))
        {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if in_main {
        out.push_str(&format!("PrefersNonDefaultGPU=true\nX-KDE-RunOnDiscreteGpu=true\n{DESKTOP_MARK}\n"));
    }
    seen_main.then_some(out)
}

/// Whether an entry is one [`desktop_override`] wrote.
pub fn is_our_override(text: &str) -> bool {
    text.lines().any(|l| l.trim() == DESKTOP_MARK)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu(pci: &str, vendor: u16, device: u16, boot_vga: bool, driver: &str) -> Gpu {
        Gpu { pci: pci.into(), vendor, device, boot_vga, driver: driver.into() }
    }

    fn nvidia() -> Gpu {
        gpu("0000:01:00.0", 0x10de, 0x25a2, false, "nvidia")
    }

    fn intel() -> Gpu {
        gpu("0000:00:02.0", 0x8086, 0x9a60, true, "i915")
    }

    /// Every file under `dir`, relative to it.
    fn files(dir: &Path) -> Vec<String> {
        let mut out = Vec::new();
        let mut todo = vec![dir.to_path_buf()];
        while let Some(d) = todo.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                if e.path().is_dir() {
                    todo.push(e.path());
                } else {
                    out.push(e.path().strip_prefix(dir).unwrap().display().to_string());
                }
            }
        }
        out.sort();
        out
    }

    /// The bug of 2026-10-04: choosing NVIDIA wrote `__NV_PRIME_RENDER_OFFLOAD`
    /// into `environment.d`, and KWin rendered on the wrong card.
    #[test]
    fn choosing_nvidia_writes_nothing_the_session_reads() {
        let home = tempfile::tempdir().unwrap();
        let ug = home.path().join(".config/unix-guardian");
        set(&ug, &nvidia()).unwrap();
        let written = files(home.path());
        assert_eq!(written, [".config/unix-guardian/default-gpu"]);
        for f in &written {
            assert!(!SESSION_WIDE.iter().any(|s| f.starts_with(s)), "{f} is read by the whole session");
            let text = std::fs::read_to_string(home.path().join(f)).unwrap();
            assert!(!text.contains("__NV_") && !text.contains("__GLX_") && !text.contains("__VK_"), "{f}: {text}");
        }
        assert_eq!(chosen(&ug).as_deref(), Some("0000:01:00.0"));
        assert_eq!(state(&ug, &[intel(), nvidia()]).chosen, "0000:01:00.0");
    }

    #[test]
    fn choosing_the_integrated_gpu_removes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        set(dir.path(), &nvidia()).unwrap();
        set(dir.path(), &intel()).unwrap();
        assert!(!file(dir.path()).exists());
        assert_eq!(chosen(dir.path()), None);
        assert_eq!(state(dir.path(), &[intel(), nvidia()]).chosen, "0000:00:02.0");
        // And again, with nothing to remove.
        set(dir.path(), &intel()).unwrap();
    }

    const OLD: &str = "# Written by Unix Guardian: programs render on this GPU (PRIME render offload).\n\
        # Choose the integrated GPU in Unix Guardian (or delete this file) to undo.\n\
        __NV_PRIME_RENDER_OFFLOAD=1\n__GLX_VENDOR_LIBRARY_NAME=nvidia\n\
        __VK_LAYER_NV_optimus=NVIDIA_only\nUG_DEFAULT_GPU=0000:01:00.0\n";

    #[test]
    fn the_old_session_file_is_moved_aside_and_its_choice_kept() {
        let home = tempfile::tempdir().unwrap();
        let config = home.path().join(".config");
        let ug = config.join("unix-guardian");
        std::fs::create_dir_all(config.join("environment.d")).unwrap();
        std::fs::write(legacy_file(&config), OLD).unwrap();
        // The repair session's own folder must not be touched.
        std::fs::create_dir_all(config.join("environment.d.disabled")).unwrap();
        std::fs::write(config.join("environment.d.disabled/90-unix-guardian-gpu.conf"), OLD).unwrap();

        let r = retire_legacy(&config, &ug, "20261004T120000").unwrap().unwrap();
        assert!(!legacy_file(&config).exists());
        assert_eq!(r.moved_to, ug.join("retired/90-unix-guardian-gpu.conf.20261004T120000"));
        assert_eq!(std::fs::read_to_string(&r.moved_to).unwrap(), OLD, "moved, not rewritten");
        let names: Vec<&str> = r.vars.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["__NV_PRIME_RENDER_OFFLOAD", "__GLX_VENDOR_LIBRARY_NAME", "__VK_LAYER_NV_optimus", MARK]);
        assert_eq!(chosen(&ug).as_deref(), Some("0000:01:00.0"));
        assert_eq!(std::fs::read_to_string(config.join("environment.d.disabled/90-unix-guardian-gpu.conf")).unwrap(), OLD);
        assert!(files(&config.join("environment.d")).is_empty());
        // Nothing left to do the second time.
        assert_eq!(retire_legacy(&config, &ug, "x").unwrap(), None);
    }

    #[test]
    fn a_file_without_our_mark_is_left_alone() {
        let home = tempfile::tempdir().unwrap();
        let config = home.path().join(".config");
        std::fs::create_dir_all(config.join("environment.d")).unwrap();
        std::fs::write(legacy_file(&config), "FOO=1\n").unwrap();
        assert_eq!(retire_legacy(&config, &config.join("unix-guardian"), "x").unwrap(), None);
        assert!(legacy_file(&config).exists());
        // And no file at all.
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(retire_legacy(empty.path(), &empty.path().join("ug"), "x").unwrap(), None);
    }

    #[test]
    fn a_desktop_override_asks_for_the_discrete_gpu_in_the_main_group_only() {
        let orig = "[Desktop Entry]\nName=Blender\nExec=blender %F\nPrefersNonDefaultGPU=false\n\n\
                    [Desktop Action new]\nName=New\nExec=blender --new\n";
        let o = desktop_override(orig).unwrap();
        let main = o.split("[Desktop Action new]").next().unwrap();
        assert!(main.contains("PrefersNonDefaultGPU=true\n"));
        assert!(!main.contains("PrefersNonDefaultGPU=false"));
        assert!(main.contains("Exec=blender %F\n"));
        assert!(o.contains("[Desktop Action new]\nName=New\nExec=blender --new\n"));
        assert_eq!(o.matches("PrefersNonDefaultGPU").count(), 1);
        assert!(is_our_override(&o));
        assert!(!is_our_override(orig));
        // Twice gives the same file.
        assert_eq!(desktop_override(&o).unwrap(), o);
        assert_eq!(desktop_override("[Other]\nA=b\n"), None);
    }
}
