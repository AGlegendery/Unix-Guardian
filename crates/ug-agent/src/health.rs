//! What the tray icon's colour says: normal, warning, critical, game mode or
//! offline, decided from one reading every couple of seconds.
//!
//! Entering a state is quick (a hot reading has to last a few seconds, so a
//! compile burst does not flash the icon); leaving one waits until things
//! have been calm for a while, so the icon does not flicker around a
//! threshold.

use serde::Deserialize;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Calm,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconState {
    Normal,
    Warning,
    Critical,
    GameMode,
    Offline,
}

impl IconState {
    pub fn name(self) -> &'static str {
        match self {
            IconState::Normal => "normal",
            IconState::Warning => "warning",
            IconState::Critical => "critical",
            IconState::GameMode => "gamemode",
            IconState::Offline => "offline",
        }
    }
}

/// The limits, from `~/.config/unix-guardian/tray.json` when present.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Thresholds {
    pub warn_temp_c: f64,
    pub warn_temp_exit_c: f64,
    pub warn_temp_hold_s: u64,
    pub warn_usage_pct: f64,
    pub warn_usage_exit_pct: f64,
    pub warn_usage_hold_s: u64,
    pub warn_ram_pct: f64,
    pub warn_ram_exit_pct: f64,
    pub warn_vram_pct: f64,
    pub crit_temp_c: f64,
    /// Critical this close to the sensor's own critical point (Intel's TjMax).
    pub crit_margin_c: f64,
    pub crit_temp_exit_c: f64,
    pub crit_ram_pct: f64,
    /// Calm this long before the icon steps down.
    pub leave_after_s: u64,
    /// The service unreachable this long before the icon says offline.
    pub offline_after_s: u64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds {
            warn_temp_c: 80.0,
            warn_temp_exit_c: 75.0,
            warn_temp_hold_s: 5,
            warn_usage_pct: 90.0,
            warn_usage_exit_pct: 80.0,
            warn_usage_hold_s: 10,
            warn_ram_pct: 90.0,
            warn_ram_exit_pct: 85.0,
            warn_vram_pct: 95.0,
            crit_temp_c: 90.0,
            crit_margin_c: 5.0,
            crit_temp_exit_c: 85.0,
            crit_ram_pct: 97.0,
            leave_after_s: 10,
            offline_after_s: 3,
        }
    }
}

impl Thresholds {
    pub fn load(path: &std::path::Path) -> Thresholds {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!(path = %path.display(), error = %e, "ignoring tray thresholds");
                Thresholds::default()
            }),
            Err(_) => Thresholds::default(),
        }
    }
}

/// One temperature, with the sensor's own critical point when it has one.
#[derive(Debug, Clone, PartialEq)]
pub struct Temp {
    pub what: String,
    pub c: f64,
    pub crit_c: Option<f64>,
}

/// One reading of everything the icon depends on. NaN where unknown.
#[derive(Debug, Clone, Default)]
pub struct Reading {
    pub temps: Vec<Temp>,
    /// (what, percent): CPU and each awake GPU.
    pub usage: Vec<(String, f64)>,
    pub ram_pct: f64,
    /// (GPU, percent of its memory used).
    pub vram: Vec<(String, f64)>,
    /// The CPU throttled for heat since the last reading.
    pub throttling: bool,
    /// Swapping hard while memory is nearly full.
    pub thrashing: bool,
    pub gaming: bool,
    /// The service answered this time.
    pub daemon_ok: bool,
}

pub struct Health {
    pub limits: Thresholds,
    level: Level,
    reasons: Vec<String>,
    hot_since: Option<Instant>,
    busy_since: Option<Instant>,
    calm_since: Option<Instant>,
    down_since: Option<Instant>,
}

impl Health {
    pub fn new(limits: Thresholds) -> Health {
        Health {
            limits,
            level: Level::Calm,
            reasons: Vec::new(),
            hot_since: None,
            busy_since: None,
            calm_since: None,
            down_since: None,
        }
    }

    /// Why the icon is not normal, for the tooltip; empty when it is.
    pub fn reasons(&self) -> &[String] {
        &self.reasons
    }

    pub fn update(&mut self, now: Instant, r: &Reading) -> IconState {
        let l = self.limits.clone();
        let secs = |s: u64| Duration::from_secs(s);

        // Critical: no waiting.
        let mut crit = Vec::new();
        for t in &r.temps {
            let near_crit = t.crit_c.is_some_and(|c| t.c >= c - l.crit_margin_c);
            if t.c >= l.crit_temp_c || near_crit {
                crit.push(format!("{} {:.0} °C", t.what, t.c));
            }
        }
        if r.throttling {
            crit.push("CPU throttling for heat".into());
        }
        if r.ram_pct >= l.crit_ram_pct {
            crit.push(format!("RAM {:.0} %", r.ram_pct));
        } else if r.thrashing {
            crit.push("RAM full, swapping hard".into());
        }

        // Warning: heat and load have to last a few seconds.
        let mut warn = Vec::new();
        let hot: Vec<&Temp> = r.temps.iter().filter(|t| t.c >= l.warn_temp_c).collect();
        if hot.is_empty() {
            self.hot_since = None;
        } else if now.duration_since(*self.hot_since.get_or_insert(now)) >= secs(l.warn_temp_hold_s)
        {
            warn.extend(hot.iter().map(|t| format!("{} {:.0} °C", t.what, t.c)));
        }
        let busy: Vec<&(String, f64)> =
            r.usage.iter().filter(|u| u.1 >= l.warn_usage_pct).collect();
        if busy.is_empty() {
            self.busy_since = None;
        } else if now.duration_since(*self.busy_since.get_or_insert(now))
            >= secs(l.warn_usage_hold_s)
        {
            warn.extend(busy.iter().map(|u| format!("{} {:.0} % busy", u.0, u.1)));
        }
        if r.ram_pct >= l.warn_ram_pct {
            warn.push(format!("RAM {:.0} %", r.ram_pct));
        }
        for (what, pct) in &r.vram {
            if *pct >= l.warn_vram_pct {
                warn.push(format!("{what} memory {pct:.0} %"));
            }
        }

        let (raw, reasons) = if !crit.is_empty() {
            (Level::Critical, crit)
        } else if !warn.is_empty() {
            (Level::Warning, warn)
        } else {
            (Level::Calm, Vec::new())
        };
        if raw >= self.level {
            self.level = raw;
            self.reasons = reasons;
            self.calm_since = None;
        } else {
            let max_temp = r
                .temps
                .iter()
                .map(|t| t.c)
                .fold(f64::NEG_INFINITY, f64::max);
            let calm = match self.level {
                Level::Critical => {
                    max_temp < l.crit_temp_exit_c
                        && !r.throttling
                        && r.ram_pct < l.crit_ram_pct
                        && !r.thrashing
                }
                Level::Warning => {
                    max_temp < l.warn_temp_exit_c
                        && r.usage.iter().all(|u| u.1 < l.warn_usage_exit_pct)
                        && r.ram_pct < l.warn_ram_exit_pct
                }
                Level::Calm => true,
            };
            if !calm {
                self.calm_since = None;
            } else if now.duration_since(*self.calm_since.get_or_insert(now))
                >= secs(l.leave_after_s)
            {
                self.level = raw;
                self.reasons = reasons;
                self.calm_since = None;
            }
        }

        if r.daemon_ok {
            self.down_since = None;
        } else if now.duration_since(*self.down_since.get_or_insert(now)) >= secs(l.offline_after_s)
        {
            return IconState::Offline;
        }
        match self.level {
            Level::Critical => IconState::Critical,
            Level::Warning => IconState::Warning,
            Level::Calm if r.gaming => IconState::GameMode,
            Level::Calm => IconState::Normal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(cpu_temp: f64, cpu_pct: f64, ram: f64) -> Reading {
        Reading {
            temps: vec![Temp {
                what: "CPU".into(),
                c: cpu_temp,
                crit_c: Some(100.0),
            }],
            usage: vec![("CPU".into(), cpu_pct)],
            ram_pct: ram,
            daemon_ok: true,
            ..Default::default()
        }
    }

    /// Feeds the same reading every 2 s for `secs`, returns the last state.
    fn run(h: &mut Health, t: &mut Instant, secs: u64, r: &Reading) -> IconState {
        let mut s = h.update(*t, r);
        for _ in 0..secs / 2 {
            *t += Duration::from_secs(2);
            s = h.update(*t, r);
        }
        s
    }

    #[test]
    fn heat_must_last_before_warning_and_calm_must_last_before_normal() {
        let mut h = Health::new(Thresholds::default());
        let mut t = Instant::now();
        assert_eq!(h.update(t, &reading(50.0, 10.0, 40.0)), IconState::Normal);
        assert_eq!(
            run(&mut h, &mut t, 2, &reading(82.0, 10.0, 40.0)),
            IconState::Normal,
            "2 s of heat"
        );
        assert_eq!(
            run(&mut h, &mut t, 4, &reading(82.0, 10.0, 40.0)),
            IconState::Warning
        );
        assert_eq!(h.reasons(), ["CPU 82 °C"]);
        // 77 °C is below the warning but not below the exit point.
        assert_eq!(
            run(&mut h, &mut t, 30, &reading(77.0, 10.0, 40.0)),
            IconState::Warning
        );
        assert_eq!(
            run(&mut h, &mut t, 8, &reading(70.0, 10.0, 40.0)),
            IconState::Warning,
            "8 s calm"
        );
        assert_eq!(
            run(&mut h, &mut t, 4, &reading(70.0, 10.0, 40.0)),
            IconState::Normal
        );
        assert!(h.reasons().is_empty());
    }

    #[test]
    fn critical_is_immediate_and_beats_game_mode() {
        let mut h = Health::new(Thresholds::default());
        let t = Instant::now();
        let mut r = reading(91.0, 10.0, 40.0);
        r.gaming = true;
        assert_eq!(h.update(t, &r), IconState::Critical);
        let near = Reading {
            temps: vec![Temp {
                what: "GPU".into(),
                c: 86.0,
                crit_c: Some(90.0),
            }],
            ..reading(50.0, 1.0, 1.0)
        };
        let mut h = Health::new(Thresholds::default());
        assert_eq!(
            h.update(t, &near),
            IconState::Critical,
            "within 5 °C of the sensor's own limit"
        );
        let mut h = Health::new(Thresholds::default());
        assert_eq!(
            h.update(
                t,
                &Reading {
                    throttling: true,
                    ..reading(70.0, 1.0, 1.0)
                }
            ),
            IconState::Critical
        );
    }

    #[test]
    fn critical_steps_down_to_warning_while_still_warm() {
        let mut h = Health::new(Thresholds::default());
        let mut t = Instant::now();
        assert_eq!(
            run(&mut h, &mut t, 6, &reading(92.0, 10.0, 40.0)),
            IconState::Critical
        );
        assert_eq!(
            run(&mut h, &mut t, 6, &reading(87.0, 10.0, 40.0)),
            IconState::Critical,
            "87 is above the exit point"
        );
        assert_eq!(
            run(&mut h, &mut t, 12, &reading(82.0, 10.0, 40.0)),
            IconState::Warning
        );
    }

    #[test]
    fn load_ram_and_game_mode() {
        let mut h = Health::new(Thresholds::default());
        let mut t = Instant::now();
        assert_eq!(
            run(&mut h, &mut t, 8, &reading(60.0, 95.0, 40.0)),
            IconState::Normal,
            "8 s of load"
        );
        assert_eq!(
            run(&mut h, &mut t, 2, &reading(60.0, 95.0, 40.0)),
            IconState::Warning
        );
        let mut h = Health::new(Thresholds::default());
        assert_eq!(h.update(t, &reading(60.0, 10.0, 92.0)), IconState::Warning);
        let mut h = Health::new(Thresholds::default());
        assert_eq!(
            h.update(
                t,
                &Reading {
                    gaming: true,
                    ..reading(60.0, 10.0, 40.0)
                }
            ),
            IconState::GameMode
        );
    }

    #[test]
    fn offline_after_three_seconds_without_the_service() {
        let mut h = Health::new(Thresholds::default());
        let mut t = Instant::now();
        let down = Reading {
            daemon_ok: false,
            ..reading(60.0, 10.0, 40.0)
        };
        assert_eq!(h.update(t, &down), IconState::Normal);
        t += Duration::from_secs(2);
        assert_eq!(h.update(t, &down), IconState::Normal);
        t += Duration::from_secs(2);
        assert_eq!(h.update(t, &down), IconState::Offline);
        assert_eq!(h.update(t, &reading(60.0, 10.0, 40.0)), IconState::Normal);
    }

    #[test]
    fn thresholds_file_overrides_some_values() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("tray.json");
        std::fs::write(&p, r#"{"warn_temp_c": 85}"#).unwrap();
        let l = Thresholds::load(&p);
        assert_eq!((l.warn_temp_c, l.crit_temp_c), (85.0, 90.0));
        assert_eq!(
            Thresholds::load(&dir.path().join("none.json")).warn_temp_c,
            80.0
        );
    }
}
