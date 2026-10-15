//! How the computer sleeps, and what the lid and the power button do.
//!
//! Both are system settings every desktop shares, so they are set where the
//! system keeps them rather than in one desktop's configuration:
//!
//! - The sleep variant is the kernel's (`/sys/power/mem_sleep`: `s2idle`
//!   keeps the CPU in its lowest idle state and wakes fast; `deep` is S3,
//!   which uses far less power overnight where the firmware has it). The
//!   kernel forgets it at boot, so "keep" adds a tmpfiles.d line that writes
//!   it again; turning keep off removes only that file.
//! - The lid and power button are logind's (`HandleLidSwitch…`,
//!   `HandlePowerKey`), written to our own drop-in
//!   (`logind.conf.d/90-unix-guardian.conf`, read after the distribution's
//!   and other tools' files) and applied by reloading logind, which keeps
//!   every session. elogind reads the same keys from `/etc/elogind`.
//!
//! Desktops (GNOME, KDE, Hyprland shells) often take the lid or the power
//! key over with a logind block inhibitor; logind's setting is then not what
//! happens, so the inhibitors are reported with the settings instead of
//! pretending the change took effect.

use std::collections::BTreeMap;
use std::path::PathBuf;
use ug_hw::SysRoot;
use ug_proto::SleepSettings;

/// The actions offered; logind knows more (kexec, halt, factory-reset) that
/// make no sense on a lid.
pub const ACTIONS: [&str; 5] = ["ignore", "lock", "suspend", "hibernate", "poweroff"];

/// D-Bus name of each control → logind key and its built-in default.
const BUTTONS: [(&str, &str, &str); 4] = [
    ("lid-battery", "HandleLidSwitch", "suspend"),
    ("lid-ac", "HandleLidSwitchExternalPower", "suspend"),
    ("lid-docked", "HandleLidSwitchDocked", "ignore"),
    ("power-key", "HandlePowerKey", "poweroff"),
];

const MEM_SLEEP: &str = "sys/power/mem_sleep";
const KEEP_FILE: &str = "etc/tmpfiles.d/unix-guardian-sleep.conf";
const DROP_IN: &str = "90-unix-guardian.conf";

pub struct Sleep {
    root: SysRoot,
    /// A development daemon on the real `/`: reads, never writes.
    simulated: bool,
    /// A development daemon: no logind to ask, our drop-in is the truth.
    development: bool,
}

/// What logind itself reports, when it is running.
#[derive(Default)]
pub struct Live {
    /// key → value, e.g. `HandleLidSwitch` → `suspend`.
    pub values: BTreeMap<String, String>,
    /// (what, who, why) of every block inhibitor.
    pub blocks: Vec<(String, String, String)>,
}

impl Sleep {
    pub fn new(root: SysRoot, simulated: bool, development: bool) -> Sleep {
        Sleep {
            root,
            simulated,
            development,
        }
    }

    /// The variants offered and the one in use (`s2idle [deep]`).
    fn modes(&self) -> (Vec<String>, String) {
        let raw = std::fs::read_to_string(self.root.join(MEM_SLEEP)).unwrap_or_default();
        let mut cur = String::new();
        let all = raw
            .split_whitespace()
            .map(|w| {
                if let Some(m) = w.strip_prefix('[').and_then(|w| w.strip_suffix(']')) {
                    cur = m.to_string();
                    m.to_string()
                } else {
                    w.to_string()
                }
            })
            .collect();
        (all, cur)
    }

    /// systemd-logind's drop-in folder, or elogind's on systems without
    /// systemd.
    fn drop_in_dir(&self) -> PathBuf {
        let systemd = self.root.join("run/systemd/system").exists()
            || !self.root.join("etc/elogind").exists();
        if systemd {
            self.root.join("etc/systemd/logind.conf.d")
        } else {
            self.root.join("etc/elogind/logind.conf.d")
        }
    }

    /// Our own drop-in's keys.
    fn ours(&self) -> BTreeMap<String, String> {
        std::fs::read_to_string(self.drop_in_dir().join(DROP_IN))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect()
    }

    pub fn settings(&self, live: Option<&Live>) -> SleepSettings {
        let (sleep_modes, sleep_mode) = self.modes();
        let ours = self.ours();
        let value = |key: &str, default: &str| -> String {
            live.and_then(|l| l.values.get(key).cloned())
                .or_else(|| ours.get(key).cloned())
                .unwrap_or_else(|| default.to_string())
        };
        let held = |what: &str| -> Vec<String> {
            live.map(|l| {
                l.blocks
                    .iter()
                    .filter(|(w, _, _)| w.split(':').any(|w| w == what))
                    .map(|(_, who, why)| format!("{who}: {why}"))
                    .collect()
            })
            .unwrap_or_default()
        };
        let has_lid = std::fs::read_dir(self.root.join("proc/acpi/button/lid"))
            .is_ok_and(|mut d| d.next().is_some())
            || self.root.join("sys/class/input").exists() && lid_input(&self.root);
        SleepSettings {
            sleep_mode_kept: self.root.join(KEEP_FILE).exists(),
            sleep_modes: if sleep_modes.len() > 1 {
                sleep_modes
            } else {
                Vec::new()
            },
            sleep_mode,
            has_lid,
            lid_battery: value(BUTTONS[0].1, BUTTONS[0].2),
            lid_ac: value(BUTTONS[1].1, BUTTONS[1].2),
            lid_docked: value(BUTTONS[2].1, BUTTONS[2].2),
            power_key: value(BUTTONS[3].1, BUTTONS[3].2),
            lid_handled_by: held("handle-lid-switch"),
            power_key_handled_by: held("handle-power-key"),
            buttons_note: if live.is_some() || self.development {
                String::new()
            } else {
                "The login manager (systemd-logind or elogind) is not running, so the lid and \
                 power button cannot be set here."
                    .into()
            },
        }
    }

    /// Sets the sleep variant now and, with `keep`, at every boot.
    pub fn set_mode(&self, mode: &str, keep: bool) -> Result<(), String> {
        let (all, _) = self.modes();
        if !all.iter().any(|m| m == mode) {
            return Err(if all.len() > 1 {
                format!("this computer sleeps one of these ways: {}", all.join(", "))
            } else {
                "This computer's firmware offers only one way to sleep, so there is nothing to \
                 choose."
                    .into()
            });
        }
        if self.simulated {
            return Err("development service: nothing is changed".into());
        }
        let p = self.root.join(MEM_SLEEP);
        std::fs::write(&p, mode).map_err(|e| format!("{}: {e}", p.display()))?;
        let keep_file = self.root.join(KEEP_FILE);
        if keep {
            let line = format!(
                "# Written by Unix Guardian (System control → Sleep); delete to stop.\n\
                 w /sys/power/mem_sleep - - - - {mode}\n"
            );
            ug_core::paths::write_atomic(&keep_file, line.as_bytes())
                .map_err(|e| format!("{}: {e}", keep_file.display()))?;
        } else if keep_file.exists() {
            std::fs::remove_file(&keep_file)
                .map_err(|e| format!("{}: {e}", keep_file.display()))?;
        }
        Ok(())
    }

    /// Writes one lid or power-button action into our drop-in. The caller
    /// reloads logind afterwards.
    pub fn set_button(&self, which: &str, action: &str) -> Result<(), String> {
        let Some((_, key, default)) = BUTTONS.iter().find(|(w, _, _)| *w == which) else {
            return Err(format!(
                "{which:?} is not one of lid-battery, lid-ac, lid-docked, power-key"
            ));
        };
        if !ACTIONS.contains(&action) {
            return Err(format!("{action:?} is not one of {}", ACTIONS.join(", ")));
        }
        if self.simulated {
            return Err("development service: nothing is changed".into());
        }
        let mut ours = self.ours();
        if action == *default {
            // Back to the built-in default: drop our line, so the
            // distribution's own file decides again.
            ours.remove(*key);
        } else {
            ours.insert(key.to_string(), action.to_string());
        }
        let dir = self.drop_in_dir();
        let file = dir.join(DROP_IN);
        if ours.is_empty() {
            if file.exists() {
                std::fs::remove_file(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            }
            return Ok(());
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let mut text = String::from(
            "# Written by Unix Guardian (System control → Sleep and wake); delete to undo.\n\
             [Login]\n",
        );
        for (k, v) in &ours {
            text.push_str(&format!("{k}={v}\n"));
        }
        ug_core::paths::write_atomic(&file, text.as_bytes())
            .map_err(|e| format!("{}: {e}", file.display()))
    }

    pub fn uses_elogind(&self) -> bool {
        self.drop_in_dir()
            .starts_with(self.root.join("etc/elogind"))
    }
}

/// Some laptops report their lid only as an input switch (no ACPI button).
fn lid_input(root: &SysRoot) -> bool {
    std::fs::read_dir(root.join("sys/class/input"))
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| {
            std::fs::read_to_string(e.path().join("name"))
                .is_ok_and(|n| n.to_ascii_lowercase().contains("lid switch"))
        })
}

/// Makes logind read its configuration again without ending any session:
/// `systemctl reload` for systemd-logind (it handles SIGHUP since systemd
/// 254), SIGHUP for elogind.
pub fn reload_logind(elogind: bool) -> Result<(), String> {
    if !elogind {
        let out = std::process::Command::new("systemctl")
            .args(["reload", "systemd-logind.service"])
            .output()
            .map_err(|e| format!("systemctl: {e}"))?;
        if out.status.success() {
            return Ok(());
        }
        return Err(format!(
            "Saved, but logind could not reload it ({}); it applies after the next restart.",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let pid = std::fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .find(|e| {
            std::fs::read_to_string(e.path().join("comm"))
                .is_ok_and(|c| c.trim().starts_with("elogind"))
        })
        .and_then(|e| e.file_name().to_str().and_then(|s| s.parse::<i32>().ok()));
    match pid {
        // SAFETY: kill with a pid read from /proc and a valid signal.
        Some(pid) if unsafe { libc::kill(pid, libc::SIGHUP) } == 0 => Ok(()),
        _ => Err(
            "Saved, but elogind was not found to reload it; it applies after the next \
                  restart."
                .into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    fn sleep(fs: &TestFs) -> Sleep {
        fs.write(MEM_SLEEP, "s2idle [deep]\n");
        fs.write("run/systemd/system/.keep", "");
        Sleep::new(SysRoot::at(fs.root()), false, false)
    }

    fn read(fs: &TestFs, p: &str) -> String {
        std::fs::read_to_string(fs.root().join(p)).unwrap_or_default()
    }

    #[test]
    fn the_sleep_variants_and_the_one_in_use_are_read() {
        let fs = TestFs::new();
        let s = sleep(&fs).settings(None);
        assert_eq!(s.sleep_modes, ["s2idle", "deep"]);
        assert_eq!(s.sleep_mode, "deep");
        assert!(!s.sleep_mode_kept);
    }

    #[test]
    fn keeping_a_sleep_variant_writes_a_tmpfiles_line_and_unkeeping_removes_it() {
        let fs = TestFs::new();
        let s = sleep(&fs);
        s.set_mode("s2idle", true).unwrap();
        assert_eq!(read(&fs, MEM_SLEEP), "s2idle");
        assert!(read(&fs, KEEP_FILE).contains("w /sys/power/mem_sleep - - - - s2idle"));
        s.set_mode("s2idle", false).unwrap();
        assert!(!fs.root().join(KEEP_FILE).exists());
    }

    #[test]
    fn a_variant_the_kernel_does_not_offer_is_refused() {
        let fs = TestFs::new();
        let s = sleep(&fs);
        assert!(s.set_mode("hibernate", false).is_err());
        fs.write(MEM_SLEEP, "[s2idle]\n");
        let e = s.set_mode("deep", false).unwrap_err();
        assert!(e.contains("only one way"), "{e}");
        assert!(s.settings(None).sleep_modes.is_empty(), "nothing to choose");
    }

    #[test]
    fn a_lid_action_goes_into_our_own_drop_in_and_the_default_removes_it() {
        let fs = TestFs::new();
        let s = sleep(&fs);
        let file = format!("etc/systemd/logind.conf.d/{DROP_IN}");
        s.set_button("lid-ac", "ignore").unwrap();
        s.set_button("power-key", "suspend").unwrap();
        let text = read(&fs, &file);
        assert!(text.contains("[Login]"), "{text}");
        assert!(
            text.contains("HandleLidSwitchExternalPower=ignore"),
            "{text}"
        );
        assert!(text.contains("HandlePowerKey=suspend"), "{text}");
        let st = s.settings(None);
        assert_eq!(st.lid_ac, "ignore");
        assert_eq!(st.lid_battery, "suspend", "default");
        s.set_button("lid-ac", "suspend").unwrap();
        s.set_button("power-key", "poweroff").unwrap();
        assert!(
            !fs.root().join(&file).exists(),
            "all defaults: no file left"
        );
    }

    #[test]
    fn elogind_systems_get_the_drop_in_under_etc_elogind() {
        let fs = TestFs::new();
        fs.write(MEM_SLEEP, "[s2idle]\n");
        fs.write("etc/elogind/logind.conf", "[Login]\n");
        let s = Sleep::new(SysRoot::at(fs.root()), false, false);
        assert!(s.uses_elogind());
        s.set_button("lid-battery", "lock").unwrap();
        assert!(read(&fs, &format!("etc/elogind/logind.conf.d/{DROP_IN}"))
            .contains("HandleLidSwitch=lock"));
    }

    #[test]
    fn a_desktop_holding_the_lid_is_reported_and_logind_values_win() {
        let fs = TestFs::new();
        let s = sleep(&fs);
        let mut live = Live::default();
        live.values
            .insert("HandleLidSwitch".into(), "hibernate".into());
        live.blocks.push((
            "handle-lid-switch".into(),
            "ryoku-clamshell".into(),
            "route lid close through the secure Ryoku policy".into(),
        ));
        live.blocks
            .push(("sleep".into(), "NetworkManager".into(), "x".into()));
        let st = s.settings(Some(&live));
        assert_eq!(st.lid_battery, "hibernate");
        assert_eq!(st.lid_handled_by.len(), 1);
        assert!(st.lid_handled_by[0].starts_with("ryoku-clamshell"));
        assert!(st.power_key_handled_by.is_empty());
    }

    #[test]
    fn unknown_controls_and_actions_are_refused_and_development_never_writes() {
        let fs = TestFs::new();
        let s = sleep(&fs);
        assert!(s.set_button("lid", "suspend").is_err());
        assert!(s.set_button("lid-ac", "factory-reset").is_err());
        let dev = Sleep::new(SysRoot::at(fs.root()), true, true);
        assert!(dev.set_mode("s2idle", true).is_err());
        assert!(dev.set_button("lid-ac", "ignore").is_err());
        assert!(
            read(&fs, MEM_SLEEP).starts_with("s2idle [deep]"),
            "untouched"
        );
    }
}
