//! Charge and use over a span, and three independent looks at battery health.
//!
//! Everything comes from the once-a-minute history (charge %, battery watts,
//! on the charger, screen on) plus the battery's own capacity figures. The
//! three health methods disagree in useful ways, so all are shown with what
//! each rests on:
//!
//! - **fuel gauge**: `energy_full / energy_full_design`, what UPower and
//!   every desktop show. It is the battery's own estimate and is relearnt
//!   only on full cycles, so it can lag or flatter.
//! - **measured**: energy we watched the battery give on discharge
//!   (watts × minutes), divided by the charge it cost. Independent of the
//!   gauge's capacity figure, but the percentage is still the gauge's, and
//!   short runs magnify its 1-point steps; only runs of 10 points or more
//!   count, and the confidence grows with the points covered.
//! - **cycles**: where the cycle count puts a typical laptop cell (about
//!   80 % after 500–1000 cycles). A range, not a measurement.
//!
//! Internal resistance (voltage sag under load) also tracks ageing, but
//! needs a baseline from when the battery was new, which nobody has.

use ug_hw::battery::BatteryInfo;
use ug_proto::{BatteryReport, HealthEstimate, PowerPeriod};

/// One history row.
#[derive(Debug, Clone, Copy, Default)]
pub struct Row {
    pub ts_ms: u64,
    pub pct: Option<f64>,
    /// Positive while draining, negative while charging.
    pub w: Option<f64>,
    pub on_ac: Option<bool>,
    pub screen: Option<bool>,
}

/// Readings further apart than this mean the computer was off or asleep.
const GAP_MS: u64 = 3 * 60_000;
/// What one reading stands for at most.
const STEP_MS: u64 = 60_000;
/// Discharge runs shorter than this many points are too coarse to measure.
const MIN_RUN_PCT: f64 = 10.0;

fn on_ac(r: &Row) -> bool {
    match (r.on_ac, r.w) {
        (Some(ac), _) => ac,
        // Older rows have no charger column: draining means on battery.
        (None, Some(w)) => w <= 0.3,
        (None, None) => true,
    }
}

/// How long each row stands for: until the next one, at most a minute.
fn spans(rows: &[Row]) -> Vec<u64> {
    rows.iter()
        .enumerate()
        .map(|(i, r)| match rows.get(i + 1) {
            Some(n) if n.ts_ms - r.ts_ms <= GAP_MS => (n.ts_ms - r.ts_ms).min(STEP_MS),
            _ => STEP_MS,
        })
        .collect()
}

pub fn report(rows: &[Row], info: Option<&BatteryInfo>, health_rows: &[Row]) -> BatteryReport {
    let mut out = BatteryReport {
        has_battery: info.is_some() || rows.iter().any(|r| r.pct.is_some()),
        design_wh: info.and_then(|i| i.design_wh).unwrap_or(f64::NAN),
        full_wh: info.and_then(|i| i.full_wh).unwrap_or(f64::NAN),
        cycles: info.and_then(|i| i.cycles).unwrap_or(0),
        avg_battery_w: f64::NAN,
        ..Default::default()
    };
    if !out.has_battery {
        return out;
    }
    let dts = spans(rows);
    let (mut bat_ms, mut ac_ms, mut screen_bat_ms) = (0u64, 0u64, 0u64);
    let (mut used_wms, mut bat_w_ms) = (0f64, 0u64);
    // Screen-on time per period, in ms until the end.
    let mut screen_ms: Vec<u64> = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let dt = dts[i];
        out.points.push((
            r.ts_ms,
            r.pct.unwrap_or(f64::NAN),
            r.w.unwrap_or(f64::NAN),
            on_ac(r),
            r.screen.unwrap_or(false),
        ));
        // Gaps become "off" periods.
        if let Some(prev) = i.checked_sub(1).map(|p| &rows[p]) {
            if r.ts_ms - prev.ts_ms > GAP_MS {
                out.periods.push(PowerPeriod {
                    start_unix_ms: prev.ts_ms + dts[i - 1],
                    end_unix_ms: r.ts_ms,
                    kind: "off".into(),
                    start_pct: prev.pct.unwrap_or(f64::NAN),
                    end_pct: r.pct.unwrap_or(f64::NAN),
                    avg_w: f64::NAN,
                    screen_on_min: 0,
                });
                screen_ms.push(0);
            }
        }
        let kind = if on_ac(r) { "ac" } else { "battery" };
        if kind == "battery" {
            bat_ms += dt;
            if r.screen == Some(true) {
                screen_bat_ms += dt;
            }
            if let Some(w) = r.w {
                used_wms += w * dt as f64;
                bat_w_ms += dt;
            }
        } else {
            ac_ms += dt;
        }
        let lit = if r.screen == Some(true) { dt } else { 0 };
        match out.periods.last_mut() {
            Some(p) if p.kind == kind => {
                let n_before = (p.end_unix_ms - p.start_unix_ms) as f64;
                if let Some(w) = r.w {
                    p.avg_w = if p.avg_w.is_nan() {
                        w
                    } else {
                        (p.avg_w * n_before + w * dt as f64) / (n_before + dt as f64)
                    };
                }
                p.end_unix_ms = r.ts_ms + dt;
                p.end_pct = r.pct.unwrap_or(p.end_pct);
                if let Some(ms) = screen_ms.last_mut() {
                    *ms += lit;
                }
            }
            _ => {
                screen_ms.push(lit);
                out.periods.push(PowerPeriod {
                start_unix_ms: r.ts_ms,
                end_unix_ms: r.ts_ms + dt,
                kind: kind.into(),
                start_pct: r.pct.unwrap_or(f64::NAN),
                end_pct: r.pct.unwrap_or(f64::NAN),
                avg_w: r.w.unwrap_or(f64::NAN),
                screen_on_min: 0,
                })
            }
        }
    }
    for (p, ms) in out.periods.iter_mut().zip(&screen_ms) {
        p.screen_on_min = ((ms + 30_000) / 60_000) as u32;
    }
    out.on_battery_min = (bat_ms / 60_000) as u32;
    out.on_ac_min = (ac_ms / 60_000) as u32;
    out.screen_on_battery_min = (screen_bat_ms / 60_000) as u32;
    if bat_w_ms > 0 {
        out.avg_battery_w = used_wms / bat_w_ms as f64;
        out.used_wh = used_wms / 3_600_000.0;
    }
    out.used_pct = out
        .periods
        .iter()
        .filter(|p| p.kind == "battery")
        .map(|p| (p.start_pct - p.end_pct).max(0.0))
        .filter(|v| !v.is_nan())
        .sum();
    if out.avg_battery_w > 0.5 && out.full_wh > 0.0 {
        out.full_runtime_min = (out.full_wh / out.avg_battery_w * 60.0) as u32;
    }
    out.health = health(info, health_rows);
    out
}

/// Discharge runs: contiguous on-battery readings with power, as
/// (energy Wh, charge points used). Energy is counted only between two
/// readings of the same run (trapezoids), so the minute after the last
/// reading, which may already be on the charger, is not added in.
fn discharge_runs(rows: &[Row]) -> Vec<(f64, f64)> {
    let mut runs = Vec::new();
    // (wh, start pct, last pct, last ts, last w)
    let mut cur: Option<(f64, f64, f64, u64, f64)> = None;
    for r in rows {
        let draining = !on_ac(r) && r.w.is_some_and(|w| w > 0.0) && r.pct.is_some();
        let (w, pct) = (r.w.unwrap_or(0.0), r.pct.unwrap_or(0.0));
        match cur.as_mut() {
            Some(c) if draining && r.ts_ms - c.3 <= GAP_MS => {
                c.0 += (c.4 + w) / 2.0 * (r.ts_ms - c.3) as f64 / 3_600_000.0;
                c.2 = pct;
                c.3 = r.ts_ms;
                c.4 = w;
            }
            _ => {
                if let Some((wh, a, b, _, _)) = cur.take() {
                    runs.push((wh, a - b));
                }
                if draining {
                    cur = Some((0.0, pct, pct, r.ts_ms, w));
                }
            }
        }
    }
    if let Some((wh, a, b, _, _)) = cur {
        runs.push((wh, a - b));
    }
    runs.into_iter().filter(|(_, d)| *d >= MIN_RUN_PCT).collect()
}

pub fn health(info: Option<&BatteryInfo>, rows: &[Row]) -> Vec<HealthEstimate> {
    let mut out = Vec::new();
    let design = info.and_then(|i| i.design_wh);
    if let (Some(d), Some(f)) = (design, info.and_then(|i| i.full_wh)) {
        let pct = f / d * 100.0;
        out.push(HealthEstimate {
            method: "fuel-gauge".into(),
            pct,
            capacity_wh: f,
            confidence: if (50.0..=105.0).contains(&pct) { "high" } else { "low" }.into(),
            basis: format!(
                "The battery's own gauge: {f:.1} Wh at the last full charge, {d:.1} Wh when new."
            ),
        });
    }
    let runs = discharge_runs(rows);
    let points: f64 = runs.iter().map(|r| r.1).sum();
    let wh: f64 = runs.iter().map(|r| r.0).sum();
    if points > 0.0 {
        let cap = wh / (points / 100.0);
        let plausible = design.is_none_or(|d| cap > 0.3 * d && cap < 1.3 * d);
        if plausible {
            out.push(HealthEstimate {
                method: "measured".into(),
                pct: design.map(|d| cap / d * 100.0).unwrap_or(f64::NAN),
                capacity_wh: cap,
                confidence: if points >= 60.0 {
                    "high"
                } else if points >= 25.0 {
                    "medium"
                } else {
                    "low"
                }
                .into(),
                basis: format!(
                    "Measured over {} discharge{}: {wh:.1} Wh used for {points:.0} points of \
                     charge, so a full charge holds about {cap:.1} Wh. Heavy use gets a little \
                     less out of a battery than light use.",
                    runs.len(),
                    if runs.len() == 1 { "" } else { "s" }
                ),
            });
        }
    }
    if let Some(c) = info.and_then(|i| i.cycles) {
        let at = |rated: f64| (100.0 - 20.0 * c as f64 / rated).clamp(40.0, 100.0);
        let (lo, hi) = (at(500.0), at(1000.0));
        out.push(HealthEstimate {
            method: "cycles".into(),
            pct: (lo + hi) / 2.0,
            capacity_wh: f64::NAN,
            confidence: "low".into(),
            basis: format!(
                "{c} charge cycles. Typical laptop cells keep about 80 % after 500 to 1000 \
                 cycles, which puts this one between {lo:.0} and {hi:.0} %."
            ),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;

    fn row(i: u64, pct: f64, w: f64, ac: bool) -> Row {
        Row {
            ts_ms: i * MIN,
            pct: Some(pct),
            w: Some(w),
            on_ac: Some(ac),
            screen: Some(true),
        }
    }

    fn info() -> BatteryInfo {
        BatteryInfo {
            design_wh: Some(60.0),
            full_wh: Some(54.0),
            cycles: Some(400),
            ..Default::default()
        }
    }

    #[test]
    fn a_morning_on_battery_then_the_charger_makes_two_periods_and_the_totals() {
        // 60 minutes on battery at 20 W from 100 % to 63 %, then 30 on the charger.
        let mut rows: Vec<Row> = (0..60)
            .map(|i| row(i, 100.0 - i as f64 * 37.0 / 59.0, 20.0, false))
            .collect();
        rows.extend((60..90).map(|i| row(i, 63.0 + (i - 60) as f64, -30.0, true)));
        let r = report(&rows, Some(&info()), &rows);
        let kinds: Vec<&str> = r.periods.iter().map(|p| p.kind.as_str()).collect();
        assert_eq!(kinds, ["battery", "ac"]);
        assert_eq!(r.on_battery_min, 60);
        assert_eq!(r.on_ac_min, 30);
        assert_eq!(r.screen_on_battery_min, 60);
        assert!((r.avg_battery_w - 20.0).abs() < 1e-9);
        assert!((r.used_wh - 20.0).abs() < 1e-9, "{}", r.used_wh);
        assert!((r.used_pct - 37.0).abs() < 1e-6);
        assert_eq!(r.full_runtime_min, 162, "54 Wh at 20 W");
        assert!((r.periods[1].avg_w + 30.0).abs() < 1e-9);
    }

    #[test]
    fn a_gap_in_the_readings_is_an_off_period() {
        let rows = vec![row(0, 80.0, 10.0, false), row(1, 80.0, 10.0, false), row(60, 70.0, 10.0, false)];
        let r = report(&rows, Some(&info()), &[]);
        let kinds: Vec<&str> = r.periods.iter().map(|p| p.kind.as_str()).collect();
        assert_eq!(kinds, ["battery", "off", "battery"]);
        assert_eq!(r.on_battery_min, 3);
    }

    #[test]
    fn health_is_measured_from_energy_per_point_of_charge() {
        // 50 points cost 27 Wh: a full charge holds 54 Wh, 90 % of 60.
        let rows: Vec<Row> = (0..=60)
            .map(|i| row(i, 100.0 - i as f64 * 50.0 / 60.0, 27.0, false))
            .collect();
        let h = health(Some(&info()), &rows);
        let m = h.iter().find(|e| e.method == "measured").unwrap();
        assert!((m.capacity_wh - 54.9).abs() < 1.0, "{}", m.capacity_wh);
        assert_eq!(m.confidence, "medium", "50 points");
        let g = h.iter().find(|e| e.method == "fuel-gauge").unwrap();
        assert!((g.pct - 90.0).abs() < 1e-9);
        assert_eq!(g.confidence, "high");
        let c = h.iter().find(|e| e.method == "cycles").unwrap();
        assert_eq!(c.confidence, "low");
        assert!(c.basis.contains("between 84 and 92"), "{}", c.basis);
    }

    #[test]
    fn short_runs_are_not_measured_and_implausible_ones_are_dropped() {
        let short: Vec<Row> = (0..10).map(|i| row(i, 100.0 - i as f64 * 0.5, 15.0, false)).collect();
        assert!(!health(Some(&info()), &short).iter().any(|e| e.method == "measured"));
        // 20 points for 1 Wh would be a 5 Wh battery.
        let silly: Vec<Row> = (0..=20).map(|i| row(i, 100.0 - i as f64, 3.0, false)).collect();
        assert!(!health(Some(&info()), &silly).iter().any(|e| e.method == "measured"));
    }

    #[test]
    fn rows_without_a_charger_column_are_judged_by_the_draw() {
        let mut r = row(0, 90.0, 12.0, false);
        r.on_ac = None;
        assert!(!on_ac(&r));
        r.w = Some(0.0);
        assert!(on_ac(&r), "full and idle: on the charger");
    }

    #[test]
    fn a_desktop_has_no_battery_report() {
        let r = report(&[], None, &[]);
        assert!(!r.has_battery);
        assert!(r.health.is_empty());
    }
}
