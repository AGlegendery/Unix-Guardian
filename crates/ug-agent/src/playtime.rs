//! Playtime history in SQLite.
//!
//! A session row is written the moment a game starts, then its end time is
//! moved forward by a heartbeat while it runs and set exactly when it stops.
//! If the agent is killed mid-game, at most one heartbeat interval of play is
//! lost — the row is already there with a recent end time.

use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;
use ug_proto::PlaytimeEntry;

pub struct Playtime {
    conn: Mutex<Connection>,
}

impl Playtime {
    /// Opens (creating if needed) the database at `path`.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
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
             CREATE TABLE IF NOT EXISTS sessions (
                 id         INTEGER PRIMARY KEY,
                 key        TEXT    NOT NULL,
                 name       TEXT    NOT NULL,
                 started_ms INTEGER NOT NULL,
                 ended_ms   INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS sessions_key   ON sessions(key);
             CREATE INDEX IF NOT EXISTS sessions_ended ON sessions(ended_ms);",
        )?;
        Ok(Playtime { conn: Mutex::new(conn) })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Records a session starting now; returns its id for later updates.
    pub fn start(&self, key: &str, name: &str, started_ms: u64) -> i64 {
        let c = self.conn();
        match c.execute(
            "INSERT INTO sessions (key, name, started_ms, ended_ms) VALUES (?1, ?2, ?3, ?3)",
            params![key, name, started_ms as i64],
        ) {
            Ok(_) => c.last_insert_rowid(),
            Err(e) => {
                tracing::warn!(error = %e, key, "could not record session start");
                -1
            }
        }
    }

    /// Moves a running session's end forward. Also used for the final stop.
    pub fn touch(&self, id: i64, ended_ms: u64) {
        if id < 0 {
            return;
        }
        if let Err(e) = self.conn().execute(
            "UPDATE sessions SET ended_ms = MAX(ended_ms, ?2) WHERE id = ?1",
            params![id, ended_ms as i64],
        ) {
            tracing::warn!(error = %e, id, "could not update session");
        }
    }

    /// Time per game since `since_ms`, most played first. Sessions that began
    /// before `since_ms` count only from `since_ms` on.
    pub fn summary(&self, since_ms: u64) -> Vec<PlaytimeEntry> {
        let c = self.conn();
        let mut stmt = match c.prepare(
            "SELECT s.key,
                    (SELECT name FROM sessions n WHERE n.key = s.key ORDER BY n.ended_ms DESC LIMIT 1),
                    SUM(s.ended_ms - MAX(s.started_ms, ?1)),
                    COUNT(*),
                    MAX(s.ended_ms)
               FROM sessions s
              WHERE s.ended_ms > ?1
           GROUP BY s.key
           ORDER BY 3 DESC",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "playtime query failed");
                return Vec::new();
            }
        };
        stmt.query_map(params![since_ms as i64], |r| {
            Ok(PlaytimeEntry {
                key: r.get(0)?,
                name: r.get(1)?,
                seconds: (r.get::<_, i64>(2)?.max(0) / 1000) as u64,
                sessions: r.get::<_, i64>(3)? as u32,
                last_played_unix_ms: r.get::<_, i64>(4)? as u64,
            })
        })
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }

    /// Deletes every session of one game; returns how many.
    pub fn forget(&self, key: &str) -> u32 {
        self.conn()
            .execute("DELETE FROM sessions WHERE key = ?1", params![key])
            .unwrap_or(0) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: u64 = 3_600_000;

    #[test]
    fn sums_sessions_per_game_most_played_first() {
        let p = Playtime::in_memory();
        let a = p.start("steam:730", "Counter-Strike 2", 0);
        p.touch(a, 2 * H);
        let b = p.start("steam:1091500", "Cyberpunk 2077", 3 * H);
        p.touch(b, 3 * H + H / 2);
        let c = p.start("steam:730", "Counter-Strike 2", 5 * H);
        p.touch(c, 6 * H);

        let s = p.summary(0);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].key, "steam:730");
        assert_eq!(s[0].seconds, 3 * 3600);
        assert_eq!(s[0].sessions, 2);
        assert_eq!(s[0].last_played_unix_ms, 6 * H);
        assert_eq!(s[1].seconds, 1800);
    }

    #[test]
    fn a_session_spanning_the_window_start_counts_only_its_later_part() {
        let p = Playtime::in_memory();
        let id = p.start("steam:730", "CS2", 0);
        p.touch(id, 10 * H);
        let s = p.summary(8 * H);
        assert_eq!(s[0].seconds, 2 * 3600, "'today' must not include last night's first eight hours");
    }

    #[test]
    fn the_end_time_never_moves_backwards() {
        let p = Playtime::in_memory();
        let id = p.start("k", "n", 0);
        p.touch(id, 5 * H);
        p.touch(id, 2 * H); // a late heartbeat racing the final stop
        assert_eq!(p.summary(0)[0].seconds, 5 * 3600);
    }

    #[test]
    fn the_latest_name_is_shown_after_a_rename() {
        let p = Playtime::in_memory();
        let a = p.start("steam:1172620", "Sea of Thieves", 0);
        p.touch(a, H);
        let b = p.start("steam:1172620", "Sea of Thieves: 2026 Edition", 2 * H);
        p.touch(b, 3 * H);
        assert_eq!(p.summary(0)[0].name, "Sea of Thieves: 2026 Edition");
    }

    #[test]
    fn forgetting_a_game_removes_only_that_game() {
        let p = Playtime::in_memory();
        let a = p.start("a", "A", 0);
        p.touch(a, H);
        let b = p.start("b", "B", 0);
        p.touch(b, H);
        assert_eq!(p.forget("a"), 1);
        let s = p.summary(0);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].key, "b");
    }

    #[test]
    fn a_session_survives_reopening_the_database() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("nested/agent.db");
        {
            let p = Playtime::open(&path).unwrap();
            let id = p.start("steam:730", "CS2", 0);
            p.touch(id, H); // last heartbeat before an unclean exit
        }
        let p = Playtime::open(&path).unwrap();
        assert_eq!(p.summary(0)[0].seconds, 3600);
    }
}
