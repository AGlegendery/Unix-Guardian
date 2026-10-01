//! GPU power limit and clock offsets, applied so that a bad setting cannot
//! trap the user.
//!
//! Three guards:
//!
//! 1. **Try, then keep.** A new setting is a trial: it is undone by itself
//!    after 30 seconds unless the user confirms it (as a screen-resolution
//!    change is), so a setting that freezes the display undoes itself.
//! 2. **Crash watchdog.** Before a trial is applied it is written to
//!    `gpu-pending.json`. If the daemon starts and finds that file, the
//!    machine or the service died during the trial: the setting is *not*
//!    applied again, and the user is told.
//! 3. **Temperature guard.** While a setting is in force and the GPU is
//!    awake, its temperature is checked every second; at 88 °C the card's
//!    defaults come back at once.
//!
//! Kept settings live in `gpu-tune.json`. A laptop's discrete GPU forgets
//! them when it powers down, so they are applied again each time it wakes —
//! and only then: the daemon never wakes a sleeping GPU to tune it.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ug_proto::{GpuTune, GpuTuneState};

pub const TRIAL: Duration = Duration::from_secs(30);
/// NVIDIA's own slowdown starts in the high 80s; this is the line Unix
/// Guardian will not hold a tuned card past.
pub const HOT_C: f64 = 88.0;

/// The hardware side, so the logic is tested without a GPU.
pub trait Control: Send + Sync {
    fn state(&self, pci: &str) -> Option<GpuTuneState>;
    fn apply(&self, pci: &str, t: &GpuTune) -> Result<(), String>;
    fn reset(&self, pci: &str) -> Result<(), String>;
    /// Awake (not runtime-suspended); reading it never wakes the GPU.
    fn awake(&self, pci: &str) -> bool;
    /// Core temperature, only asked while awake; NaN when unknown.
    fn temp(&self, pci: &str) -> f64;
}

/// The real GPUs, through the sampler's backends.
pub struct Real {
    pub sampler: Arc<crate::sampler::Sampler>,
    pub root: ug_hw::SysRoot,
}

impl Control for Real {
    fn state(&self, pci: &str) -> Option<GpuTuneState> {
        self.sampler.gpu_backend(pci, |b| b.tune_state())
    }
    fn apply(&self, pci: &str, t: &GpuTune) -> Result<(), String> {
        self.sampler.gpu_backend(pci, |b| b.apply_tune(t)).unwrap_or_else(|| Err(format!("no GPU at {pci}")))
    }
    fn reset(&self, pci: &str) -> Result<(), String> {
        self.sampler.gpu_backend(pci, |b| b.reset_tune()).unwrap_or_else(|| Err(format!("no GPU at {pci}")))
    }
    fn awake(&self, pci: &str) -> bool {
        !ug_gpu::pci::runtime_suspended(&self.root, pci)
    }
    fn temp(&self, pci: &str) -> f64 {
        self.sampler.gpu_backend(pci, |b| b.telemetry().temp_c).unwrap_or(f64::NAN)
    }
}

/// A development daemon's GPU: believable limits, nothing touched.
pub struct Simulated {
    pub cards: Mutex<BTreeMap<String, GpuTuneState>>,
    pub temp: Mutex<f64>,
}

impl Simulated {
    pub fn new(pcis: &[String]) -> Simulated {
        let cards = pcis
            .iter()
            .map(|p| {
                (
                    p.clone(),
                    GpuTuneState {
                        pci: p.clone(),
                        power_limit: true,
                        power_limit_w: 60.0,
                        power_default_w: 60.0,
                        power_min_w: 35.0,
                        power_max_w: 95.0,
                        core_offset: true,
                        core_min_mhz: -200,
                        core_max_mhz: 300,
                        mem_offset: true,
                        mem_min_mhz: -500,
                        mem_max_mhz: 1000,
                        notes: vec!["Development service: a simulated GPU, nothing is changed.".into()],
                        ..Default::default()
                    },
                )
            })
            .collect();
        Simulated { cards: Mutex::new(cards), temp: Mutex::new(55.0) }
    }
}

impl Control for Simulated {
    fn state(&self, pci: &str) -> Option<GpuTuneState> {
        self.cards.lock().unwrap_or_else(|e| e.into_inner()).get(pci).cloned()
    }
    fn apply(&self, pci: &str, t: &GpuTune) -> Result<(), String> {
        let mut cards = self.cards.lock().unwrap_or_else(|e| e.into_inner());
        let s = cards.get_mut(pci).ok_or("no such GPU")?;
        ug_gpu::check_tune(t, s)?;
        s.power_limit_w = if t.power_limit_w == 0.0 { s.power_default_w } else { t.power_limit_w };
        s.core_offset_mhz = t.core_offset_mhz;
        s.mem_offset_mhz = t.mem_offset_mhz;
        Ok(())
    }
    fn reset(&self, pci: &str) -> Result<(), String> {
        self.apply(pci, &GpuTune::default())
    }
    fn awake(&self, _pci: &str) -> bool {
        true
    }
    fn temp(&self, _pci: &str) -> f64 {
        *self.temp.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Kept {
    #[serde(default)]
    gpus: BTreeMap<String, (f64, i32, i32)>,
}

#[derive(Serialize, Deserialize)]
struct Pending {
    pci: String,
    tune: (f64, i32, i32),
}

fn to_tuple(t: &GpuTune) -> (f64, i32, i32) {
    (t.power_limit_w, t.core_offset_mhz, t.mem_offset_mhz)
}

fn from_tuple(t: (f64, i32, i32)) -> GpuTune {
    GpuTune { power_limit_w: t.0, core_offset_mhz: t.1, mem_offset_mhz: t.2 }
}

struct Trial {
    pci: String,
    tune: GpuTune,
    until: Instant,
    previous: GpuTune,
}

pub struct Tuner {
    control: Box<dyn Control>,
    kept_path: PathBuf,
    pending_path: PathBuf,
    applied: BTreeMap<String, GpuTune>,
    crashed: BTreeMap<String, GpuTune>,
    /// Why a setting was taken back, shown until the next change.
    reverted: BTreeMap<String, String>,
    trial: Option<Trial>,
    /// GPUs that were awake at the last tick (a wake means: apply again).
    awake: BTreeMap<String, bool>,
}

pub type SharedTuner = Arc<Mutex<Tuner>>;

impl Tuner {
    pub fn new(control: Box<dyn Control>, state_dir: &Path) -> Tuner {
        let kept_path = state_dir.join("gpu-tune.json");
        let pending_path = state_dir.join("gpu-pending.json");
        let kept: Kept = std::fs::read_to_string(&kept_path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let mut crashed = BTreeMap::new();
        if let Some(p) = std::fs::read_to_string(&pending_path).ok().and_then(|s| serde_json::from_str::<Pending>(&s).ok()) {
            tracing::warn!(pci = %p.pci, "a GPU setting was being tried when the machine or the service stopped; it is not applied again");
            crashed.insert(p.pci, from_tuple(p.tune));
        }
        let _ = std::fs::remove_file(&pending_path);
        Tuner {
            control,
            kept_path,
            pending_path,
            applied: kept.gpus.into_iter().map(|(k, v)| (k, from_tuple(v))).collect(),
            crashed,
            reverted: BTreeMap::new(),
            trial: None,
            awake: BTreeMap::new(),
        }
    }

    fn save(&self) {
        let kept = Kept { gpus: self.applied.iter().map(|(k, v)| (k.clone(), to_tuple(v))).collect() };
        if let Err(e) = ug_core::paths::write_atomic(&self.kept_path, serde_json::to_string_pretty(&kept).expect("serialize").as_bytes()) {
            tracing::error!(error = %e, "could not save GPU settings");
        }
    }

    pub fn state(&self, pci: &str) -> Option<GpuTuneState> {
        let mut s = self.control.state(pci)?;
        s.applied = self.applied.get(pci).copied().unwrap_or_default();
        if let Some(t) = self.trial.as_ref().filter(|t| t.pci == pci) {
            s.applied = t.tune;
            s.notes.insert(0, "Trying: keep it within 30 seconds, or it is undone by itself.".into());
        }
        s.crashed = self.crashed.get(pci).copied().unwrap_or_default();
        if let Some(why) = self.reverted.get(pci) {
            s.notes.insert(0, why.clone());
        }
        Some(s)
    }

    /// Applies `t` on trial: undone after [`TRIAL`] unless confirmed.
    pub fn try_tune(&mut self, pci: &str, t: GpuTune, now: Instant) -> Result<(), String> {
        let st = self.control.state(pci).ok_or_else(|| format!("no GPU at {pci}"))?;
        ug_gpu::check_tune(&t, &st)?;
        // A trial still running is replaced; its "previous" is what stays.
        let previous = match self.trial.take() {
            Some(old) if old.pci == pci => old.previous,
            Some(old) => {
                self.undo_trial(old, "Undone: another GPU setting was tried before this one was kept.");
                self.applied.get(pci).copied().unwrap_or_default()
            }
            None => self.applied.get(pci).copied().unwrap_or_default(),
        };
        let pending = Pending { pci: pci.to_string(), tune: to_tuple(&t) };
        ug_core::paths::write_atomic(&self.pending_path, serde_json::to_string(&pending).expect("serialize").as_bytes())
            .map_err(|e| format!("could not record the trial, so nothing was changed: {e}"))?;
        if let Err(e) = self.control.apply(pci, &t) {
            let _ = std::fs::remove_file(&self.pending_path);
            let _ = self.control.apply(pci, &previous);
            return Err(e);
        }
        self.reverted.remove(pci);
        self.crashed.remove(pci);
        self.trial = Some(Trial { pci: pci.to_string(), tune: t, until: now + TRIAL, previous });
        tracing::info!(pci, ?t, "GPU setting on trial");
        Ok(())
    }

    /// Keeps the setting on trial.
    pub fn confirm(&mut self, pci: &str) -> Result<(), String> {
        let t = self.trial.take_if(|t| t.pci == pci).ok_or("nothing is being tried on this GPU")?;
        if t.tune.is_default() {
            self.applied.remove(pci);
        } else {
            self.applied.insert(pci.to_string(), t.tune);
        }
        self.save();
        let _ = std::fs::remove_file(&self.pending_path);
        tracing::info!(pci, tune = ?t.tune, "GPU setting kept");
        Ok(())
    }

    /// Undoes the setting on trial now, back to what was kept before.
    pub fn cancel(&mut self, pci: &str) -> Result<(), String> {
        let t = self.trial.take_if(|t| t.pci == pci).ok_or("nothing is being tried on this GPU")?;
        self.undo_trial(t, "Undone.");
        Ok(())
    }

    /// The card's own defaults, now and from now on.
    pub fn reset(&mut self, pci: &str) -> Result<(), String> {
        if self.trial.as_ref().is_some_and(|t| t.pci == pci) {
            self.trial = None;
            let _ = std::fs::remove_file(&self.pending_path);
        }
        self.applied.remove(pci);
        self.crashed.remove(pci);
        self.reverted.remove(pci);
        self.save();
        self.control.reset(pci)
    }

    fn undo_trial(&mut self, t: Trial, why: &str) {
        if let Err(e) = self.control.apply(&t.pci, &t.previous) {
            tracing::error!(pci = %t.pci, error = %e, "could not undo a GPU trial; resetting");
            let _ = self.control.reset(&t.pci);
        }
        let _ = std::fs::remove_file(&self.pending_path);
        self.reverted.insert(t.pci.clone(), why.into());
        tracing::info!(pci = %t.pci, why, "GPU trial undone");
    }

    /// Once a second: ends expired trials, re-applies kept settings when a
    /// GPU wakes, and guards the temperature of tuned GPUs.
    pub fn tick(&mut self, now: Instant) {
        if let Some(t) = self.trial.take() {
            if now >= t.until {
                self.undo_trial(t, "Undone: the setting was not kept within 30 seconds.");
            } else {
                self.trial = Some(t);
            }
        }
        let tuned: Vec<String> = self.applied.keys().cloned().chain(self.trial.iter().map(|t| t.pci.clone())).collect();
        for pci in tuned {
            let awake = self.control.awake(&pci);
            let was = self.awake.insert(pci.clone(), awake).unwrap_or(false);
            if !awake {
                continue;
            }
            if !was {
                if let Some(t) = self.applied.get(&pci).copied().filter(|_| self.trial.as_ref().is_none_or(|x| x.pci != pci)) {
                    match self.control.apply(&pci, &t) {
                        Ok(()) => tracing::info!(pci, "GPU woke; kept setting applied again"),
                        Err(e) => tracing::warn!(pci, error = %e, "could not apply the kept GPU setting"),
                    }
                }
            }
            let c = self.control.temp(&pci);
            if c >= HOT_C {
                tracing::warn!(pci, temp = c, "tuned GPU too hot; defaults restored");
                self.trial = self.trial.take().filter(|t| t.pci != pci);
                let _ = std::fs::remove_file(&self.pending_path);
                self.applied.remove(&pci);
                self.save();
                let _ = self.control.reset(&pci);
                self.reverted.insert(pci.clone(), format!("Undone: the card reached {c:.0} °C, so its own defaults were restored."));
            }
        }
    }

    pub fn busy(&self) -> bool {
        self.trial.is_some() || !self.applied.is_empty()
    }
}

/// Ticks once a second while anything is tuned, otherwise every 10 s.
pub async fn run(tuner: SharedTuner) {
    loop {
        let t = Arc::clone(&tuner);
        let busy = tokio::task::spawn_blocking(move || {
            let mut t = t.lock().unwrap_or_else(|e| e.into_inner());
            t.tick(Instant::now());
            t.busy()
        })
        .await
        .unwrap_or(false);
        tokio::time::sleep(if busy { Duration::from_secs(1) } else { Duration::from_secs(10) }).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GPU: &str = "0000:01:00.0";

    fn tuner(dir: &Path) -> (Tuner, Arc<Simulated>) {
        let sim = Arc::new(Simulated::new(&[GPU.to_string()]));
        struct Shared(Arc<Simulated>);
        impl Control for Shared {
            fn state(&self, p: &str) -> Option<GpuTuneState> {
                self.0.state(p)
            }
            fn apply(&self, p: &str, t: &GpuTune) -> Result<(), String> {
                self.0.apply(p, t)
            }
            fn reset(&self, p: &str) -> Result<(), String> {
                self.0.reset(p)
            }
            fn awake(&self, p: &str) -> bool {
                self.0.awake(p)
            }
            fn temp(&self, p: &str) -> f64 {
                self.0.temp(p)
            }
        }
        (Tuner::new(Box::new(Shared(Arc::clone(&sim))), dir), sim)
    }

    fn tune(pl: f64, core: i32) -> GpuTune {
        GpuTune { power_limit_w: pl, core_offset_mhz: core, mem_offset_mhz: 0 }
    }

    #[test]
    fn an_unconfirmed_trial_undoes_itself() {
        let d = tempfile::tempdir().unwrap();
        let (mut t, sim) = tuner(d.path());
        let now = Instant::now();
        t.try_tune(GPU, tune(45.0, 100), now).unwrap();
        assert_eq!(sim.state(GPU).unwrap().power_limit_w, 45.0);
        t.tick(now + Duration::from_secs(10));
        assert_eq!(sim.state(GPU).unwrap().core_offset_mhz, 100, "still trying");
        t.tick(now + TRIAL + Duration::from_secs(1));
        let s = sim.state(GPU).unwrap();
        assert_eq!((s.power_limit_w, s.core_offset_mhz), (60.0, 0), "back to the card's defaults");
        assert!(t.state(GPU).unwrap().notes[0].contains("not kept"));
        assert!(!d.path().join("gpu-pending.json").exists());
    }

    #[test]
    fn a_kept_setting_survives_a_restart_and_is_applied_when_the_gpu_wakes() {
        let d = tempfile::tempdir().unwrap();
        let (mut t, _) = tuner(d.path());
        let now = Instant::now();
        t.try_tune(GPU, tune(50.0, 150), now).unwrap();
        t.confirm(GPU).unwrap();
        t.tick(now + TRIAL * 2);
        drop(t);
        let (mut again, sim) = tuner(d.path());
        assert_eq!(sim.state(GPU).unwrap().power_limit_w, 60.0, "a fresh card has its defaults");
        again.tick(Instant::now());
        let s = sim.state(GPU).unwrap();
        assert_eq!((s.power_limit_w, s.core_offset_mhz), (50.0, 150), "applied when seen awake");
        assert_eq!(again.state(GPU).unwrap().applied, tune(50.0, 150));
    }

    #[test]
    fn a_crash_during_a_trial_is_not_repeated_on_the_next_start() {
        let d = tempfile::tempdir().unwrap();
        let (mut t, _) = tuner(d.path());
        t.try_tune(GPU, tune(95.0, 300), Instant::now()).unwrap();
        std::mem::forget(t); // the machine froze before the user kept it
        let (mut again, sim) = tuner(d.path());
        again.tick(Instant::now());
        let s = sim.state(GPU).unwrap();
        assert_eq!((s.power_limit_w, s.core_offset_mhz), (60.0, 0), "not applied again");
        assert_eq!(again.state(GPU).unwrap().crashed, tune(95.0, 300), "and the user is told what it was");
    }

    #[test]
    fn a_hot_tuned_gpu_gets_its_defaults_back() {
        let d = tempfile::tempdir().unwrap();
        let (mut t, sim) = tuner(d.path());
        t.try_tune(GPU, tune(90.0, 200), Instant::now()).unwrap();
        t.confirm(GPU).unwrap();
        *sim.temp.lock().unwrap() = 91.0;
        t.tick(Instant::now());
        let s = sim.state(GPU).unwrap();
        assert_eq!((s.power_limit_w, s.core_offset_mhz), (60.0, 0));
        assert!(t.state(GPU).unwrap().notes[0].contains("91"));
        assert_eq!(t.state(GPU).unwrap().applied, GpuTune::default(), "and it stays off");
    }

    #[test]
    fn out_of_range_requests_change_nothing() {
        let d = tempfile::tempdir().unwrap();
        let (mut t, sim) = tuner(d.path());
        let e = t.try_tune(GPU, tune(200.0, 0), Instant::now()).unwrap_err();
        assert!(e.contains("35–95 W"), "{e}");
        assert!(t.try_tune(GPU, tune(0.0, 900), Instant::now()).is_err());
        assert_eq!(sim.state(GPU).unwrap().power_limit_w, 60.0);
        assert!(!d.path().join("gpu-pending.json").exists());
    }
}
