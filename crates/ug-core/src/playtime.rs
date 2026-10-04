//! Playtime helpers shared by the CLI, the TUI and the window: the ranges
//! the pages offer, local dates, and the CSV export.
//!
//! The sessions themselves are recorded by the agent (`Agent1.Playtime`);
//! everything here only shapes what it returns.

use std::path::{Path, PathBuf};
use ug_proto::PlaytimeSession;

/// The ranges the Playtime pages offer: today (since local midnight), the
/// last seven days, everything.
pub const RANGES: [&str; 3] = ["today", "week", "all"];

/// Where a range starts, as Unix milliseconds.
pub fn since(range: &str, now_ms: u64) -> u64 {
    match range {
        "today" => local_midnight_ms(),
        "week" => now_ms.saturating_sub(7 * 24 * 3_600_000),
        _ => 0,
    }
}

/// Start of today in local time, as Unix milliseconds.
pub fn local_midnight_ms() -> u64 {
    // SAFETY: localtime_r writes only into the struct we pass.
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        tm.tm_hour = 0;
        tm.tm_min = 0;
        tm.tm_sec = 0;
        (libc::mktime(&mut tm).max(0) as u64) * 1000
    }
}

/// Local midnight of a `YYYY-MM-DD` date, and of the day after (its end;
/// a day with a clock change is 23 or 25 hours), as Unix milliseconds.
pub fn local_day_ms(date: &str) -> Option<(u64, u64)> {
    let mut it = date.split('-').map(|p| p.parse::<i32>().ok());
    let (Some(Some(y)), Some(Some(m)), Some(Some(d)), None) = (it.next(), it.next(), it.next(), it.next()) else {
        return None;
    };
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let at = |day: i32| {
        // SAFETY: mktime only reads and normalises the struct we pass.
        unsafe {
            let mut tm: libc::tm = std::mem::zeroed();
            tm.tm_year = y - 1900;
            tm.tm_mon = m - 1;
            tm.tm_mday = day;
            tm.tm_isdst = -1;
            libc::mktime(&mut tm)
        }
    };
    let (a, b) = (at(d), at(d + 1));
    (a >= 0 && b > a).then(|| (a as u64 * 1000, b as u64 * 1000))
}

/// Local midnight `days` days after the day holding `unix_ms` (negative:
/// before), as Unix milliseconds; clock changes give 23- or 25-hour days.
pub fn local_midnight_offset(unix_ms: u64, days: i32) -> u64 {
    // SAFETY: localtime_r and mktime only touch the struct we pass.
    unsafe {
        let t = (unix_ms / 1000) as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm.tm_hour = 0;
        tm.tm_min = 0;
        tm.tm_sec = 0;
        tm.tm_mday += days;
        tm.tm_isdst = -1;
        (libc::mktime(&mut tm).max(0) as u64) * 1000
    }
}

/// The local hour (0–23) of a time.
fn local_hour(unix_ms: u64) -> usize {
    // SAFETY: localtime_r writes only into the struct we pass.
    unsafe {
        let t = (unix_ms / 1000) as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm.tm_hour.clamp(0, 23) as usize
    }
}

/// Seconds of a session inside `[a, b)`.
pub fn overlap_s(s: &PlaytimeSession, a: u64, b: u64) -> u64 {
    s.ended_unix_ms.min(b).saturating_sub(s.started_unix_ms.max(a)) / 1000
}

/// Seconds played on each of the `n` local days ending today: (that day's
/// midnight, seconds). A session over midnight counts on both days.
pub fn per_day(sessions: &[PlaytimeSession], now_ms: u64, n: u32) -> Vec<(u64, u64)> {
    let n = n.max(1) as i32;
    (0..n)
        .map(|i| {
            let a = local_midnight_offset(now_ms, i - n + 1);
            let b = local_midnight_offset(now_ms, i - n + 2);
            (a, sessions.iter().map(|s| overlap_s(s, a, b)).sum())
        })
        .collect()
}

/// Seconds played in each local hour of the day, summed over the sessions.
pub fn per_hour(sessions: &[PlaytimeSession]) -> [u64; 24] {
    let mut out = [0u64; 24];
    for s in sessions {
        let mut t = s.started_unix_ms;
        while t < s.ended_unix_ms {
            // The next whole hour (every zone's offset here is whole minutes,
            // so stepping to the next minute boundary of a whole hour works).
            let next = ((t / 3_600_000) + 1) * 3_600_000;
            let next = (t..=next).step_by(60_000).find(|m| *m > t && local_hour(*m) != local_hour(t)).unwrap_or(next);
            let end = next.min(s.ended_unix_ms);
            out[local_hour(t)] += (end - t) / 1000;
            t = end;
        }
    }
    out
}

/// Days in a row with at least a minute played, ending today (or
/// yesterday, when nothing was played yet today).
pub fn streak(days: &[(u64, u64)]) -> u32 {
    let mut n = 0;
    for (i, (_, secs)) in days.iter().enumerate().rev() {
        if *secs >= 60 {
            n += 1;
        } else if i + 1 == days.len() {
            continue;
        } else {
            break;
        }
    }
    n
}

/// `2026-09-30 21:04:05` in local time, which spreadsheets read as a date.
pub fn local_time(unix_ms: u64) -> String {
    // SAFETY: localtime_r writes only into the struct we pass.
    let tm = unsafe {
        let t = (unix_ms / 1000) as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

/// A CSV field, quoted only when it has to be.
pub fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// The export: one row per session.
pub fn csv(sessions: &[PlaytimeSession]) -> String {
    let mut out = String::from("game,key,started,ended,minutes\n");
    for s in sessions {
        out.push_str(&csv_row(s));
        out.push('\n');
    }
    out
}

pub fn csv_row(s: &PlaytimeSession) -> String {
    let minutes = s.ended_unix_ms.saturating_sub(s.started_unix_ms) as f64 / 60_000.0;
    format!(
        "{},{},{},{},{minutes:.1}",
        csv_field(&s.name),
        csv_field(&s.key),
        local_time(s.started_unix_ms),
        local_time(s.ended_unix_ms)
    )
}

/// The user's Downloads folder (from `user-dirs.dirs`), else their home.
pub fn downloads_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    let dirs = crate::gpu_default::config_home().join("user-dirs.dirs");
    std::fs::read_to_string(dirs)
        .ok()
        .and_then(|t| downloads_from(&t, &home))
        .filter(|p| p.is_dir())
        .unwrap_or(home)
}

fn downloads_from(user_dirs: &str, home: &Path) -> Option<PathBuf> {
    let v = user_dirs.lines().find_map(|l| l.trim().strip_prefix("XDG_DOWNLOAD_DIR="))?;
    let v = v.trim().trim_matches('"');
    Some(match v.strip_prefix("$HOME") {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None => PathBuf::from(v),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(start: u64, end: u64) -> PlaytimeSession {
        PlaytimeSession { key: "steam:730".into(), name: "CS2".into(), started_unix_ms: start, ended_unix_ms: end }
    }

    #[test]
    fn days_and_hours_split_a_session_where_it_crosses_them() {
        let (today, _) = local_day_ms(&local_time(local_midnight_ms())[..10]).unwrap();
        let yesterday = local_midnight_offset(today, -1);
        // 23:30 yesterday to 00:45 today, and 10:00–10:30 today.
        let s = [session(today - 1_800_000, today + 2_700_000), session(today + 36_000_000, today + 37_800_000)];
        let d = per_day(&s, today + 40_000_000, 3);
        assert_eq!(d.len(), 3);
        assert_eq!(d[1], (yesterday, 1_800));
        assert_eq!(d[2], (today, 2_700 + 1_800));
        let h = per_hour(&s);
        assert_eq!((h[23], h[0], h[10]), (1_800, 2_700, 1_800));
        assert_eq!(h.iter().sum::<u64>(), 6_300);
        assert_eq!(streak(&d), 2);
        assert_eq!(streak(&[(0, 600), (1, 0)]), 1, "nothing yet today keeps yesterday's streak");
        assert_eq!(streak(&[(0, 600), (1, 0), (2, 0)]), 0);
    }

    #[test]
    fn a_local_day_is_found_from_its_date() {
        let (a, b) = local_day_ms("2026-10-04").unwrap();
        assert!((23 * 3_600_000..=25 * 3_600_000).contains(&(b - a)));
        assert_eq!(&local_time(a)[..], "2026-10-04 00:00:00");
        for bad in ["2026-13-01", "2026-10", "x", "2026-10-04-1"] {
            assert!(local_day_ms(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn csv_fields_are_quoted_only_when_needed() {
        assert_eq!(csv_field("Counter-Strike 2"), "Counter-Strike 2");
        assert_eq!(csv_field("Warhammer 40,000"), "\"Warhammer 40,000\"");
        assert_eq!(csv_field("The \"Game\""), "\"The \"\"Game\"\"\"");
    }

    #[test]
    fn the_export_has_a_header_and_a_row_per_session() {
        let s = PlaytimeSession { key: "steam:730".into(), name: "Counter-Strike 2".into(), started_unix_ms: 0, ended_unix_ms: 90_000 };
        let text = csv(&[s.clone(), s]);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "game,key,started,ended,minutes");
        assert_eq!(lines.len(), 3);
        assert!(lines[1].starts_with("Counter-Strike 2,steam:730,") && lines[1].ends_with(",1.5"));
    }

    #[test]
    fn downloads_come_from_user_dirs() {
        let home = Path::new("/home/u");
        assert_eq!(downloads_from("XDG_DOWNLOAD_DIR=\"$HOME/Downloads\"\n", home), Some(PathBuf::from("/home/u/Downloads")));
        assert_eq!(downloads_from("XDG_DOWNLOAD_DIR=\"/data/dl\"", home), Some(PathBuf::from("/data/dl")));
        assert_eq!(downloads_from("# nothing", home), None);
    }

    #[test]
    fn ranges_start_where_they_say() {
        assert_eq!(since("all", 1_000_000_000), 0);
        assert_eq!(since("week", 1_000_000_000), 1_000_000_000 - 604_800_000);
        assert!(since("today", 0) <= local_midnight_ms());
    }
}
