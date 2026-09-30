//! Monitoring history: one reading a minute, kept 90 days, for the charts
//! that look further back than the live two minutes.
//!
//! One `Snapshot` call a minute costs the daemon a few milliseconds and never
//! wakes a sleeping GPU (the daemon's own rule). Readings are averages of what
//! the daemon reports at that instant, not minute averages — enough for
//! trends, and labelled as such by the UI. Network traffic is kept as the
//! interface's cumulative byte counters, so daily totals are exact; a counter
//! that goes backwards (reboot, another interface) starts a new baseline.

use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;
use ug_proto::{DayTraffic, HistoryPoint, Snapshot};

const KEEP_MS: u64 = 90 * 24 * 3_600_000;

pub struct History {
    conn: Mutex<Connection>,
}

fn nan_to_null(v: f64) -> Option<f64> {
    (!v.is_nan()).then_some(v)
}

impl History {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(p) = path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        Self::init(Connection::open(path)?)
    }

    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self::init(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS samples (
                 ts_ms       INTEGER PRIMARY KEY,
                 cpu_temp    REAL,
                 cpu_load    REAL,
                 cpu_w       REAL,
                 gpu_temp    REAL,
                 gpu_load    REAL,
                 gpu_w       REAL,
                 mem_used_kb INTEGER,
                 iface       TEXT,
                 rx_total    INTEGER,
                 tx_total    INTEGER
             );",
        )?;
        Ok(History { conn: Mutex::new(conn) })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Stores a reading. `gpu_pci` picks the GPU charted (the one that works:
    /// the caller passes the discrete one only while it is awake).
    pub fn record(&self, s: &Snapshot, gpu_pci: Option<&str>) {
        let g = gpu_pci.and_then(|pci| s.gpus.iter().find(|g| g.pci == pci));
        let mem_used = s.mem.total_kb.saturating_sub(s.mem.available_kb);
        let r = self.conn().execute(
            "INSERT OR REPLACE INTO samples VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                s.ts_unix_ms as i64,
                nan_to_null(s.cpu.temp_c),
                nan_to_null(s.cpu.load_pct),
                nan_to_null(s.cpu.power_w),
                g.and_then(|g| nan_to_null(g.temp_c)),
                g.and_then(|g| nan_to_null(g.load_pct)),
                g.and_then(|g| nan_to_null(g.power_w)),
                mem_used as i64,
                s.net.iface,
                s.net.rx_total_bytes as i64,
                s.net.tx_total_bytes as i64,
            ],
        );
        if let Err(e) = r {
            tracing::warn!(error = %e, "could not record a history sample");
        }
    }

    /// Deletes readings older than 90 days.
    pub fn prune(&self, now_ms: u64) {
        let _ = self.conn().execute("DELETE FROM samples WHERE ts_ms < ?1", params![now_ms.saturating_sub(KEEP_MS) as i64]);
    }

    /// Readings between `since` and `until`, averaged into at most `points`
    /// equal buckets. Missing readings stay missing (NaN), never zero.
    pub fn range(&self, since_ms: u64, until_ms: u64, points: u32) -> Vec<HistoryPoint> {
        let points = points.clamp(2, 2000) as u64;
        let bucket = ((until_ms.saturating_sub(since_ms)) / points).max(60_000) as i64;
        let c = self.conn();
        let mut stmt = match c.prepare(
            "SELECT (ts_ms / ?3) * ?3, AVG(cpu_temp), AVG(cpu_load), AVG(cpu_w),
                    AVG(gpu_temp), AVG(gpu_load), AVG(gpu_w), AVG(mem_used_kb)
               FROM samples WHERE ts_ms >= ?1 AND ts_ms <= ?2
           GROUP BY 1 ORDER BY 1",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "history query failed");
                return Vec::new();
            }
        };
        let f = |v: Option<f64>| v.unwrap_or(f64::NAN);
        stmt.query_map(params![since_ms as i64, until_ms as i64, bucket], |r| {
            Ok(HistoryPoint {
                ts_unix_ms: r.get::<_, i64>(0)? as u64,
                cpu_temp_c: f(r.get(1)?),
                cpu_load_pct: f(r.get(2)?),
                cpu_w: f(r.get(3)?),
                gpu_temp_c: f(r.get(4)?),
                gpu_load_pct: f(r.get(5)?),
                gpu_w: f(r.get(6)?),
                mem_used_kb: r.get::<_, Option<f64>>(7)?.unwrap_or(0.0) as u64,
            })
        })
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }

    /// Bytes received and sent per local day, from the cumulative counters.
    /// `day_of` maps a timestamp to its local date.
    pub fn traffic(&self, since_ms: u64, day_of: impl Fn(u64) -> String) -> Vec<DayTraffic> {
        let c = self.conn();
        let Ok(mut stmt) =
            c.prepare("SELECT ts_ms, iface, rx_total, tx_total FROM samples WHERE ts_ms >= ?1 ORDER BY ts_ms")
        else {
            return Vec::new();
        };
        let rows: Vec<(u64, String, u64, u64)> = stmt
            .query_map(params![since_ms as i64], |r| {
                Ok((
                    r.get::<_, i64>(0)? as u64,
                    r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    r.get::<_, Option<i64>>(2)?.unwrap_or(0) as u64,
                    r.get::<_, Option<i64>>(3)?.unwrap_or(0) as u64,
                ))
            })
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default();
        let mut out: Vec<DayTraffic> = Vec::new();
        let mut prev: Option<&(u64, String, u64, u64)> = None;
        for row in &rows {
            if let Some(p) = prev {
                // Same interface and counters moving forward: a real delta.
                if p.1 == row.1 && row.2 >= p.2 && row.3 >= p.3 {
                    let day = day_of(row.0);
                    if out.last().map(|d| &d.date) != Some(&day) {
                        out.push(DayTraffic { date: day, rx_bytes: 0, tx_bytes: 0 });
                    }
                    let d = out.last_mut().expect("just pushed");
                    d.rx_bytes += row.2 - p.2;
                    d.tx_bytes += row.3 - p.3;
                }
            }
            prev = Some(row);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_proto::{CpuSample, GpuSample, MemSample, NetSample};

    const MIN: u64 = 60_000;

    fn snap(ts: u64, cpu_temp: f64, rx: u64, tx: u64, iface: &str) -> Snapshot {
        Snapshot {
            ts_unix_ms: ts,
            cpu: CpuSample {
                load_pct: 20.0,
                temp_c: cpu_temp,
                power_w: f64::NAN,
                freq_avg_mhz: 0,
                freq_max_mhz: 0,
                per_core_load: vec![],
                governor: String::new(),
                epp: String::new(),
                driver: String::new(),
            },
            gpus: vec![GpuSample {
                pci: "0000:01:00.0".into(),
                temp_c: 60.0,
                temp_hotspot_c: f64::NAN,
                temp_mem_c: f64::NAN,
                load_pct: 50.0,
                power_w: 30.0,
                power_limit_w: f64::NAN,
                core_clock_mhz: 0,
                mem_clock_mhz: 0,
                vram_used_mb: 0,
                fan_pct: f64::NAN,
                fan_rpm: 0,
                asleep: false,
            }],
            mem: MemSample { total_kb: 1000, available_kb: 400, swap_total_kb: 0, swap_used_kb: 0, zram_used_kb: 0 },
            net: NetSample { iface: iface.into(), rx_bps: 0, tx_bps: 0, rx_total_bytes: rx, tx_total_bytes: tx },
        }
    }

    #[test]
    fn readings_are_averaged_into_buckets_and_gaps_stay_missing() {
        let h = History::in_memory();
        for (i, t) in [40.0, 50.0, 60.0, 70.0].into_iter().enumerate() {
            h.record(&snap(i as u64 * MIN, t, 0, 0, "wlan0"), None);
        }
        h.record(&snap(10 * MIN, f64::NAN, 0, 0, "wlan0"), None);
        let pts = h.range(0, 12 * MIN, 6); // 2-minute buckets
        assert_eq!(pts[0].cpu_temp_c, 45.0);
        assert_eq!(pts[1].cpu_temp_c, 65.0);
        assert!(pts.last().unwrap().cpu_temp_c.is_nan(), "a missing reading is not zero");
        assert!(pts[0].gpu_temp_c.is_nan(), "no GPU charted when none was passed");
        assert_eq!(pts[0].mem_used_kb, 600);
    }

    #[test]
    fn the_working_gpu_is_charted() {
        let h = History::in_memory();
        h.record(&snap(0, 40.0, 0, 0, "wlan0"), Some("0000:01:00.0"));
        assert_eq!(h.range(0, MIN, 10)[0].gpu_temp_c, 60.0);
    }

    #[test]
    fn daily_traffic_is_exact_and_survives_a_counter_reset() {
        let h = History::in_memory();
        let day = |ts: u64| if ts < 3 * MIN { "d1".to_string() } else { "d2".to_string() };
        h.record(&snap(0, 40.0, 1_000, 100, "wlan0"), None);
        h.record(&snap(MIN, 40.0, 5_000, 300, "wlan0"), None);
        h.record(&snap(2 * MIN, 40.0, 6_000, 400, "wlan0"), None);
        h.record(&snap(3 * MIN, 40.0, 50, 10, "wlan0"), None); // rebooted: counters reset
        h.record(&snap(4 * MIN, 40.0, 2_050, 510, "wlan0"), None);
        h.record(&snap(5 * MIN, 40.0, 900, 900, "eth0"), None); // switched interface
        let t = h.traffic(0, day);
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].rx_bytes, t[0].tx_bytes), (5_000, 300));
        assert_eq!((t[1].rx_bytes, t[1].tx_bytes), (2_000, 500));
    }

    #[test]
    fn old_readings_are_pruned() {
        let h = History::in_memory();
        h.record(&snap(0, 40.0, 0, 0, "wlan0"), None);
        h.record(&snap(KEEP_MS + 2 * MIN, 40.0, 0, 0, "wlan0"), None);
        h.prune(KEEP_MS + 3 * MIN);
        assert_eq!(h.range(0, KEEP_MS * 2, 100).len(), 1);
    }
}
