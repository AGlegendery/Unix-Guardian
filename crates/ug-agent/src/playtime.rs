//! Playtime history in SQLite.
//!
//! A session row is written the moment a game starts, then its end time is
//! moved forward by a heartbeat while it runs and set exactly when it stops.
//! If the agent is killed mid-game, at most one heartbeat interval of play is
//! lost — the row is already there with a recent end time.
//!
//! Imports (Steam's own totals) are not sessions: they have no dates. Each
//! game's import is what the source counted *beyond* the sessions recorded
//! here, so time played since Unix Guardian was installed is never counted
//! twice, and importing again simply replaces the last import. They add to
//! the all-time totals only; today and this week stay what was recorded.

use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;
use ug_proto::{PlaytimeEntry, PlaytimeSession};

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
             CREATE INDEX IF NOT EXISTS sessions_ended ON sessions(ended_ms);
             CREATE TABLE IF NOT EXISTS imported (
                 key            TEXT    PRIMARY KEY,
                 name           TEXT    NOT NULL,
                 ms             INTEGER NOT NULL,
                 last_played_ms INTEGER NOT NULL,
                 source         TEXT    NOT NULL
             );",
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
    /// before `since_ms` count only from `since_ms` on. All time (`0`)
    /// includes imports.
    pub fn summary(&self, since_ms: u64) -> Vec<PlaytimeEntry> {
        let mut v = self.recorded(since_ms);
        if since_ms == 0 {
            for i in self.imported() {
                match v.iter_mut().find(|e| e.key == i.key) {
                    Some(e) => {
                        e.seconds += i.seconds;
                        e.last_played_unix_ms = e.last_played_unix_ms.max(i.last_played_unix_ms);
                    }
                    None => v.push(i),
                }
            }
            v.sort_by(|a, b| b.seconds.cmp(&a.seconds).then_with(|| a.key.cmp(&b.key)));
        }
        v
    }

    fn recorded(&self, since_ms: u64) -> Vec<PlaytimeEntry> {
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

    /// Every session that ended after `since_ms`, oldest first.
    pub fn sessions(&self, since_ms: u64) -> Vec<PlaytimeSession> {
        let c = self.conn();
        let mut stmt = match c.prepare(
            "SELECT key, name, started_ms, ended_ms FROM sessions WHERE ended_ms > ?1 ORDER BY started_ms, id",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "playtime query failed");
                return Vec::new();
            }
        };
        stmt.query_map(params![since_ms as i64], |r| {
            Ok(PlaytimeSession {
                key: r.get(0)?,
                name: r.get(1)?,
                started_unix_ms: r.get::<_, i64>(2)?.max(0) as u64,
                ended_unix_ms: r.get::<_, i64>(3)?.max(0) as u64,
            })
        })
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }

    /// Deletes every session of one game, and its import; returns how many
    /// sessions.
    pub fn forget(&self, key: &str) -> u32 {
        let c = self.conn();
        let _ = c.execute("DELETE FROM imported WHERE key = ?1", params![key]);
        c.execute("DELETE FROM sessions WHERE key = ?1", params![key]).unwrap_or(0) as u32
    }

    /// Files a game's history under another key ("not a game" → app).
    /// Returns the sessions moved.
    pub fn move_key(&self, from: &str, to: &str) -> u32 {
        self.conn()
            .execute("UPDATE sessions SET key = ?2 WHERE key = ?1", params![from, to])
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, from, to, "could not move playtime");
                0
            }) as u32
    }

    /// Replaces the import from `source` with these totals: (key, name,
    /// seconds the source counted in all, last played). Each game keeps only
    /// what the source counted beyond the sessions recorded here. Returns
    /// (games with time added, seconds added).
    pub fn import(&self, source: &str, games: &[(String, String, u64, u64)]) -> rusqlite::Result<(u32, u64)> {
        let mut c = self.conn();
        let tx = c.transaction()?;
        tx.execute("DELETE FROM imported WHERE source = ?1", params![source])?;
        let (mut n, mut added_ms) = (0u32, 0u64);
        for (key, name, seconds, last_played_ms) in games {
            let recorded: i64 = tx.query_row(
                "SELECT COALESCE(SUM(ended_ms - started_ms), 0) FROM sessions WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )?;
            let extra = (*seconds as i64 * 1000).saturating_sub(recorded.max(0));
            if extra <= 0 {
                continue;
            }
            tx.execute(
                "INSERT OR REPLACE INTO imported (key, name, ms, last_played_ms, source) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![key, name, extra, *last_played_ms as i64, source],
            )?;
            n += 1;
            added_ms += extra as u64;
        }
        tx.commit()?;
        Ok((n, added_ms / 1000))
    }

    /// What imports added, per game, most first. `sessions` is 0.
    pub fn imported(&self) -> Vec<PlaytimeEntry> {
        let c = self.conn();
        let Ok(mut stmt) = c.prepare("SELECT key, name, ms, last_played_ms FROM imported ORDER BY ms DESC, key") else {
            return Vec::new();
        };
        stmt.query_map([], |r| {
            Ok(PlaytimeEntry {
                key: r.get(0)?,
                name: r.get(1)?,
                seconds: (r.get::<_, i64>(2)?.max(0) / 1000) as u64,
                sessions: 0,
                last_played_unix_ms: r.get::<_, i64>(3)?.max(0) as u64,
            })
        })
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }

    /// Removes every import; returns how many games had one.
    pub fn forget_imported(&self) -> u32 {
        self.conn().execute("DELETE FROM imported", []).unwrap_or(0) as u32
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
    fn sessions_are_listed_oldest_first_and_filtered_by_their_end() {
        let p = Playtime::in_memory();
        let late = p.start("b", "B", 5 * H);
        p.touch(late, 6 * H);
        let early = p.start("a", "A", 0);
        p.touch(early, H);
        let all = p.sessions(0);
        assert_eq!(all.iter().map(|s| s.key.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!((all[1].started_unix_ms, all[1].ended_unix_ms), (5 * H, 6 * H));
        assert_eq!(p.sessions(2 * H).len(), 1, "a session over before the window is left out");
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
    fn an_import_adds_only_what_was_not_recorded_here_and_only_to_all_time() {
        let p = Playtime::in_memory();
        // One hour recorded here; Steam has counted ten in all.
        let a = p.start("steam:730", "Counter-Strike 2", 100 * H);
        p.touch(a, 101 * H);
        let games = [
            ("steam:730".to_string(), "Counter-Strike 2".to_string(), 10 * 3600, 101 * H),
            ("steam:413150".to_string(), "Stardew Valley".to_string(), 2 * 3600, 50 * H),
        ];
        assert_eq!(p.import("steam", &games).unwrap(), (2, 9 * 3600 + 2 * 3600));

        let all = p.summary(0);
        assert_eq!((all[0].key.as_str(), all[0].seconds, all[0].sessions), ("steam:730", 10 * 3600, 1), "not 11 hours");
        assert_eq!((all[1].key.as_str(), all[1].seconds, all[1].sessions), ("steam:413150", 2 * 3600, 0));
        assert_eq!(p.summary(100 * H).len(), 1, "a period only has what was recorded in it");
        assert_eq!(p.summary(100 * H)[0].seconds, 3600);

        // Importing again after more play replaces, never adds up.
        let b = p.start("steam:730", "Counter-Strike 2", 102 * H);
        p.touch(b, 103 * H);
        let again = [("steam:730".to_string(), "Counter-Strike 2".to_string(), 11 * 3600, 103 * H)];
        assert_eq!(p.import("steam", &again).unwrap(), (1, 9 * 3600));
        let all = p.summary(0);
        assert_eq!(all.len(), 1, "a game no longer in the import is gone from it");
        assert_eq!(all[0].seconds, 11 * 3600);

        assert_eq!(p.forget_imported(), 1);
        assert_eq!(p.summary(0)[0].seconds, 2 * 3600);
    }

    #[test]
    fn a_game_recorded_longer_here_than_by_steam_imports_nothing() {
        let p = Playtime::in_memory();
        let a = p.start("steam:730", "CS2", 0);
        p.touch(a, 2 * H);
        assert_eq!(p.import("steam", &[("steam:730".into(), "CS2".into(), 3600, H)]).unwrap(), (0, 0));
        assert!(p.imported().is_empty());
    }

    #[test]
    fn moving_a_games_history_keeps_every_session() {
        let p = Playtime::in_memory();
        for i in 0..3 {
            let id = p.start("exe:wondershare filmora", "Wondershare Filmora", i * H);
            p.touch(id, i * H + H / 2);
        }
        assert_eq!(p.move_key("exe:wondershare filmora", "app:wondershare filmora"), 3);
        let s = p.summary(0);
        assert_eq!((s[0].key.as_str(), s[0].seconds, s[0].sessions), ("app:wondershare filmora", 3 * 1800, 3));
    }

    #[test]
    fn forgetting_a_game_also_forgets_its_import() {
        let p = Playtime::in_memory();
        p.import("steam", &[("steam:730".into(), "CS2".into(), 3600, H)]).unwrap();
        p.forget("steam:730");
        assert!(p.summary(0).is_empty());
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
