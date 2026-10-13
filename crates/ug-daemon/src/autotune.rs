//! Auto-tune: find a setting by measuring, not guessing.
//!
//! - **Faster**: raise the core clock offset in 30 MHz steps, each tested
//!   for 20 s; at the first failure, go back one step, then confirm the
//!   result with a 2-minute test (going back further if that fails).
//! - **Cooler**: lower the power limit in 5 % steps while the clock under
//!   the same load stays within 5 % of where it started; the last step that
//!   held is the answer: the same speed for less power and heat.
//!
//! The report compares the same load before and after: temperature, power
//! and clock, as measured, never estimated. The result is left on trial,
//! so it is still the user who keeps it.

use ug_proto::{GpuTune, GpuTuneState};

/// Averages over a stress run (NaN when the card does not report it).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Measure {
    pub temp_c: f64,
    pub power_w: f64,
    pub clock_mhz: f64,
}

pub trait Env {
    fn state(&mut self) -> Option<GpuTuneState>;
    fn try_tune(&mut self, t: GpuTune) -> Result<(), String>;
    /// Runs the stability test for `secs` on the setting being tried,
    /// measuring while it runs. Ok((stable, measure)).
    fn stress(&mut self, secs: u32) -> Result<(bool, Measure), String>;
    fn progress(&mut self, step: u32, steps: u32, message: String);
    fn cancelled(&self) -> bool;
}

#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub tune: GpuTune,
    pub before: Measure,
    pub after: Measure,
    pub summary: String,
}

const STEP_MHZ: i32 = 30;
const MAX_OFFSET: i32 = 300;
const QUICK: u32 = 20;
const CONFIRM: u32 = 120;

fn report(before: &Measure, after: &Measure) -> String {
    let d = |a: f64, b: f64| {
        if a.is_nan() || b.is_nan() {
            None
        } else {
            Some(b - a)
        }
    };
    let mut parts = Vec::new();
    if let Some(x) = d(before.temp_c, after.temp_c) {
        parts.push(format!("{x:+.0} °C"));
    }
    if let Some(x) = d(before.power_w, after.power_w) {
        parts.push(format!("{x:+.0} W"));
    }
    if let Some(x) = d(before.clock_mhz, after.clock_mhz) {
        parts.push(format!("{x:+.0} MHz clock"));
    }
    if parts.is_empty() {
        "The card does not report temperature, power or clock, so there is nothing to compare."
            .into()
    } else {
        format!("Under the same load: {}.", parts.join(", "))
    }
}

pub fn run(env: &mut dyn Env, goal: &str) -> Result<Outcome, String> {
    let st = env.state().ok_or("the GPU cannot be read")?;
    let start = st.applied;
    let base = GpuTune {
        power_limit_w: if start.power_limit_w == 0.0 {
            0.0
        } else {
            start.power_limit_w
        },
        ..start
    };
    env.progress(1, 3, "Measuring the card as it is…".into());
    env.try_tune(base)?;
    let (ok, before) = env.stress(QUICK)?;
    if !ok {
        return Err(
            "The card is not stable even as it is set now; reset it to its defaults first.".into(),
        );
    }
    match goal {
        "faster" => faster(env, &st, base, before),
        "cooler" => cooler(env, &st, base, before),
        _ => Err("goal is faster or cooler".into()),
    }
}

fn faster(
    env: &mut dyn Env,
    st: &GpuTuneState,
    base: GpuTune,
    before: Measure,
) -> Result<Outcome, String> {
    if !st.core_offset {
        return Err("this card does not allow a core clock offset".into());
    }
    let top = st.core_max_mhz.min(MAX_OFFSET);
    let steps = ((top - base.core_offset_mhz).max(0) / STEP_MHZ) as u32 + 2;
    let mut good = base.core_offset_mhz;
    let mut step = 1;
    let mut off = base.core_offset_mhz + STEP_MHZ;
    while off <= top {
        if env.cancelled() {
            return Err("cancelled".into());
        }
        step += 1;
        env.progress(step, steps, format!("Testing {off:+} MHz…"));
        env.try_tune(GpuTune {
            core_offset_mhz: off,
            ..base
        })?;
        let (ok, _) = env.stress(QUICK)?;
        if !ok {
            break;
        }
        good = off;
        off += STEP_MHZ;
    }
    // Confirm, stepping back while the long test fails.
    loop {
        env.progress(
            steps,
            steps,
            format!("Confirming {good:+} MHz for two minutes…"),
        );
        let t = GpuTune {
            core_offset_mhz: good,
            ..base
        };
        env.try_tune(t)?;
        let (ok, after) = env.stress(CONFIRM)?;
        if ok {
            let summary = if good == base.core_offset_mhz {
                format!(
                    "No higher clock was stable; it stays at {good:+} MHz. {}",
                    report(&before, &after)
                )
            } else {
                format!("Stable at {good:+} MHz. {}", report(&before, &after))
            };
            return Ok(Outcome {
                tune: t,
                before,
                after,
                summary,
            });
        }
        if good <= base.core_offset_mhz {
            return Err(
                "The card was not stable in the long test even without a higher clock.".into(),
            );
        }
        good = (good - STEP_MHZ).max(base.core_offset_mhz);
    }
}

fn cooler(
    env: &mut dyn Env,
    st: &GpuTuneState,
    base: GpuTune,
    before: Measure,
) -> Result<Outcome, String> {
    if !st.power_limit {
        return Err("this card does not allow a power limit".into());
    }
    if before.clock_mhz.is_nan() {
        return Err(
            "the card does not report its clock, so 'the same speed' cannot be measured".into(),
        );
    }
    let start_w = if base.power_limit_w == 0.0 {
        st.power_default_w
    } else {
        base.power_limit_w
    };
    let mut good = start_w;
    let mut w = start_w * 0.95;
    let steps = 8;
    let mut step = 1;
    while w >= st.power_min_w && step < steps {
        if env.cancelled() {
            return Err("cancelled".into());
        }
        step += 1;
        let pl = w.round();
        env.progress(step, steps, format!("Testing {pl:.0} W…"));
        env.try_tune(GpuTune {
            power_limit_w: pl,
            ..base
        })?;
        let (ok, m) = env.stress(QUICK)?;
        if !ok || m.clock_mhz < before.clock_mhz * 0.95 {
            break;
        }
        good = pl;
        w *= 0.95;
    }
    let t = GpuTune {
        power_limit_w: if (good - st.power_default_w).abs() < 0.5 {
            0.0
        } else {
            good
        },
        ..base
    };
    env.progress(
        steps,
        steps,
        format!("Confirming {good:.0} W for two minutes…"),
    );
    env.try_tune(t)?;
    let (ok, after) = env.stress(CONFIRM)?;
    if !ok {
        return Err("The long test failed at the lower power limit; nothing was changed.".into());
    }
    let summary = if good >= start_w {
        format!("Lowering the power limit cost speed at once; it stays at {good:.0} W.")
    } else {
        format!(
            "{good:.0} W keeps the same speed. {}",
            report(&before, &after)
        )
    };
    Ok(Outcome {
        tune: t,
        before,
        after,
        summary,
    })
}

/// The real thing: the tuner, the stability test, the sampler's readings.
pub struct Real {
    pub tuner: crate::gputune::SharedTuner,
    pub sampler: std::sync::Arc<crate::sampler::Sampler>,
    pub pci: String,
    pub status: std::sync::Arc<std::sync::Mutex<ug_proto::AutoTuneStatus>>,
    pub cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// A development daemon: the GPU is simulated, so is the load (the
    /// real card is never stressed by a test run).
    pub simulated: bool,
}

impl Env for Real {
    fn state(&mut self) -> Option<GpuTuneState> {
        self.tuner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .state(&self.pci)
    }

    fn try_tune(&mut self, t: GpuTune) -> Result<(), String> {
        self.tuner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .try_tune(&self.pci, t, std::time::Instant::now())
    }

    fn stress(&mut self, secs: u32) -> Result<(bool, Measure), String> {
        if self.simulated {
            std::thread::sleep(std::time::Duration::from_millis(200));
            let t = self.state().map(|s| s.applied).unwrap_or_default();
            let pl = if t.power_limit_w == 0.0 {
                60.0
            } else {
                t.power_limit_w
            };
            let _ = secs;
            return Ok((
                t.core_offset_mhz <= 120,
                Measure {
                    temp_c: 40.0 + pl * 0.5,
                    power_w: pl,
                    clock_mhz: if pl >= 48.0 { 1800.0 } else { 1500.0 } + t.core_offset_mhz as f64,
                },
            ));
        }
        let bin = crate::gputune::stress_binary()
            .ok_or("the stability test (ug-stress) is not installed")?;
        self.tuner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(&self.pci, std::time::Duration::from_secs(secs as u64 + 60));
        let mut child = std::process::Command::new(bin)
            .args(["--pci", &self.pci, "--seconds", &secs.to_string()])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("could not start the stability test: {e}"))?;
        // Read the card once a second while it works; the first quarter is
        // warm-up and left out of the averages.
        let started = std::time::Instant::now();
        let mut samples: Vec<(f64, f64, f64)> = Vec::new();
        let status = loop {
            if let Ok(Some(s)) = child.try_wait() {
                break s;
            }
            if self.cancel.load(std::sync::atomic::Ordering::SeqCst) {
                let _ = child.kill();
                let _ = child.wait();
                return Err("cancelled".into());
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
            if started.elapsed().as_secs_f64() > secs as f64 / 4.0 {
                if let Some(g) = self.sampler.gpu_backend(&self.pci, |b| b.telemetry()) {
                    samples.push((g.temp_c, g.power_w, g.core_clock_mhz as f64));
                }
            }
        };
        let avg = |f: fn(&(f64, f64, f64)) -> f64| {
            let v: Vec<f64> = samples
                .iter()
                .map(f)
                .filter(|x| !x.is_nan() && *x > 0.0)
                .collect();
            if v.is_empty() {
                f64::NAN
            } else {
                v.iter().sum::<f64>() / v.len() as f64
            }
        };
        let m = Measure {
            temp_c: avg(|s| s.0),
            power_w: avg(|s| s.1),
            clock_mhz: avg(|s| s.2),
        };
        match status.code() {
            Some(0) => Ok((true, m)),
            Some(2) => Ok((false, m)),
            _ => Err("the stability test could not run on this GPU".into()),
        }
    }

    fn progress(&mut self, step: u32, steps: u32, message: String) {
        let mut s = self.status.lock().unwrap_or_else(|e| e.into_inner());
        s.step = step;
        s.steps = steps;
        s.message = message;
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A card that is stable up to +120 MHz in short tests and +90 in long
    /// ones, and keeps full clock down to 48 W.
    struct Fake {
        tried: Vec<GpuTune>,
        now: GpuTune,
    }

    impl Env for Fake {
        fn state(&mut self) -> Option<GpuTuneState> {
            Some(GpuTuneState {
                power_limit: true,
                power_default_w: 60.0,
                power_min_w: 35.0,
                power_max_w: 95.0,
                core_offset: true,
                core_min_mhz: -200,
                core_max_mhz: 1000,
                ..Default::default()
            })
        }
        fn try_tune(&mut self, t: GpuTune) -> Result<(), String> {
            self.tried.push(t);
            self.now = t;
            Ok(())
        }
        fn stress(&mut self, secs: u32) -> Result<(bool, Measure), String> {
            let limit = if secs >= CONFIRM { 90 } else { 120 };
            let pl = if self.now.power_limit_w == 0.0 {
                60.0
            } else {
                self.now.power_limit_w
            };
            let clock = if pl >= 48.0 { 1800.0 } else { 1500.0 };
            Ok((
                self.now.core_offset_mhz <= limit,
                Measure {
                    temp_c: 40.0 + pl * 0.5,
                    power_w: pl,
                    clock_mhz: clock + self.now.core_offset_mhz as f64,
                },
            ))
        }
        fn progress(&mut self, _: u32, _: u32, _: String) {}
        fn cancelled(&self) -> bool {
            false
        }
    }

    #[test]
    fn faster_stops_at_the_first_failure_and_steps_back_for_the_long_test() {
        let mut f = Fake {
            tried: vec![],
            now: GpuTune::default(),
        };
        let o = run(&mut f, "faster").unwrap();
        assert_eq!(
            o.tune.core_offset_mhz, 90,
            "+120 passed quick tests, failed the long one; +90 held"
        );
        assert!(
            f.tried.iter().any(|t| t.core_offset_mhz == 150),
            "+150 was tried and failed"
        );
        assert!(
            f.tried.iter().all(|t| t.core_offset_mhz <= 150),
            "never went past the first failure"
        );
        assert!(o.summary.contains("+90 MHz"));
        assert!(o.summary.contains("MHz clock"), "{}", o.summary);
    }

    #[test]
    fn cooler_lowers_power_while_the_clock_holds() {
        let mut f = Fake {
            tried: vec![],
            now: GpuTune::default(),
        };
        let o = run(&mut f, "cooler").unwrap();
        assert_eq!(
            o.tune.power_limit_w, 49.0,
            "60→57→54→51→49 held, 46 lost clock"
        );
        assert!(o.summary.contains("-11 W"), "{}", o.summary);
        assert!(o.after.temp_c < o.before.temp_c);
    }

    #[test]
    fn an_unstable_start_is_refused() {
        struct Bad;
        impl Env for Bad {
            fn state(&mut self) -> Option<GpuTuneState> {
                Some(GpuTuneState {
                    core_offset: true,
                    core_max_mhz: 300,
                    ..Default::default()
                })
            }
            fn try_tune(&mut self, _: GpuTune) -> Result<(), String> {
                Ok(())
            }
            fn stress(&mut self, _: u32) -> Result<(bool, Measure), String> {
                Ok((false, Measure::default()))
            }
            fn progress(&mut self, _: u32, _: u32, _: String) {}
            fn cancelled(&self) -> bool {
                false
            }
        }
        assert!(run(&mut Bad, "faster").unwrap_err().contains("not stable"));
    }
}
