//! The GPU programs start on: the dashboard's card switch.
//!
//! A hybrid laptop cannot move its desktop to the other card without a MUX
//! (the panel is wired to the integrated GPU), so the switch changes where
//! *programs* render, with PRIME render offload. The choice is a file in
//! `~/.config/environment.d`, which systemd reads when the session starts:
//! every program opened after the next login inherits it. Programs Unix
//! Guardian starts itself (`ug run` without `--gpu`) use it at once.
//!
//! Only render-offload variables go in the file, never ones that move a
//! program's display or the compositor: NVIDIA's offload variables touch
//! GLX and Vulkan only, and for a Mesa card only Vulkan is steered
//! (`MESA_VK_DEVICE_SELECT` without `!`), because a session-wide
//! `DRI_PRIME` would also put the shell's own EGL drawing on the discrete
//! card and keep it awake for good. The integrated GPU is the system's own
//! default, so choosing it removes the file.

use std::path::{Path, PathBuf};
use ug_hw::gpu_use::Gpu;

/// Written into the file as well, so a program can tell which choice its
/// session started with.
pub const MARK: &str = "UG_DEFAULT_GPU";

/// The file, under the user's configuration home (`~/.config`).
pub fn file(config_home: &Path) -> PathBuf {
    config_home.join("environment.d/90-unix-guardian-gpu.conf")
}

/// The session variables that start programs on `gpu`.
pub fn session_env(gpu: &Gpu) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = if gpu.driver == "nvidia" {
        [("__NV_PRIME_RENDER_OFFLOAD", "1"), ("__GLX_VENDOR_LIBRARY_NAME", "nvidia"), ("__VK_LAYER_NV_optimus", "NVIDIA_only")]
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .to_vec()
    } else {
        vec![("MESA_VK_DEVICE_SELECT".into(), format!("{:04x}:{:04x}", gpu.vendor, gpu.device))]
    };
    v.push((MARK.into(), gpu.pci.clone()));
    v
}

/// The PCI address chosen in the file; `None` is the system's default.
pub fn chosen(config_home: &Path) -> Option<String> {
    let text = std::fs::read_to_string(file(config_home)).ok()?;
    text.lines()
        .find_map(|l| l.trim().strip_prefix(MARK)?.strip_prefix('=').map(|v| v.trim().to_string()))
        .filter(|v| !v.is_empty())
}

/// The choice this session started with (from its environment).
pub fn active() -> Option<String> {
    std::env::var(MARK).ok().filter(|v| !v.is_empty())
}

/// Makes `gpu` the default. The boot GPU is the system's own default, so
/// choosing it removes the file instead of writing one.
pub fn set(config_home: &Path, gpu: &Gpu) -> std::io::Result<()> {
    let path = file(config_home);
    if gpu.boot_vga {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    let mut text = String::from(
        "# Written by Unix Guardian: programs render on this GPU (PRIME render offload).\n\
         # Choose the integrated GPU in Unix Guardian (or delete this file) to undo.\n",
    );
    for (k, v) in session_env(gpu) {
        text.push_str(&format!("{k}={v}\n"));
    }
    crate::paths::write_atomic(&path, text.as_bytes())
}

/// The switch's state, for the window, the TUI and the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    /// The chosen GPU's PCI address (the boot GPU when nothing is chosen).
    pub chosen: String,
    /// The GPU this session's programs start on.
    pub active: String,
    /// The choice takes effect at the next login.
    pub pending: bool,
}

pub fn state(config_home: &Path, gpus: &[Gpu]) -> State {
    state_for(config_home, gpus, active())
}

/// `state`, with the session's choice given rather than read from the
/// environment.
fn state_for(config_home: &Path, gpus: &[Gpu], active: Option<String>) -> State {
    let boot = gpus.iter().find(|g| g.boot_vga).or(gpus.first()).map(|g| g.pci.clone()).unwrap_or_default();
    let chosen = chosen(config_home);
    State {
        pending: chosen != active,
        chosen: chosen.unwrap_or_else(|| boot.clone()),
        active: active.unwrap_or(boot),
    }
}

/// `~/.config`, the directory `environment.d` lives in.
pub fn config_home() -> PathBuf {
    crate::paths::user_config().parent().map(Path::to_path_buf).unwrap_or_default()
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

    #[test]
    fn choosing_nvidia_writes_its_offload_variables_and_the_mark() {
        let dir = tempfile::tempdir().unwrap();
        set(dir.path(), &nvidia()).unwrap();
        let text = std::fs::read_to_string(file(dir.path())).unwrap();
        assert!(text.contains("__NV_PRIME_RENDER_OFFLOAD=1\n"));
        assert!(text.contains("__VK_LAYER_NV_optimus=NVIDIA_only\n"));
        assert!(!text.contains("DRI_PRIME"), "would move the shell's own drawing too");
        assert_eq!(chosen(dir.path()).as_deref(), Some("0000:01:00.0"));
    }

    #[test]
    fn choosing_the_integrated_gpu_removes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        set(dir.path(), &nvidia()).unwrap();
        set(dir.path(), &intel()).unwrap();
        assert!(!file(dir.path()).exists());
        assert_eq!(chosen(dir.path()), None);
        // And again, with nothing to remove.
        set(dir.path(), &intel()).unwrap();
    }

    #[test]
    fn a_mesa_card_is_steered_for_vulkan_only() {
        let amd = gpu("0000:03:00.0", 0x1002, 0x73ff, false, "amdgpu");
        let env = session_env(&amd);
        assert_eq!(env[0], ("MESA_VK_DEVICE_SELECT".to_string(), "1002:73ff".to_string()));
        assert!(env.iter().all(|(k, _)| k != "DRI_PRIME"));
    }

    #[test]
    fn a_new_choice_is_pending_until_the_next_login() {
        let dir = tempfile::tempdir().unwrap();
        let gpus = [intel(), nvidia()];
        // A session started before anything was chosen.
        let s = state_for(dir.path(), &gpus, None);
        assert_eq!((s.chosen.as_str(), s.active.as_str(), s.pending), ("0000:00:02.0", "0000:00:02.0", false));
        set(dir.path(), &nvidia()).unwrap();
        let s = state_for(dir.path(), &gpus, None);
        assert_eq!((s.chosen.as_str(), s.active.as_str(), s.pending), ("0000:01:00.0", "0000:00:02.0", true));
        // After logging in again.
        let s = state_for(dir.path(), &gpus, Some("0000:01:00.0".into()));
        assert!(!s.pending);
    }
}
