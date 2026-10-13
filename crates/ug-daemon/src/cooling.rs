//! Fan curves and the laptop firmware's fan modes.
//!
//! A curve takes a fan out of automatic control, so three things put it back:
//! the daemon's clean stop, the next start after a crash (the fans it had
//! switched are listed in `fans-restore.json` *before* they are switched),
//! and the loop itself when a fan's temperature can no longer be read.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use ug_hw::{fans, SysRoot};
use ug_proto::{CoolingState, FanInfo};

use crate::polkit::{action, Gate};

/// Lenovo IdeaPad `fan_mode` values the driver documents (2, dust cleaning,
/// runs the fan at full speed on purpose and is not offered).
const IDEAPAD_MODES: [(i32, &str); 3] = [(0, "quiet"), (1, "standard"), (4, "cooling")];
const IDEAPAD_FAN_MODE: &str = "sys/bus/platform/devices/VPC2004:00/fan_mode";

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    #[serde(default)]
    curves: BTreeMap<String, Vec<(f64, f64)>>,
}

/// What to restore: fan id → (hwmon dir, output, previous `_enable`).
#[derive(Serialize, Deserialize, Default)]
struct Restore {
    #[serde(default)]
    fans: BTreeMap<String, (PathBuf, u32, u32)>,
}

pub struct Cooling {
    root: SysRoot,
    /// A development daemon on the real `/`: shows, never writes.
    simulated: bool,
    saved_path: PathBuf,
    restore_path: PathBuf,
    curves: BTreeMap<String, Vec<(f64, f64)>>,
    restore: Restore,
    /// Smoothed temperature and last duty per curve fan.
    ema: BTreeMap<String, f64>,
    pct: BTreeMap<String, f64>,
    misses: BTreeMap<String, u32>,
}

pub type SharedCooling = Arc<Mutex<Cooling>>;

fn write(p: &Path, v: &str) -> Result<(), String> {
    std::fs::write(p, v).map_err(|e| format!("{}: {e}", p.display()))
}

impl Cooling {
    pub fn new(root: SysRoot, simulated: bool, state_dir: &Path) -> Cooling {
        let saved_path = state_dir.join("fan-curves.json");
        let restore_path = state_dir.join("fans-restore.json");
        let saved: Saved = std::fs::read_to_string(&saved_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let restore: Restore = std::fs::read_to_string(&restore_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let mut c = Cooling {
            root,
            simulated,
            saved_path,
            restore_path,
            curves: saved.curves,
            restore,
            ema: BTreeMap::new(),
            pct: BTreeMap::new(),
            misses: BTreeMap::new(),
        };
        // A crash left fans in manual mode: give them back first.
        if !c.restore.fans.is_empty() {
            tracing::warn!(
                fans = c.restore.fans.len(),
                "fans were left under Unix Guardian's control; handing them back"
            );
            c.restore_all();
        }
        c
    }

    fn save(&self) {
        let s = Saved {
            curves: self.curves.clone(),
        };
        let _ = ug_core::paths::write_atomic(
            &self.saved_path,
            serde_json::to_string_pretty(&s)
                .expect("serialize")
                .as_bytes(),
        );
    }

    fn save_restore(&self) -> Result<(), String> {
        ug_core::paths::write_atomic(
            &self.restore_path,
            serde_json::to_string(&self.restore)
                .expect("serialize")
                .as_bytes(),
        )
        .map_err(|e| e.to_string())
    }

    /// Every fan back to what it was before Unix Guardian took it.
    pub fn restore_all(&mut self) {
        for (id, (dir, n, prev)) in std::mem::take(&mut self.restore.fans) {
            if let Err(e) = write(&dir.join(format!("pwm{n}_enable")), &prev.to_string()) {
                tracing::error!(fan = %id, error = %e, "could not hand the fan back to automatic control");
            }
        }
        let _ = std::fs::remove_file(&self.restore_path);
    }

    fn release(&mut self, id: &str) {
        if let Some((dir, n, prev)) = self.restore.fans.remove(id) {
            let _ = write(&dir.join(format!("pwm{n}_enable")), &prev.to_string());
            let _ = self.save_restore();
        }
        self.ema.remove(id);
        self.pct.remove(id);
    }

    pub fn state(&self) -> CoolingState {
        let mut st = CoolingState {
            fan_mode: -1,
            simulated: self.simulated,
            ..Default::default()
        };
        for f in fans::discover(&self.root) {
            let curve = self.curves.get(&f.id).cloned().unwrap_or_default();
            st.fans.push(FanInfo {
                label: match f.chip.as_str() {
                    "amdgpu" => "Graphics card fan".into(),
                    other => format!("{other} fan {}", f.n),
                },
                rpm: f.rpm.unwrap_or(0),
                pct: f.pwm.map(|p| p as f64 * 100.0 / 255.0).unwrap_or(f64::NAN),
                temp_c: fans::source_temp(&self.root, &f).unwrap_or(f64::NAN),
                mode: if curve.is_empty() {
                    "auto".into()
                } else {
                    "curve".into()
                },
                curve,
                id: f.id,
            });
        }
        if let Ok(v) = std::fs::read_to_string(self.root.join(IDEAPAD_FAN_MODE)) {
            st.fan_mode = v.trim().parse().unwrap_or(-1);
            st.fan_modes = IDEAPAD_MODES
                .iter()
                .map(|(v, n)| (*v, n.to_string()))
                .collect();
        }
        if st.fans.is_empty() {
            st.notes.push(if st.fan_mode >= 0 {
                "The fans are run by the laptop's firmware; it offers the fan modes below.".into()
            } else {
                "No fan here can be driven by software: the firmware runs them. Board fans may appear after loading the right sensor driver (sensors-detect)."
                    .into()
            });
        }
        if self.simulated {
            st.notes
                .push("Development service: shown, never changed.".into());
        }
        st
    }

    pub fn set_curve(&mut self, id: &str, points: Vec<(f64, f64)>) -> Result<(), String> {
        fans::validate(&points)?;
        let fan = fans::discover(&self.root)
            .into_iter()
            .find(|f| f.id == id)
            .ok_or_else(|| format!("no fan {id}"))?;
        if self.simulated {
            return Err("development service: nothing is changed".into());
        }
        if !self.restore.fans.contains_key(id) {
            // Recorded before the switch, so a crash right after still restores it.
            self.restore.fans.insert(
                id.to_string(),
                (fan.dir.clone(), fan.n, fan.enable.unwrap_or(2)),
            );
            self.save_restore()?;
            write(&fan.dir.join(format!("pwm{}_enable", fan.n)), "1")?;
        }
        self.curves.insert(id.to_string(), points);
        self.save();
        self.tick();
        Ok(())
    }

    pub fn set_auto(&mut self, id: &str) -> Result<(), String> {
        self.curves.remove(id);
        self.save();
        self.release(id);
        Ok(())
    }

    pub fn set_fan_mode(&mut self, mode: i32) -> Result<(), String> {
        if !IDEAPAD_MODES.iter().any(|(v, _)| *v == mode) {
            return Err("unknown fan mode".into());
        }
        let p = self.root.join(IDEAPAD_FAN_MODE);
        if !p.exists() {
            return Err("this machine has no firmware fan modes".into());
        }
        if self.simulated {
            return Err("development service: nothing is changed".into());
        }
        write(&p, &mode.to_string())
    }

    /// Once a second while a curve is set.
    pub fn tick(&mut self) {
        if self.simulated {
            return;
        }
        let all = fans::discover(&self.root);
        for (id, curve) in self.curves.clone() {
            let Some(f) = all.iter().find(|f| f.id == id) else {
                continue;
            };
            if !self.restore.fans.contains_key(&id) {
                // Applied again after a restart: take it over the same way.
                self.restore
                    .fans
                    .insert(id.clone(), (f.dir.clone(), f.n, f.enable.unwrap_or(2)));
                if self.save_restore().is_err()
                    || write(&f.dir.join(format!("pwm{}_enable", f.n)), "1").is_err()
                {
                    continue;
                }
            }
            match fans::source_temp(&self.root, f) {
                Some(t) => {
                    self.misses.remove(&id);
                    let e = self.ema.get(&id).map_or(t, |prev| prev + 0.3 * (t - prev));
                    self.ema.insert(id.clone(), e);
                    let target = fans::curve_pct(&curve, e);
                    let pct = fans::next_pct(*self.pct.get(&id).unwrap_or(&0.0), target, 5.0);
                    self.pct.insert(id.clone(), pct);
                    let _ = write(
                        &f.dir.join(format!("pwm{}", f.n)),
                        &((pct * 255.0 / 100.0).round() as u32).to_string(),
                    );
                }
                None => {
                    let m = self.misses.entry(id.clone()).or_insert(0);
                    *m += 1;
                    if *m >= 5 {
                        tracing::warn!(fan = %id, "its temperature cannot be read; handing it back to automatic control");
                        self.curves.remove(&id);
                        self.save();
                        self.release(&id);
                    }
                }
            }
        }
    }

    pub fn busy(&self) -> bool {
        !self.curves.is_empty()
    }
}

pub async fn run(c: SharedCooling) {
    loop {
        let x = Arc::clone(&c);
        let busy = tokio::task::spawn_blocking(move || {
            let mut c = x.lock().unwrap_or_else(|e| e.into_inner());
            c.tick();
            c.busy()
        })
        .await
        .unwrap_or(false);
        tokio::time::sleep(Duration::from_secs(if busy { 1 } else { 15 })).await;
    }
}

pub struct CoolingIface {
    pub cooling: SharedCooling,
    pub gate: Gate,
}

fn lock(c: &SharedCooling) -> std::sync::MutexGuard<'_, Cooling> {
    c.lock().unwrap_or_else(|e| e.into_inner())
}

#[zbus::interface(name = "org.unixguardian.Daemon1.Cooling")]
impl CoolingIface {
    async fn state(&self) -> CoolingState {
        let c = Arc::clone(&self.cooling);
        tokio::task::spawn_blocking(move || lock(&c).state())
            .await
            .unwrap_or_default()
    }

    /// Drives a fan by a curve of (°C, %) points; refused if it would run
    /// below 30 % at 80 °C or more.
    async fn set_curve(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        id: &str,
        points: Vec<(f64, f64)>,
    ) -> zbus::fdo::Result<()> {
        self.gate.authorize(&hdr, action::COOLING).await?;
        let c = Arc::clone(&self.cooling);
        let id = id.to_string();
        tokio::task::spawn_blocking(move || lock(&c).set_curve(&id, points))
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
            .map_err(zbus::fdo::Error::InvalidArgs)
    }

    /// Hands a fan back to automatic control.
    async fn set_auto(&self, id: &str) -> zbus::fdo::Result<()> {
        let c = Arc::clone(&self.cooling);
        let id = id.to_string();
        tokio::task::spawn_blocking(move || lock(&c).set_auto(&id))
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
            .map_err(zbus::fdo::Error::Failed)
    }

    /// The laptop firmware's fan mode (see `State().fan_modes`).
    async fn set_fan_mode(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        mode: i32,
    ) -> zbus::fdo::Result<()> {
        self.gate.authorize(&hdr, action::COOLING).await?;
        let c = Arc::clone(&self.cooling);
        tokio::task::spawn_blocking(move || lock(&c).set_fan_mode(mode))
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
            .map_err(zbus::fdo::Error::InvalidArgs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    const G: &str = "sys/class/hwmon/hwmon3";

    fn card(fs: &TestFs, temp_milli: u32) {
        fs.write(format!("{G}/name"), "amdgpu")
            .write(format!("{G}/pwm1"), "100")
            .write(format!("{G}/pwm1_enable"), "2");
        fs.write(format!("{G}/temp2_label"), "junction")
            .write(format!("{G}/temp2_input"), &temp_milli.to_string());
    }

    fn get(fs: &TestFs, f: &str) -> String {
        std::fs::read_to_string(fs.root().join(format!("{G}/{f}")))
            .unwrap()
            .trim()
            .to_string()
    }

    #[test]
    fn a_curve_drives_the_fan_and_auto_hands_it_back() {
        let fs = TestFs::new();
        card(&fs, 70_000);
        let d = tempfile::tempdir().unwrap();
        let mut c = Cooling::new(SysRoot::at(fs.root()), false, d.path());
        c.set_curve("amdgpu/pwm1", vec![(40.0, 20.0), (80.0, 100.0)])
            .unwrap();
        assert_eq!(get(&fs, "pwm1_enable"), "1");
        assert_eq!(get(&fs, "pwm1"), "204", "70 °C → 80 % → 204/255");
        c.set_auto("amdgpu/pwm1").unwrap();
        assert_eq!(get(&fs, "pwm1_enable"), "2", "back to what it was");
        assert!(
            !d.path().join("fans-restore.json").exists()
                || std::fs::read_to_string(d.path().join("fans-restore.json"))
                    .unwrap()
                    .contains("{}")
        );
    }

    #[test]
    fn after_a_crash_the_fan_is_handed_back_before_anything_else() {
        let fs = TestFs::new();
        card(&fs, 60_000);
        let d = tempfile::tempdir().unwrap();
        let mut c = Cooling::new(SysRoot::at(fs.root()), false, d.path());
        c.set_curve("amdgpu/pwm1", vec![(40.0, 30.0), (80.0, 100.0)])
            .unwrap();
        std::mem::forget(c); // killed
        assert_eq!(get(&fs, "pwm1_enable"), "1");
        let _again = Cooling::new(SysRoot::at(fs.root()), false, d.path());
        assert_eq!(get(&fs, "pwm1_enable"), "2", "restored at start");
    }

    #[test]
    fn a_lost_temperature_gives_the_fan_back_after_five_seconds() {
        let fs = TestFs::new();
        card(&fs, 60_000);
        let d = tempfile::tempdir().unwrap();
        let mut c = Cooling::new(SysRoot::at(fs.root()), false, d.path());
        c.set_curve("amdgpu/pwm1", vec![(40.0, 30.0), (80.0, 100.0)])
            .unwrap();
        std::fs::remove_file(fs.root().join(format!("{G}/temp2_input"))).unwrap();
        std::fs::remove_file(fs.root().join(format!("{G}/temp2_label"))).unwrap();
        for _ in 0..5 {
            c.tick();
        }
        assert_eq!(get(&fs, "pwm1_enable"), "2");
        assert!(!c.busy());
    }

    #[test]
    fn a_development_service_never_writes() {
        let fs = TestFs::new();
        card(&fs, 60_000);
        let d = tempfile::tempdir().unwrap();
        let mut c = Cooling::new(SysRoot::at(fs.root()), true, d.path());
        assert!(c
            .set_curve("amdgpu/pwm1", vec![(40.0, 30.0), (80.0, 100.0)])
            .is_err());
        assert_eq!(get(&fs, "pwm1_enable"), "2");
    }

    #[test]
    fn the_ideapad_fan_mode_is_offered_and_set() {
        let fs = TestFs::new();
        fs.write(IDEAPAD_FAN_MODE, "0");
        let d = tempfile::tempdir().unwrap();
        let mut c = Cooling::new(SysRoot::at(fs.root()), false, d.path());
        let st = c.state();
        assert_eq!(st.fan_mode, 0);
        assert_eq!(st.fan_modes.len(), 3);
        assert!(st.notes[0].contains("firmware"));
        c.set_fan_mode(4).unwrap();
        assert_eq!(
            std::fs::read_to_string(fs.root().join(IDEAPAD_FAN_MODE)).unwrap(),
            "4"
        );
        assert!(c.set_fan_mode(2).is_err(), "dust cleaning is not offered");
    }
}
