//! Which way gpu-screen-recorder can capture on this desktop, shared by the
//! agent's recorder and the test lab.
//!
//! - Wayland: the desktop's screen-cast portal (a dialog), or a monitor
//!   through KMS (a root helper, so a password).
//! - X11 with the desktop on an NVIDIA card: a monitor through NvFBC, no
//!   root needed.
//! - X11 with the desktop on Intel or AMD: a monitor only through KMS
//!   again. The Flatpak finds its root helper only when it is installed
//!   for all users (`/var/lib/flatpak`); a per-user install cannot record a
//!   monitor at all there (XFCE, 2026-10-10: "The recorder is not running").
//!   A window (`focused`, through XComposite) needs nothing.

use std::path::Path;

pub const FLATPAK_ID: &str = "com.dec05eba.gpu_screen_recorder";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureEnv {
    pub wayland: bool,
    /// The card that shows the desktop is NVIDIA's (X11 then uses NvFBC).
    pub nvidia_desktop: bool,
    /// gpu-screen-recorder can start its KMS helper: a native install, or
    /// the Flatpak installed for all users.
    pub kms_reachable: bool,
}

impl CaptureEnv {
    /// This session and machine. `root` is `/` outside tests.
    pub fn detect(root: &Path) -> CaptureEnv {
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
            || std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v == "wayland");
        let native = std::env::var_os("PATH").is_some_and(|p| {
            std::env::split_paths(&p).any(|d| d.join("gpu-screen-recorder").is_file())
        });
        CaptureEnv {
            wayland,
            nvidia_desktop: nvidia_desktop(root),
            kms_reachable: native || root.join("var/lib/flatpak/app").join(FLATPAK_ID).exists(),
        }
    }

    /// Recording a monitor goes through the KMS helper here.
    pub fn monitor_needs_kms(&self) -> bool {
        self.wayland || !self.nvidia_desktop
    }

    /// Recording a monitor cannot work here at all.
    pub fn monitor_unavailable(&self) -> bool {
        self.monitor_needs_kms() && !self.kms_reachable
    }

    /// What "automatic" records: the portal on Wayland; on X11 the whole
    /// screen, or the active window where a monitor cannot be recorded.
    pub fn auto_capture(&self) -> &'static str {
        if self.wayland {
            "portal"
        } else if self.monitor_unavailable() {
            "focused"
        } else {
            "screen"
        }
    }

    /// The capture a setting means (empty is automatic).
    pub fn effective<'a>(&self, setting: &'a str) -> &'a str {
        if setting.is_empty() {
            self.auto_capture()
        } else {
            setting
        }
    }

    /// Starting a recording opens a dialog or asks for a password first.
    pub fn asks_first(&self, setting: &str) -> bool {
        let c = self.effective(setting);
        c == "portal" || (is_monitor(c) && self.monitor_needs_kms())
    }
}

/// `screen` or a monitor's name (`eDP-1`), as opposed to the portal or a
/// window.
pub fn is_monitor(capture: &str) -> bool {
    !matches!(capture, "portal" | "focused" | "window" | "region") && !capture.starts_with("/dev/")
}

fn nvidia_desktop(root: &Path) -> bool {
    let Ok(dir) = std::fs::read_dir(root.join("sys/class/drm")) else {
        return false;
    };
    dir.filter_map(|e| e.ok()).any(|e| {
        let d = e.path().join("device");
        let read = |f: &str| {
            std::fs::read_to_string(d.join(f))
                .map(|s| s.trim().to_string())
                .unwrap_or_default()
        };
        read("boot_vga") == "1" && read("vendor") == "0x10de"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(wayland: bool, nvidia_desktop: bool, kms_reachable: bool) -> CaptureEnv {
        CaptureEnv {
            wayland,
            nvidia_desktop,
            kms_reachable,
        }
    }

    #[test]
    fn x11_on_intel_with_a_per_user_flatpak_records_the_window() {
        let e = env(false, false, false);
        assert!(e.monitor_unavailable());
        assert_eq!(e.auto_capture(), "focused");
        assert!(!e.asks_first(""), "a window needs no password");
    }

    #[test]
    fn x11_on_intel_with_the_system_flatpak_records_the_screen_after_a_password() {
        let e = env(false, false, true);
        assert_eq!(e.auto_capture(), "screen");
        assert!(e.asks_first(""));
        assert!(e.asks_first("eDP-1"));
        assert!(!e.asks_first("focused"));
    }

    #[test]
    fn x11_on_nvidia_records_the_screen_without_asking() {
        let e = env(false, true, false);
        assert!(!e.monitor_unavailable());
        assert_eq!(e.auto_capture(), "screen");
        assert!(!e.asks_first(""));
    }

    #[test]
    fn wayland_asks_through_the_portal_or_the_helper() {
        let e = env(true, false, true);
        assert_eq!(e.auto_capture(), "portal");
        assert!(e.asks_first(""));
        assert!(e.asks_first("portal"));
        assert!(
            e.asks_first("eDP-1"),
            "a monitor on Wayland needs the helper's password"
        );
    }

    #[test]
    fn the_desktop_card_is_read_from_boot_vga() {
        let dir = std::env::temp_dir().join(format!("ug-capture-{}", std::process::id()));
        let card = |n: &str, vendor: &str, boot: &str| {
            let d = dir.join(format!("sys/class/drm/{n}/device"));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("vendor"), vendor).unwrap();
            std::fs::write(d.join("boot_vga"), boot).unwrap();
        };
        card("card0", "0x8086\n", "1\n");
        card("card1", "0x10de\n", "0\n");
        assert!(!nvidia_desktop(&dir));
        card("card1", "0x10de\n", "1\n");
        card("card0", "0x8086\n", "0\n");
        assert!(nvidia_desktop(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
