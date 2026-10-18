//! "Sleep anyway": what keeps the computer awake, in plain words, and
//! locking the screen before sleeping past it.
//!
//! A program can hold a block inhibitor on sleep (logind refuses Suspend,
//! root included since systemd 256). Some say why in words people know
//! ("caffeine mode"); some exist only to route sleep through the desktop's
//! own lock screen (Ryoku's shell). Skipping them must not leave the
//! session unlocked on wake, so the screen is locked first and the lock is
//! confirmed; when it cannot be, the caller asks the user before sleeping.
//! Runs as the user (window, terminal UI, CLI), never in the root daemon,
//! which cannot reach the user's shell.

use std::time::{Duration, Instant};

/// A blocker in plain words.
pub fn describe(who: &str, why: &str) -> String {
    let w = why.to_lowercase();
    if w.contains("caffeine") {
        return format!("Caffeine mode (keep awake) is on in {who}");
    }
    if w.contains("lock") && (w.contains("suspend") || w.contains("sleep")) {
        return format!("{who} wants sleep to go through its lock screen");
    }
    if why.is_empty() {
        return who.to_string();
    }
    format!("{who}: {why}")
}

/// How the screen was locked, or why that could not be confirmed.
#[derive(Debug, PartialEq)]
pub enum Locked {
    /// Locked and confirmed; names the way ("Ryoku", "the login manager").
    Yes(&'static str),
    No(String),
}

/// Runs a program: `Some(stdout)` when it ran and succeeded.
pub type Run<'a> = &'a dyn Fn(&str, &[&str]) -> Option<String>;
/// Starts a program without waiting for it.
pub type Start<'a> = &'a dyn Fn(&str, &[&str]) -> bool;

/// Locks the screen and waits up to `timeout` for the lock to be confirmed.
/// Ryoku's shell first (its own lock screen, the one it wants before sleep),
/// else logind's session lock, confirmed by LockedHint.
pub fn lock_screen(run: Run, start: Start, timeout: Duration) -> Locked {
    let poll = |check: &dyn Fn() -> bool| {
        let end = Instant::now() + timeout;
        loop {
            if check() {
                return true;
            }
            if Instant::now() >= end {
                return false;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    };
    if let Some(st) = run("ryoku-shell", &["lock", "status"]) {
        if st.trim() == "locked" {
            return Locked::Yes("Ryoku");
        }
        if !start("ryoku-shell", &["lock"]) {
            return Locked::No("Ryoku's lock screen could not be started.".into());
        }
        let ok =
            poll(&|| run("ryoku-shell", &["lock", "status"]).is_some_and(|s| s.trim() == "locked"));
        return if ok {
            Locked::Yes("Ryoku")
        } else {
            Locked::No("Ryoku's lock screen did not confirm it is locked.".into())
        };
    }
    let hint = || {
        run(
            "loginctl",
            &["show-session", "self", "-p", "LockedHint", "--value"],
        )
        .is_some_and(|s| s.trim() == "yes")
    };
    if hint() {
        return Locked::Yes("the login manager");
    }
    if run("loginctl", &["lock-session"]).is_none() {
        return Locked::No("The login manager could not lock this session.".into());
    }
    if poll(&hint) {
        Locked::Yes("the login manager")
    } else {
        Locked::No("The desktop did not confirm the screen is locked.".into())
    }
}

/// [`lock_screen`] with real programs, waiting at most five seconds.
pub fn lock_screen_now() -> Locked {
    let run = |prog: &str, args: &[&str]| {
        let out = std::process::Command::new(prog).args(args).output().ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let start = |prog: &str, args: &[&str]| {
        match std::process::Command::new(prog)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(mut c) => {
                // The lock client may run until unlocked: reap it later.
                std::thread::spawn(move || c.wait());
                true
            }
            Err(_) => false,
        }
    };
    lock_screen(&run, &start, Duration::from_secs(5))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn known_blockers_read_as_plain_words() {
        assert_eq!(
            describe("Ryoku", "Ryoku caffeine mode"),
            "Caffeine mode (keep awake) is on in Ryoku"
        );
        assert_eq!(
            describe(
                "ryoku-shell",
                "route suspend through a compositor-secure session lock"
            ),
            "ryoku-shell wants sleep to go through its lock screen"
        );
        assert_eq!(describe("vlc", "Playing a video"), "vlc: Playing a video");
    }

    #[test]
    fn ryoku_locks_first_and_must_confirm() {
        let locked = RefCell::new(false);
        let run = |p: &str, a: &[&str]| match (p, a) {
            ("ryoku-shell", ["lock", "status"]) => Some(
                if *locked.borrow() {
                    "locked\n"
                } else {
                    "unlocked\n"
                }
                .into(),
            ),
            _ => None,
        };
        let start = |p: &str, a: &[&str]| {
            assert_eq!((p, a), ("ryoku-shell", &["lock"][..]));
            *locked.borrow_mut() = true;
            true
        };
        assert_eq!(
            lock_screen(&run, &start, Duration::from_millis(600)),
            Locked::Yes("Ryoku")
        );
        // A lock screen that never says locked: not confirmed, never silent.
        let never = |p: &str, a: &[&str]| match (p, a) {
            ("ryoku-shell", ["lock", "status"]) => Some("unlocked".into()),
            _ => None,
        };
        let ok = |_: &str, _: &[&str]| true;
        assert!(matches!(
            lock_screen(&never, &ok, Duration::from_millis(300)),
            Locked::No(_)
        ));
    }

    #[test]
    fn without_ryoku_the_session_lock_is_confirmed_by_locked_hint() {
        let locked = RefCell::new(false);
        let run = |p: &str, a: &[&str]| match (p, a.first().copied()) {
            ("loginctl", Some("lock-session")) => {
                *locked.borrow_mut() = true;
                Some(String::new())
            }
            ("loginctl", Some("show-session")) => {
                Some(if *locked.borrow() { "yes" } else { "no" }.into())
            }
            _ => None,
        };
        let start = |_: &str, _: &[&str]| false;
        assert_eq!(
            lock_screen(&run, &start, Duration::from_millis(600)),
            Locked::Yes("the login manager")
        );
        let deaf = |p: &str, a: &[&str]| match (p, a.first().copied()) {
            ("loginctl", Some("lock-session")) => Some(String::new()),
            ("loginctl", Some("show-session")) => Some("no".into()),
            _ => None,
        };
        assert!(matches!(
            lock_screen(&deaf, &start, Duration::from_millis(300)),
            Locked::No(_)
        ));
    }
}
