//! The hardware clock's wake alarm: "wake the computer at 7:00".
//!
//! `/sys/class/rtc/rtc0/wakealarm` takes seconds since the epoch *as the RTC
//! counts them*. Most Linux systems keep the RTC in UTC, where that is plain
//! Unix time; dual-boot machines often keep it in local time (`LOCAL` on the
//! third line of `/etc/adjtime`), where the RTC runs ahead or behind by the
//! UTC offset, and a naive write wakes the machine hours off. The kernel also
//! refuses to replace an armed alarm, so a new one is written after a `0`.

use std::io::Write;
use ug_hw::SysRoot;

pub struct Rtc {
    root: SysRoot,
}

const ALARM: &str = "sys/class/rtc/rtc0/wakealarm";

impl Rtc {
    pub fn new(root: SysRoot) -> Rtc {
        Rtc { root }
    }

    pub fn available(&self) -> bool {
        self.root.join(ALARM).exists()
    }

    /// The armed alarm as Unix seconds, if any.
    pub fn get(&self) -> Option<u64> {
        let raw: u64 = std::fs::read_to_string(self.root.join(ALARM))
            .ok()?
            .trim()
            .parse()
            .ok()?;
        if raw == 0 {
            return None;
        }
        Some((raw as i64 - self.rtc_offset(raw as i64)) as u64)
    }

    /// Arms the alarm for `at_unix_s`, replacing any armed one.
    pub fn set(&self, at_unix_s: u64) -> Result<(), String> {
        let raw = at_unix_s as i64 + self.rtc_offset(at_unix_s as i64);
        self.write("0")?;
        self.write(&raw.to_string())
    }

    pub fn clear(&self) -> Result<(), String> {
        self.write("0")
    }

    fn write(&self, v: &str) -> Result<(), String> {
        let path = self.root.join(ALARM);
        std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&path)
            .and_then(|mut f| f.write_all(v.as_bytes()))
            .map_err(|e| write_error(&path, &e))
    }

    /// Seconds the RTC runs ahead of UTC: 0 for a UTC clock, the local UTC
    /// offset at that moment for a local-time one.
    fn rtc_offset(&self, at_unix_s: i64) -> i64 {
        let local = std::fs::read_to_string(self.root.join("etc/adjtime"))
            .ok()
            .and_then(|s| s.lines().nth(2).map(|l| l.trim() == "LOCAL"))
            .unwrap_or(false);
        if !local {
            return 0;
        }
        // SAFETY: localtime_r writes only into the struct we pass.
        unsafe {
            let t = at_unix_s as libc::time_t;
            let mut tm: libc::tm = std::mem::zeroed();
            libc::localtime_r(&t, &mut tm);
            tm.tm_gmtoff
        }
    }
}

/// What a failed write means. EINVAL is the clock refusing the time: PC
/// clocks (rtc_cmos) often take alarms only within the next 24 hours.
fn write_error(path: &std::path::Path, e: &std::io::Error) -> String {
    if e.raw_os_error() == Some(libc::EINVAL) {
        format!("This computer's clock cannot wake it at that time (many can only wake within the next 24 hours); pick a time sooner. ({}: {e})", path.display())
    } else {
        format!("{}: {e}", path.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    fn rtc(fs: &TestFs, adjtime: &str) -> Rtc {
        fs.write(ALARM, "");
        fs.write("etc/adjtime", adjtime);
        Rtc::new(SysRoot::at(fs.root()))
    }

    #[test]
    fn a_utc_clock_gets_plain_unix_time_and_reads_it_back() {
        let fs = TestFs::new();
        let r = rtc(&fs, "0.0 0 0.0\n0\nUTC\n");
        assert!(r.available());
        assert_eq!(r.get(), None, "nothing armed");
        r.set(1_800_000_000).unwrap();
        assert_eq!(
            std::fs::read_to_string(fs.root().join(ALARM)).unwrap(),
            "1800000000"
        );
        assert_eq!(r.get(), Some(1_800_000_000));
        r.clear().unwrap();
        assert_eq!(r.get(), None, "cleared");
    }

    #[test]
    fn a_local_time_clock_is_shifted_by_the_utc_offset() {
        let fs = TestFs::new();
        let r = rtc(&fs, "0.0 0 0.0\n0\nLOCAL\n");
        r.set(1_800_000_000).unwrap();
        let raw: i64 = std::fs::read_to_string(fs.root().join(ALARM))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(raw - 1_800_000_000, r.rtc_offset(1_800_000_000));
        assert_eq!(
            r.get(),
            Some(1_800_000_000),
            "reads back as the same moment"
        );
    }

    #[test]
    fn a_machine_without_an_rtc_says_so() {
        let fs = TestFs::new();
        assert!(!Rtc::new(SysRoot::at(fs.root())).available());
    }

    #[test]
    fn a_time_the_clock_refuses_is_explained() {
        let e = std::io::Error::from_raw_os_error(libc::EINVAL);
        let m = write_error(std::path::Path::new("/sys/class/rtc/rtc0/wakealarm"), &e);
        assert!(m.contains("within the next 24 hours"), "{m}");
        let other = write_error(
            std::path::Path::new("/x"),
            &std::io::Error::from_raw_os_error(libc::EACCES),
        );
        assert!(other.starts_with("/x: "), "{other}");
    }
}
