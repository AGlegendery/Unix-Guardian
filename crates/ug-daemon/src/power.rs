//! `org.unixguardian.Daemon1.Power`: shut down, restart and sleep, now or on
//! a schedule.
//!
//! The work itself is logind's (`org.freedesktop.login1`, provided by
//! systemd-logind or elogind): the daemon asks it as root, non-interactively,
//! after checking the caller with our own polkit action. A development daemon
//! (`--session` / `--sysroot`) never talks to logind: it records what it would
//! have done, which the end-to-end checks read back.

use crate::polkit::{action, Gate};
use crate::schedule::{Action, Job, Repeat, Schedule};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;
use ug_proto::{PowerCapability, PowerJob};
use zbus::object_server::SignalEmitter;

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Login1 {
    fn can_power_off(&self) -> zbus::Result<String>;
    fn can_reboot(&self) -> zbus::Result<String>;
    fn can_suspend(&self) -> zbus::Result<String>;
    fn can_hibernate(&self) -> zbus::Result<String>;
    fn can_hybrid_sleep(&self) -> zbus::Result<String>;
    fn can_suspend_then_hibernate(&self) -> zbus::Result<String>;
    fn can_reboot_to_firmware_setup(&self) -> zbus::Result<String>;
    fn set_reboot_to_firmware_setup(&self, enable: bool) -> zbus::Result<()>;
    fn power_off(&self, interactive: bool) -> zbus::Result<()>;
    fn reboot(&self, interactive: bool) -> zbus::Result<()>;
    fn suspend(&self, interactive: bool) -> zbus::Result<()>;
    fn hibernate(&self, interactive: bool) -> zbus::Result<()>;
    fn hybrid_sleep(&self, interactive: bool) -> zbus::Result<()>;
    fn suspend_then_hibernate(&self, interactive: bool) -> zbus::Result<()>;
}

/// Who carries power actions out.
pub enum Backend {
    /// logind on the system bus.
    Logind(zbus::Connection),
    /// Development: nothing happens; each action is appended here.
    Simulated(Mutex<Vec<String>>),
}

/// How late a scheduled job may still run. Beyond this the machine was
/// asleep or the daemon stalled, and the job is skipped (see
/// [`Schedule::take_due`]).
const GRACE_MS: u64 = 120_000;
/// How long before a job the `Upcoming` warning goes out.
const WARN_MS: u64 = 60_000;
/// Longest single sleep while a job is queued. Timers use the monotonic clock,
/// which stops while the machine is suspended; waking regularly re-reads the
/// wall clock so a resume is noticed within this long.
const MAX_NAP_MS: u64 = 30_000;

impl Backend {
    async fn capability(&self, a: Action) -> (bool, String) {
        let conn = match self {
            Backend::Simulated(_) => return (true, String::new()),
            Backend::Logind(c) => c,
        };
        let answer = match Login1Proxy::new(conn).await {
            Ok(p) => match a {
                Action::PowerOff => p.can_power_off().await,
                Action::Reboot => p.can_reboot().await,
                Action::Suspend => p.can_suspend().await,
                Action::Hibernate => p.can_hibernate().await,
                Action::HybridSleep => p.can_hybrid_sleep().await,
                Action::SuspendThenHibernate => p.can_suspend_then_hibernate().await,
                Action::RebootFirmware => p.can_reboot_to_firmware_setup().await,
            },
            Err(e) => Err(e),
        };
        match answer.as_deref() {
            Ok("yes") | Ok("challenge") => (true, String::new()),
            Ok("na") => (false, not_supported(a)),
            Ok("no") => (false, "The system's policy does not allow this.".into()),
            Ok(other) => (false, format!("The login manager answered {other:?}.")),
            Err(_) => (false, "The login manager (systemd-logind or elogind) is not running.".into()),
        }
    }

    async fn perform(&self, a: Action) -> Result<(), String> {
        let conn = match self {
            Backend::Simulated(log) => {
                tracing::warn!(action = a.name(), "development run: power action simulated, nothing happened");
                log.lock().unwrap_or_else(|e| e.into_inner()).push(a.name().to_string());
                return Ok(());
            }
            Backend::Logind(c) => c,
        };
        let p = Login1Proxy::new(conn).await.map_err(|e| e.to_string())?;
        // Not interactive: the caller was already authorised by our polkit
        // action, and a scheduled job has nobody to answer a prompt.
        let r = match a {
            Action::PowerOff => p.power_off(false).await,
            Action::Reboot => p.reboot(false).await,
            Action::Suspend => p.suspend(false).await,
            Action::Hibernate => p.hibernate(false).await,
            Action::HybridSleep => p.hybrid_sleep(false).await,
            Action::SuspendThenHibernate => p.suspend_then_hibernate(false).await,
            // logind sets the one-shot EFI OsIndications flag; the next boot
            // opens the firmware setup and the flag clears itself.
            Action::RebootFirmware => match p.set_reboot_to_firmware_setup(true).await {
                Ok(()) => p.reboot(false).await,
                Err(e) => Err(e),
            },
        };
        r.map_err(|e| match e {
            zbus::Error::MethodError(_, Some(msg), _) => msg,
            other => other.to_string(),
        })
    }
}

fn not_supported(a: Action) -> String {
    match a {
        Action::Hibernate | Action::HybridSleep | Action::SuspendThenHibernate => {
            "Hibernation is not set up on this system: it needs a swap area at least as large as the \
             memory in use and a resume device, and Secure Boot lockdown blocks it."
                .into()
        }
        Action::RebootFirmware => {
            "This computer does not report a firmware setup screen that can be reached from the \
             system (it needs UEFI with OsIndications support)."
                .into()
        }
        _ => format!("This system does not support {}.", a.name()),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn to_wire(j: &Job) -> PowerJob {
    PowerJob {
        id: j.id,
        action: j.action.name().to_string(),
        at_unix_ms: j.at_ms,
        repeat: j.repeat.name().to_string(),
        note: j.note.clone(),
    }
}

pub type SharedSchedule = Arc<Mutex<Schedule>>;

pub struct PowerIface {
    pub schedule: SharedSchedule,
    pub backend: Arc<Backend>,
    pub gate: Gate,
    /// Wakes the runner when the queue changes.
    pub changed: Arc<Notify>,
    /// The hardware clock; `None` in a development run on the real `/sys`,
    /// where the alarm is only simulated (kept in `simulated_alarm`).
    pub rtc: Option<crate::rtc::Rtc>,
    pub simulated_alarm: Mutex<Option<u64>>,
}

fn lock(s: &SharedSchedule) -> std::sync::MutexGuard<'_, Schedule> {
    s.lock().unwrap_or_else(|e| e.into_inner())
}

fn invalid(e: String) -> zbus::fdo::Error {
    zbus::fdo::Error::InvalidArgs(e)
}

#[zbus::interface(name = "org.unixguardian.Daemon1.Power")]
impl PowerIface {
    async fn capabilities(&self) -> Vec<PowerCapability> {
        let mut out = Vec::new();
        for a in Action::ALL {
            let (available, reason) = self.backend.capability(a).await;
            out.push(PowerCapability { action: a.name().to_string(), available, reason });
        }
        out
    }

    async fn now(&self, #[zbus(header)] hdr: zbus::message::Header<'_>, action: &str) -> zbus::fdo::Result<()> {
        let a = Action::parse(action).map_err(invalid)?;
        self.gate.authorize(&hdr, action::POWER).await?;
        let (ok, why) = self.backend.capability(a).await;
        if !ok {
            return Err(zbus::fdo::Error::NotSupported(why));
        }
        tracing::info!(action = a.name(), "power action requested now");
        self.backend.perform(a).await.map_err(zbus::fdo::Error::Failed)
    }

    async fn schedule(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        action: &str,
        at_unix_ms: u64,
        repeat: &str,
        note: &str,
    ) -> zbus::fdo::Result<PowerJob> {
        let a = Action::parse(action).map_err(invalid)?;
        let r = Repeat::parse(repeat).map_err(invalid)?;
        self.gate.authorize(&hdr, action::POWER).await?;
        let (ok, why) = self.backend.capability(a).await;
        if !ok {
            return Err(zbus::fdo::Error::NotSupported(why));
        }
        let job = lock(&self.schedule).add(a, at_unix_ms, r, note, now_ms()).map_err(invalid)?;
        tracing::info!(id = job.id, action = a.name(), at_ms = job.at_ms, repeat = r.name(), "power action scheduled");
        self.changed.notify_one();
        Ok(to_wire(&job))
    }

    async fn jobs(&self) -> Vec<PowerJob> {
        lock(&self.schedule).jobs().iter().map(to_wire).collect()
    }

    async fn cancel(&self, #[zbus(header)] hdr: zbus::message::Header<'_>, id: u32) -> zbus::fdo::Result<bool> {
        self.gate.authorize(&hdr, action::POWER).await?;
        let removed = lock(&self.schedule).cancel(id).map_err(zbus::fdo::Error::Failed)?;
        if removed {
            tracing::info!(id, "scheduled power action cancelled");
            self.changed.notify_one();
        }
        Ok(removed)
    }

    /// Wakes the computer at `at_unix_ms` (from sleep, or from off where the
    /// firmware allows it). Replaces any alarm already set.
    async fn set_wake_alarm(&self, #[zbus(header)] hdr: zbus::message::Header<'_>, at_unix_ms: u64) -> zbus::fdo::Result<()> {
        if at_unix_ms <= now_ms() + 30_000 {
            return Err(invalid("pick a time at least a minute from now".into()));
        }
        self.gate.authorize(&hdr, action::POWER).await?;
        match &self.rtc {
            Some(r) if r.available() => r.set(at_unix_ms / 1000).map_err(zbus::fdo::Error::Failed)?,
            Some(_) => return Err(zbus::fdo::Error::NotSupported("This computer has no wake-capable hardware clock.".into())),
            None => {
                *self.simulated_alarm.lock().unwrap_or_else(|e| e.into_inner()) = Some(at_unix_ms / 1000);
                if let Backend::Simulated(log) = &*self.backend {
                    log.lock().unwrap_or_else(|e| e.into_inner()).push(format!("wake-alarm {}", at_unix_ms / 1000));
                }
            }
        }
        tracing::info!(at_unix_ms, "wake alarm set");
        Ok(())
    }

    async fn clear_wake_alarm(&self, #[zbus(header)] hdr: zbus::message::Header<'_>) -> zbus::fdo::Result<()> {
        self.gate.authorize(&hdr, action::POWER).await?;
        match &self.rtc {
            Some(r) if r.available() => r.clear().map_err(zbus::fdo::Error::Failed)?,
            Some(_) => {}
            None => *self.simulated_alarm.lock().unwrap_or_else(|e| e.into_inner()) = None,
        }
        Ok(())
    }

    /// The armed wake alarm in Unix milliseconds, 0 when none.
    #[zbus(property)]
    async fn wake_alarm(&self) -> u64 {
        let s = match &self.rtc {
            Some(r) => r.get(),
            None => *self.simulated_alarm.lock().unwrap_or_else(|e| e.into_inner()),
        };
        s.map(|s| s * 1000).unwrap_or(0)
    }

    #[zbus(property)]
    async fn wake_alarm_supported(&self) -> bool {
        self.rtc.as_ref().is_none_or(|r| r.available())
    }

    #[zbus(signal)]
    pub async fn upcoming(emitter: &SignalEmitter<'_>, job: PowerJob, seconds: u32) -> zbus::Result<()>;

    #[zbus(property)]
    async fn simulated(&self) -> Vec<String> {
        match &*self.backend {
            Backend::Simulated(log) => log.lock().unwrap_or_else(|e| e.into_inner()).clone(),
            Backend::Logind(_) => Vec::new(),
        }
    }
}

/// Runs queued jobs on time. With nothing queued it parks until the queue
/// changes: no periodic work on an idle machine.
pub async fn run(conn: zbus::Connection, schedule: SharedSchedule, backend: Arc<Backend>, changed: Arc<Notify>) {
    // (job id, time) pairs already warned about, so each gets one warning.
    let mut warned: Vec<(u32, u64)> = Vec::new();
    loop {
        let next = lock(&schedule).jobs().first().cloned();
        let Some(job) = next else {
            warned.clear();
            changed.notified().await;
            continue;
        };
        let now = now_ms();
        if job.at_ms > now {
            let warn_at = job.at_ms.saturating_sub(WARN_MS);
            if now >= warn_at && !warned.contains(&(job.id, job.at_ms)) {
                warned.push((job.id, job.at_ms));
                let secs = ((job.at_ms - now) / 1000) as u32;
                if let Ok(iface) = conn.object_server().interface::<_, PowerIface>(ug_proto::DAEMON_OBJECT_PATH).await {
                    let _ = PowerIface::upcoming(iface.signal_emitter(), to_wire(&job), secs).await;
                }
                tracing::info!(id = job.id, action = job.action.name(), secs, "power action coming up");
            }
            let target = if now < warn_at { warn_at } else { job.at_ms };
            let nap = (target - now).clamp(1, MAX_NAP_MS);
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_millis(nap)) => {}
                _ = changed.notified() => {}
            }
            continue;
        }
        let (due, missed) = lock(&schedule).take_due(now, GRACE_MS);
        for j in missed {
            tracing::warn!(id = j.id, action = j.action.name(), "skipped: it fell due while the machine was asleep");
        }
        // Several jobs due together: the first one decides the machine's fate
        // (a shutdown ends everything after it), so run them in order and stop
        // at the first that takes the machine away.
        for j in due {
            tracing::info!(id = j.id, action = j.action.name(), "running scheduled power action");
            if let Err(e) = backend.perform(j.action).await {
                tracing::error!(id = j.id, error = %e, "scheduled power action failed");
                continue;
            }
            if matches!(j.action, Action::PowerOff | Action::Reboot | Action::RebootFirmware) {
                break;
            }
        }
    }
}
