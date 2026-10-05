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
use ug_proto::{AppTraffic, AppUsage, DayTraffic, ExtraSample, FpsSession, HistoryPoint, Snapshot, TrafficBucket};

const KEEP_MS: u64 = 90 * 24 * 3_600_000;

pub struct History {
    conn: Mutex<Connection>,
    /// Each app's byte counters at the last reading (by cgroup), and whether
    /// there was a last reading at all since the agent started.
    apps_seen: Mutex<(std::collections::HashMap<String, (u64, u64)>, bool)>,
}

/// Per-app traffic is kept in 15-minute slots, so local days, weeks and
/// months line up even in time zones half an hour off UTC (Iran).
const SLOT_MS: u64 = 15 * 60_000;

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
             );
             CREATE TABLE IF NOT EXISTS fps_sessions (
                 started_ms INTEGER NOT NULL,
                 game       TEXT    NOT NULL,
                 duration_s INTEGER NOT NULL,
                 frames     INTEGER NOT NULL,
                 avg_fps    REAL    NOT NULL,
                 low1       REAL    NOT NULL,
                 low01      REAL    NOT NULL,
                 p99_ms     REAL    NOT NULL,
                 stutters   INTEGER NOT NULL,
                 PRIMARY KEY (started_ms, game)
             );
             CREATE TABLE IF NOT EXISTS app_traffic (
                 slot_ms  INTEGER NOT NULL,
                 name     TEXT    NOT NULL,
                 category TEXT    NOT NULL,
                 rx       INTEGER NOT NULL,
                 tx       INTEGER NOT NULL,
                 PRIMARY KEY (slot_ms, name)
             );
             CREATE TABLE IF NOT EXISTS meta (
                 key   TEXT PRIMARY KEY,
                 value INTEGER NOT NULL
             );",
        )?;
        // Columns added after the first release; an existing table gets them
        // empty (older readings simply have no value there).
        for col in ["cpu_mhz", "gpu_mhz", "swap_kb", "disk_r", "disk_w", "disk_busy", "load1", "procs", "bat_pct", "bat_w"] {
            let has = conn
                .prepare("SELECT 1 FROM pragma_table_info('samples') WHERE name = ?1")?
                .exists(params![col])?;
            if !has {
                conn.execute_batch(&format!("ALTER TABLE samples ADD COLUMN {col} REAL"))?;
            }
        }
        Ok(History { conn: Mutex::new(conn), apps_seen: Mutex::new((Default::default(), false)) })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Stores one finished game session's frame statistics (kept forever:
    /// a row a session is nothing).
    pub fn record_fps(&self, s: &FpsSession) -> bool {
        self.conn()
            .execute(
                "INSERT OR REPLACE INTO fps_sessions VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    s.started_unix_ms as i64,
                    s.game,
                    s.duration_s as i64,
                    s.frames as i64,
                    s.avg_fps,
                    s.low_1pct_fps,
                    s.low_01pct_fps,
                    s.p99_frametime_ms,
                    s.stutters as i64
                ],
            )
            .inspect_err(|e| tracing::warn!(error = %e, "could not store an FPS session"))
            .is_ok()
    }

    /// Sessions that started since `since`, newest first.
    pub fn fps_sessions(&self, since: u64) -> Vec<FpsSession> {
        let c = self.conn();
        let Ok(mut q) = c.prepare(
            "SELECT started_ms, game, duration_s, frames, avg_fps, low1, low01, p99_ms, stutters
             FROM fps_sessions WHERE started_ms >= ?1 ORDER BY started_ms DESC",
        ) else {
            return Vec::new();
        };
        q.query_map(params![since as i64], |r| {
            Ok(FpsSession {
                started_unix_ms: r.get::<_, i64>(0)? as u64,
                game: r.get(1)?,
                duration_s: r.get::<_, i64>(2)? as u64,
                frames: r.get::<_, i64>(3)? as u64,
                avg_fps: r.get(4)?,
                low_1pct_fps: r.get(5)?,
                low_01pct_fps: r.get(6)?,
                p99_frametime_ms: r.get(7)?,
                stutters: r.get::<_, i64>(8)? as u64,
            })
        })
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }

    pub fn forget_fps(&self) -> u32 {
        self.conn().execute("DELETE FROM fps_sessions", []).unwrap_or(0) as u32
    }

    /// Stores a reading. `gpu_pci` picks the GPU charted (the one that works:
    /// the caller passes the discrete one only while it is awake). `extra`
    /// adds disks, load, processes and the battery when the service gave it.
    pub fn record(&self, s: &Snapshot, gpu_pci: Option<&str>, extra: Option<&ExtraSample>) {
        let g = gpu_pci.and_then(|pci| s.gpus.iter().find(|g| g.pci == pci));
        let mem_used = s.mem.total_kb.saturating_sub(s.mem.available_kb);
        let e = |f: fn(&ExtraSample) -> f64| extra.and_then(|x| nan_to_null(f(x)));
        let r = self.conn().execute(
            "INSERT OR REPLACE INTO samples
               (ts_ms, cpu_temp, cpu_load, cpu_w, gpu_temp, gpu_load, gpu_w, mem_used_kb, iface, rx_total, tx_total,
                cpu_mhz, gpu_mhz, swap_kb, disk_r, disk_w, disk_busy, load1, procs, bat_pct, bat_w)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
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
                (s.cpu.freq_max_mhz > 0).then_some(s.cpu.freq_max_mhz as f64),
                g.and_then(|g| (g.core_clock_mhz > 0).then_some(g.core_clock_mhz as f64)),
                s.mem.swap_used_kb as f64,
                e(|x| x.disk_read_bps as f64),
                e(|x| x.disk_write_bps as f64),
                e(|x| x.disk_busy_pct),
                e(|x| x.load_avg_1),
                e(|x| x.processes as f64),
                e(|x| x.battery_pct),
                e(|x| x.battery_w),
            ],
        );
        if let Err(e) = r {
            tracing::warn!(error = %e, "could not record a history sample");
        }
    }

    /// Deletes readings older than 90 days.
    pub fn prune(&self, now_ms: u64) {
        let cut = now_ms.saturating_sub(KEEP_MS) as i64;
        let _ = self.conn().execute("DELETE FROM samples WHERE ts_ms < ?1", params![cut]);
        let _ = self.conn().execute("DELETE FROM app_traffic WHERE slot_ms < ?1", params![cut]);
    }

    /// Readings between `since` and `until`, averaged into at most `points`
    /// equal buckets. Missing readings stay missing (NaN), never zero.
    /// Internet speeds come from the byte counters between consecutive
    /// readings (same interface, counters moving forward, at most three
    /// minutes apart), so they are averages, not instants.
    pub fn range(&self, since_ms: u64, until_ms: u64, points: u32) -> Vec<HistoryPoint> {
        let points = points.clamp(2, 2000) as u64;
        let bucket = ((until_ms.saturating_sub(since_ms)) / points).max(60_000) as i64;
        let c = self.conn();
        let mut stmt = match c.prepare(
            "WITH d AS (
                 SELECT *,
                        LAG(ts_ms) OVER w AS p_ts, LAG(iface) OVER w AS p_if,
                        LAG(rx_total) OVER w AS p_rx, LAG(tx_total) OVER w AS p_tx
                   FROM samples WHERE ts_ms >= ?1 - 180000 AND ts_ms <= ?2
                 WINDOW w AS (ORDER BY ts_ms)),
             r AS (
                 SELECT *,
                        CASE WHEN iface = p_if AND rx_total >= p_rx AND tx_total >= p_tx
                                  AND ts_ms - p_ts BETWEEN 1 AND 180000
                             THEN (rx_total - p_rx) * 1000.0 / (ts_ms - p_ts) END AS rx_bps,
                        CASE WHEN iface = p_if AND rx_total >= p_rx AND tx_total >= p_tx
                                  AND ts_ms - p_ts BETWEEN 1 AND 180000
                             THEN (tx_total - p_tx) * 1000.0 / (ts_ms - p_ts) END AS tx_bps
                   FROM d WHERE ts_ms >= ?1)
             SELECT (ts_ms / ?3) * ?3, AVG(cpu_temp), AVG(cpu_load), AVG(cpu_w),
                    AVG(gpu_temp), AVG(gpu_load), AVG(gpu_w), AVG(mem_used_kb),
                    AVG(cpu_mhz), AVG(gpu_mhz), AVG(swap_kb), AVG(disk_r), AVG(disk_w), AVG(disk_busy),
                    AVG(rx_bps), AVG(tx_bps), AVG(load1), AVG(procs), AVG(bat_pct), AVG(bat_w)
               FROM r GROUP BY 1 ORDER BY 1",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "history query failed");
                return Vec::new();
            }
        };
        let f = |v: Option<f64>| v.unwrap_or(f64::NAN);
        stmt.query_map(params![since_ms as i64, until_ms.min(i64::MAX as u64) as i64, bucket], |r| {
            Ok(HistoryPoint {
                ts_unix_ms: r.get::<_, i64>(0)? as u64,
                cpu_temp_c: f(r.get(1)?),
                cpu_load_pct: f(r.get(2)?),
                cpu_w: f(r.get(3)?),
                gpu_temp_c: f(r.get(4)?),
                gpu_load_pct: f(r.get(5)?),
                gpu_w: f(r.get(6)?),
                mem_used_kb: r.get::<_, Option<f64>>(7)?.unwrap_or(0.0) as u64,
                cpu_mhz: f(r.get(8)?),
                gpu_mhz: f(r.get(9)?),
                swap_used_kb: f(r.get(10)?),
                disk_read_bps: f(r.get(11)?),
                disk_write_bps: f(r.get(12)?),
                disk_busy_pct: f(r.get(13)?),
                net_rx_bps: f(r.get(14)?),
                net_tx_bps: f(r.get(15)?),
                load_avg_1: f(r.get(16)?),
                processes: f(r.get(17)?),
                battery_pct: f(r.get(18)?),
                battery_w: f(r.get(19)?),
            })
        })
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }

    /// Byte counts between consecutive readings, each at the later reading's
    /// time: only where the interface stayed the same and its counters moved
    /// forward (a reboot or another interface starts a new baseline).
    fn deltas(&self, since_ms: u64, until_ms: u64) -> Vec<(u64, u64, u64)> {
        let reset = self.reset_at();
        let since_ms = since_ms.max(reset);
        let c = self.conn();
        // The reading just before `since` is the first delta's baseline.
        let Ok(mut stmt) = c.prepare(
            "SELECT ts_ms, iface, rx_total, tx_total FROM samples
              WHERE ts_ms >= COALESCE((SELECT MAX(ts_ms) FROM samples WHERE ts_ms < ?1 AND ts_ms >= ?3), ?1) AND ts_ms <= ?2
              ORDER BY ts_ms",
        ) else {
            return Vec::new();
        };
        let rows: Vec<(u64, String, u64, u64)> = stmt
            .query_map(params![since_ms as i64, until_ms.min(i64::MAX as u64) as i64, reset as i64], |r| {
                Ok((
                    r.get::<_, i64>(0)? as u64,
                    r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    r.get::<_, Option<i64>>(2)?.unwrap_or(0) as u64,
                    r.get::<_, Option<i64>>(3)?.unwrap_or(0) as u64,
                ))
            })
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default();
        rows.windows(2)
            .filter(|w| w[1].0 >= since_ms && w[0].1 == w[1].1 && w[1].2 >= w[0].2 && w[1].3 >= w[0].3)
            .map(|w| (w[1].0, w[1].2 - w[0].2, w[1].3 - w[0].3))
            .collect()
    }

    /// Bytes received and sent per local day, from the cumulative counters.
    /// `day_of` maps a timestamp to its local date.
    pub fn traffic(&self, since_ms: u64, day_of: impl Fn(u64) -> String) -> Vec<DayTraffic> {
        let mut out: Vec<DayTraffic> = Vec::new();
        for (ts, rx, tx) in self.deltas(since_ms, u64::MAX) {
            let day = day_of(ts);
            if out.last().map(|d| &d.date) != Some(&day) {
                out.push(DayTraffic { date: day, rx_bytes: 0, tx_bytes: 0 });
            }
            let d = out.last_mut().expect("just pushed");
            d.rx_bytes += rx;
            d.tx_bytes += tx;
        }
        out
    }

    /// Bytes received and sent in equal buckets of `bucket_ms` from `since`
    /// (so a caller's local midnight gives local hours). Every bucket up to
    /// `until` is returned, empty ones as zero; at most 2000.
    pub fn traffic_buckets(&self, since_ms: u64, until_ms: u64, bucket_ms: u64) -> Vec<TrafficBucket> {
        let bucket = bucket_ms.max(60_000);
        let n = (until_ms.saturating_sub(since_ms)).div_ceil(bucket).clamp(1, 2000);
        let mut out: Vec<TrafficBucket> =
            (0..n).map(|i| TrafficBucket { start_unix_ms: since_ms + i * bucket, rx_bytes: 0, tx_bytes: 0 }).collect();
        for (ts, rx, tx) in self.deltas(since_ms, until_ms) {
            // A delta at `since` itself was mostly used before it.
            if ts == since_ms {
                continue;
            }
            let i = ((ts - since_ms - 1) / bucket) as usize;
            if let Some(b) = out.get_mut(i) {
                b.rx_bytes += rx;
                b.tx_bytes += tx;
            }
        }
        out
    }

    /// When the traffic counts were last reset (0: never). Readings before it
    /// are kept for the speed charts but not counted as traffic.
    pub fn reset_at(&self) -> u64 {
        self.conn()
            .query_row("SELECT value FROM meta WHERE key = 'traffic_reset_ms'", [], |r| r.get::<_, i64>(0))
            .map(|v| v.max(0) as u64)
            .unwrap_or(0)
    }

    /// Starts the traffic counts again from `now_ms`: totals, days and the
    /// per-app list start from zero.
    pub fn reset_traffic(&self, now_ms: u64) -> bool {
        let c = self.conn();
        let ok = c
            .execute("INSERT OR REPLACE INTO meta (key, value) VALUES ('traffic_reset_ms', ?1)", params![now_ms as i64])
            .and_then(|_| c.execute("DELETE FROM app_traffic", []))
            .inspect_err(|e| tracing::warn!(error = %e, "could not reset the traffic counts"))
            .is_ok();
        ok
    }

    /// Adds what each app sent and received since the last reading (once a
    /// minute, from the service's per-app counters). Apps are summed by name;
    /// a counter that went backwards (the service restarted) counts from
    /// zero. On the agent's first reading only the baselines are taken, so
    /// traffic from before is not counted twice.
    pub fn record_apps(&self, ts_ms: u64, apps: &[AppTraffic]) {
        let mut by_name: std::collections::BTreeMap<String, (String, u64, u64)> = Default::default();
        {
            let mut seen = self.apps_seen.lock().unwrap_or_else(|e| e.into_inner());
            let (last, primed) = &mut *seen;
            let mut now = std::collections::HashMap::new();
            for a in apps {
                let d = |cur: u64, prev: Option<u64>| match prev {
                    Some(p) if cur >= p => cur - p,
                    Some(_) => cur,
                    None if *primed => cur,
                    None => 0,
                };
                let prev = last.get(&a.id).copied();
                let (rx, tx) = (d(a.rx_bytes, prev.map(|p| p.0)), d(a.tx_bytes, prev.map(|p| p.1)));
                now.insert(a.id.clone(), (a.rx_bytes, a.tx_bytes));
                if rx + tx > 0 {
                    let e = by_name.entry(a.name.clone()).or_insert_with(|| (a.category.clone(), 0, 0));
                    e.1 += rx;
                    e.2 += tx;
                }
            }
            *last = now;
            *primed = true;
        }
        if by_name.is_empty() {
            return;
        }
        let slot = (ts_ms / SLOT_MS * SLOT_MS) as i64;
        let mut c = self.conn();
        let Ok(t) = c.transaction() else { return };
        for (name, (category, rx, tx)) in by_name {
            let _ = t.execute(
                "INSERT INTO app_traffic (slot_ms, name, category, rx, tx) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (slot_ms, name) DO UPDATE SET rx = rx + excluded.rx, tx = tx + excluded.tx, category = excluded.category",
                params![slot, name, category, rx as i64, tx as i64],
            );
        }
        let _ = t.commit();
    }

    /// Each app's traffic between two times (whole 15-minute slots that start
    /// in the range), most used first.
    pub fn app_traffic(&self, since_ms: u64, until_ms: u64) -> Vec<AppUsage> {
        let c = self.conn();
        let Ok(mut q) = c.prepare(
            "SELECT name, MAX(category), SUM(rx), SUM(tx) FROM app_traffic
              WHERE slot_ms >= ?1 AND slot_ms < ?2 GROUP BY name ORDER BY SUM(rx) + SUM(tx) DESC, name",
        ) else {
            return Vec::new();
        };
        q.query_map(params![since_ms as i64, until_ms.min(i64::MAX as u64) as i64], |r| {
            Ok(AppUsage {
                name: r.get(0)?,
                category: r.get(1)?,
                rx_bytes: r.get::<_, i64>(2)?.max(0) as u64,
                tx_bytes: r.get::<_, i64>(3)?.max(0) as u64,
            })
        })
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
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
            h.record(&snap(i as u64 * MIN, t, 0, 0, "wlan0"), None, None);
        }
        h.record(&snap(10 * MIN, f64::NAN, 0, 0, "wlan0"), None, None);
        let pts = h.range(0, 12 * MIN, 6); // 2-minute buckets
        assert_eq!(pts[0].cpu_temp_c, 45.0);
        assert_eq!(pts[1].cpu_temp_c, 65.0);
        assert!(pts.last().unwrap().cpu_temp_c.is_nan(), "a missing reading is not zero");
        assert!(pts[0].gpu_temp_c.is_nan(), "no GPU charted when none was passed");
        assert_eq!(pts[0].mem_used_kb, 600);
    }

    #[test]
    fn every_live_reading_is_kept_and_speeds_come_from_the_counters() {
        let h = History::in_memory();
        let x = ExtraSample { disk_read_bps: 2_000_000, disk_busy_pct: 10.0, load_avg_1: 1.5, processes: 300, battery_pct: f64::NAN, battery_w: f64::NAN, ..Default::default() };
        h.record(&snap(0, 40.0, 0, 0, "wlan0"), None, Some(&x));
        h.record(&snap(MIN, 40.0, 6_000_000, 600_000, "wlan0"), None, Some(&x));
        h.record(&snap(2 * MIN, 40.0, 18_000_000, 600_000, "wlan0"), None, None);
        let pts = h.range(0, 3 * MIN, 3);
        assert!(pts[0].net_rx_bps.is_nan(), "no speed without a reading before");
        assert_eq!(pts[1].net_rx_bps, 100_000.0, "6 MB in a minute");
        assert_eq!(pts[1].net_tx_bps, 10_000.0);
        assert_eq!(pts[2].net_rx_bps, 200_000.0);
        assert_eq!((pts[0].disk_read_bps, pts[0].load_avg_1, pts[0].processes), (2_000_000.0, 1.5, 300.0));
        assert!(pts[2].disk_read_bps.is_nan(), "no extra reading: missing, not zero");
        assert!(pts[0].battery_pct.is_nan(), "no battery");
        // The first bucket's speed uses the reading just before the range.
        assert_eq!(h.range(MIN, 3 * MIN, 2)[0].net_rx_bps, 100_000.0);
    }

    #[test]
    fn a_table_from_an_older_version_gets_the_new_columns() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE samples (ts_ms INTEGER PRIMARY KEY, cpu_temp REAL, cpu_load REAL, cpu_w REAL, gpu_temp REAL,
             gpu_load REAL, gpu_w REAL, mem_used_kb INTEGER, iface TEXT, rx_total INTEGER, tx_total INTEGER);
             INSERT INTO samples VALUES (0, 50, 10, 5, NULL, NULL, NULL, 100, 'wlan0', 0, 0);",
        )
        .unwrap();
        let h = History::init(c).unwrap();
        h.record(&snap(MIN, 60.0, 0, 0, "wlan0"), None, None);
        let pts = h.range(0, 2 * MIN, 2);
        assert_eq!(pts[0].cpu_temp_c, 50.0, "old readings are kept");
        assert!(pts[0].cpu_mhz.is_nan());
        assert_eq!(pts[1].cpu_temp_c, 60.0);
    }

    #[test]
    fn the_working_gpu_is_charted() {
        let h = History::in_memory();
        h.record(&snap(0, 40.0, 0, 0, "wlan0"), Some("0000:01:00.0"), None);
        assert_eq!(h.range(0, MIN, 10)[0].gpu_temp_c, 60.0);
    }

    #[test]
    fn daily_traffic_is_exact_and_survives_a_counter_reset() {
        let h = History::in_memory();
        let day = |ts: u64| if ts < 3 * MIN { "d1".to_string() } else { "d2".to_string() };
        h.record(&snap(0, 40.0, 1_000, 100, "wlan0"), None, None);
        h.record(&snap(MIN, 40.0, 5_000, 300, "wlan0"), None, None);
        h.record(&snap(2 * MIN, 40.0, 6_000, 400, "wlan0"), None, None);
        h.record(&snap(3 * MIN, 40.0, 50, 10, "wlan0"), None, None); // rebooted: counters reset
        h.record(&snap(4 * MIN, 40.0, 2_050, 510, "wlan0"), None, None);
        h.record(&snap(5 * MIN, 40.0, 900, 900, "eth0"), None, None); // switched interface
        let t = h.traffic(0, day);
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].rx_bytes, t[0].tx_bytes), (5_000, 300));
        assert_eq!((t[1].rx_bytes, t[1].tx_bytes), (2_000, 500));
    }

    #[test]
    fn traffic_in_buckets_counts_each_delta_once_and_keeps_empty_buckets() {
        let h = History::in_memory();
        h.record(&snap(0, 40.0, 1_000, 100, "wlan0"), None, None);
        h.record(&snap(MIN, 40.0, 5_000, 300, "wlan0"), None, None);
        h.record(&snap(2 * MIN, 40.0, 6_000, 400, "wlan0"), None, None);
        h.record(&snap(3 * MIN, 40.0, 50, 10, "wlan0"), None, None); // rebooted
        h.record(&snap(4 * MIN, 40.0, 2_050, 510, "wlan0"), None, None);
        // Two-minute buckets from the first minute: (1, 3], (3, 5], (5, 7].
        let b = h.traffic_buckets(MIN, 7 * MIN, 2 * MIN);
        assert_eq!(b.len(), 3);
        assert_eq!(b[0].start_unix_ms, MIN);
        // The delta ending at `since` belongs before it.
        assert_eq!((b[0].rx_bytes, b[0].tx_bytes), (1_000, 100));
        assert_eq!((b[1].rx_bytes, b[1].tx_bytes), (2_000, 500));
        assert_eq!((b[2].rx_bytes, b[2].tx_bytes), (0, 0));
        // The whole range adds up to the daily total.
        let all = h.traffic_buckets(0, 4 * MIN, 4 * MIN);
        let day = h.traffic(0, |_| "d".into());
        assert_eq!((all[0].rx_bytes, all[0].tx_bytes), (day[0].rx_bytes, day[0].tx_bytes));
    }

    #[test]
    fn old_readings_are_pruned() {
        let h = History::in_memory();
        h.record(&snap(0, 40.0, 0, 0, "wlan0"), None, None);
        h.record(&snap(KEEP_MS + 2 * MIN, 40.0, 0, 0, "wlan0"), None, None);
        h.prune(KEEP_MS + 3 * MIN);
        assert_eq!(h.range(0, KEEP_MS * 2, 100).len(), 1);
    }

    #[test]
    fn fps_sessions_are_stored_listed_newest_first_and_forgotten() {
        let h = History::in_memory();
        let s = |t: u64, g: &str| FpsSession {
            game: g.into(), started_unix_ms: t, duration_s: 600, frames: 80_000, avg_fps: 133.0,
            low_1pct_fps: 88.0, low_01pct_fps: 61.0, p99_frametime_ms: 11.4, stutters: 3,
        };
        assert!(h.record_fps(&s(1_000, "cs2")));
        assert!(h.record_fps(&s(5_000, "eldenring")));
        assert!(h.record_fps(&s(5_000, "eldenring")), "the same session twice is one row");
        let all = h.fps_sessions(0);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].game, "eldenring");
        assert_eq!(all[1], s(1_000, "cs2"));
        assert_eq!(h.fps_sessions(2_000).len(), 1);
        assert_eq!(h.forget_fps(), 2);
        assert!(h.fps_sessions(0).is_empty());
    }

    fn app(id: &str, name: &str, rx: u64, tx: u64) -> AppTraffic {
        AppTraffic { id: id.into(), name: name.into(), kind: "app".into(), category: "launcher".into(), rx_bytes: rx, tx_bytes: tx, ..Default::default() }
    }

    #[test]
    fn app_traffic_counts_deltas_by_name_and_not_what_came_before_the_agent() {
        let h = History::in_memory();
        // First reading: baselines only (the agent just started).
        h.record_apps(0, &[app("a/steam", "steam", 5_000, 50)]);
        h.record_apps(MIN, &[app("a/steam", "steam", 8_000, 80), app("b/firefox", "firefox", 1_000, 10)]);
        // Two cgroups with one name add up; a counter that went back counts from zero.
        h.record_apps(2 * MIN, &[app("a/steam", "steam", 500, 5), app("c/steam", "steam", 100, 1), app("b/firefox", "firefox", 1_500, 15)]);
        let apps = h.app_traffic(0, 3 * MIN);
        assert_eq!(apps.len(), 2);
        assert_eq!((apps[0].name.as_str(), apps[0].rx_bytes, apps[0].tx_bytes), ("steam", 3_000 + 600, 30 + 6));
        assert_eq!((apps[1].name.as_str(), apps[1].rx_bytes), ("firefox", 1_500));
        assert_eq!(apps[0].category, "launcher");
        assert!(h.app_traffic(SLOT_MS, 2 * SLOT_MS).is_empty(), "outside the range");
    }

    #[test]
    fn resetting_traffic_starts_totals_and_apps_from_zero_but_keeps_speeds() {
        let h = History::in_memory();
        h.record(&snap(0, 40.0, 1_000, 100, "wlan0"), None, None);
        h.record(&snap(MIN, 40.0, 9_000, 900, "wlan0"), None, None);
        h.record_apps(0, &[app("a", "steam", 0, 0)]);
        h.record_apps(MIN, &[app("a", "steam", 700, 0)]);
        assert!(h.reset_traffic(MIN + 1));
        assert_eq!(h.reset_at(), MIN + 1);
        assert!(h.traffic(0, |_| "d".into()).is_empty(), "nothing counted before the reset");
        assert!(h.app_traffic(0, u64::MAX / 2).is_empty());
        h.record(&snap(2 * MIN, 40.0, 10_000, 1_000, "wlan0"), None, None);
        h.record(&snap(3 * MIN, 40.0, 12_000, 1_100, "wlan0"), None, None);
        let d = h.traffic(0, |_| "d".into());
        assert_eq!((d[0].rx_bytes, d[0].tx_bytes), (2_000, 100), "counted from the first reading after the reset");
        assert!(h.range(0, 4 * MIN, 4).iter().any(|p| !p.net_rx_bps.is_nan()), "speed history is kept");
    }
}
