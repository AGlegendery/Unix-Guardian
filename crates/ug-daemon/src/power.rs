//! `org.unixguardian.Daemon1.Power`: shut down, restart and sleep, now or on
//! a schedule.
//!
//! The work itself is logind's (`org.freedesktop.login1`, provided by
//! systemd-logind or elogind): the daemon asks it as root, non-interactively,
//! after checking the caller with our own polkit action. A development daemon
//! (`--session` / `--sysroot`) never talks to logind: it records what it would
//! have done, which the end-to-end checks read back.

use crate::conditions::{Condition, Observation, Watch, Watches};
use crate::polkit::{action, Gate};
use crate::schedule::{Action, Job, Repeat, Schedule};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;
use ug_hw::SysRoot;
use ug_proto::{BatteryCare, DayEnergy, PowerCapability, PowerJob, PowerUsage, PowerWatch};
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
            Err(_) => (
                false,
                "The login manager (systemd-logind or elogind) is not running.".into(),
            ),
        }
    }

    async fn perform(&self, a: Action) -> Result<(), String> {
        let conn = match self {
            Backend::Simulated(log) => {
                tracing::warn!(
                    action = a.name(),
                    "development run: power action simulated, nothing happened"
                );
                log.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(a.name().to_string());
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
    pub watches: SharedWatches,
    pub observer: Arc<Observer>,
    /// Wakes the condition loop when a watch is added.
    pub watches_changed: Arc<Notify>,
    pub energy: Arc<Mutex<crate::energy::EnergyMeter>>,
    pub battery: SharedBattery,
}

pub type SharedBattery = Arc<Mutex<crate::battery::Battery>>;

fn battery_to_wire(b: &crate::battery::Battery) -> BatteryCare {
    use crate::battery::Care;
    let (mode, limit) = match b.care {
        Care::Off => ("off", 100),
        Care::Protect { limit } => ("protect", limit),
        Care::FullOnce { limit } => ("full-once", limit),
    };
    let (capacity, status) = b.level().unwrap_or((255, String::new()));
    BatteryCare {
        supported: b.method.is_some(),
        method: b
            .method
            .as_ref()
            .map(|m| m.name().to_string())
            .unwrap_or_default(),
        adjustable: b.method.as_ref().is_some_and(|m| m.adjustable()),
        mode: mode.into(),
        limit,
        capacity,
        status,
        reason: if b.method.is_some() {
            String::new()
        } else {
            crate::battery::unsupported_reason().into()
        },
    }
}

pub type SharedWatches = Arc<Mutex<Watches>>;

/// Gathers what conditions look at. Reads only when a watch exists.
pub struct Observer {
    pub sampler: Arc<crate::sampler::Sampler>,
    pub game_active: Arc<AtomicBool>,
    pub root: SysRoot,
}

impl Observer {
    pub fn read(&self) -> Observation {
        let snap = self.sampler.snapshot();
        Observation {
            rx_bps: snap.net.rx_bps,
            cpu_pct: Some(snap.cpu.load_pct).filter(|v| !v.is_nan()),
            game_active: self.game_active.load(Ordering::SeqCst),
            processes: process_names(&self.root),
        }
    }
}

/// Every running process's name (`/proc/<pid>/comm`, which the kernel cuts
/// at 15 bytes).
fn process_names(root: &SysRoot) -> Vec<String> {
    let Ok(dir) = std::fs::read_dir(root.join("proc")) else {
        return Vec::new();
    };
    dir.filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .bytes()
                .all(|b| b.is_ascii_digit())
        })
        .filter_map(|e| std::fs::read_to_string(e.path().join("comm")).ok())
        .map(|s| s.trim().to_string())
        .collect()
}

fn watch_to_wire(w: &Watch) -> PowerWatch {
    let (kind, threshold, minutes, name) = match &w.condition {
        Condition::NetIdle { below_bps, minutes } => {
            ("net-idle", *below_bps as f64, *minutes, String::new())
        }
        Condition::CpuIdle { below_pct, minutes } => {
            ("cpu-idle", *below_pct, *minutes, String::new())
        }
        Condition::ProcessGone { name } => ("process-gone", 0.0, 0, name.clone()),
        Condition::GameOver => ("game-over", 0.0, 0, String::new()),
    };
    PowerWatch {
        id: w.id,
        action: w.action.name().into(),
        kind: kind.into(),
        threshold,
        minutes,
        name,
        note: w.note.clone(),
    }
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
            out.push(PowerCapability {
                action: a.name().to_string(),
                available,
                reason,
            });
        }
        out
    }

    async fn now(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        action: &str,
    ) -> zbus::fdo::Result<()> {
        let a = Action::parse(action).map_err(invalid)?;
        self.gate.authorize(&hdr, action::POWER).await?;
        let (ok, why) = self.backend.capability(a).await;
        if !ok {
            return Err(zbus::fdo::Error::NotSupported(why));
        }
        tracing::info!(action = a.name(), "power action requested now");
        self.backend
            .perform(a)
            .await
            .map_err(zbus::fdo::Error::Failed)
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
        let job = lock(&self.schedule)
            .add(a, at_unix_ms, r, note, now_ms())
            .map_err(invalid)?;
        tracing::info!(
            id = job.id,
            action = a.name(),
            at_ms = job.at_ms,
            repeat = r.name(),
            "power action scheduled"
        );
        self.changed.notify_one();
        Ok(to_wire(&job))
    }

    async fn jobs(&self) -> Vec<PowerJob> {
        lock(&self.schedule).jobs().iter().map(to_wire).collect()
    }

    async fn cancel(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        id: u32,
    ) -> zbus::fdo::Result<bool> {
        self.gate.authorize(&hdr, action::POWER).await?;
        let removed = lock(&self.schedule)
            .cancel(id)
            .map_err(zbus::fdo::Error::Failed)?;
        if removed {
            tracing::info!(id, "scheduled power action cancelled");
            self.changed.notify_one();
        }
        Ok(removed)
    }

    #[allow(clippy::too_many_arguments)]
    async fn when(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        action: &str,
        kind: &str,
        threshold: f64,
        minutes: u32,
        name: &str,
        note: &str,
    ) -> zbus::fdo::Result<PowerWatch> {
        let a = Action::parse(action).map_err(invalid)?;
        let cond = match kind {
            "net-idle" => Condition::NetIdle {
                below_bps: threshold.max(0.0) as u64,
                minutes,
            },
            "cpu-idle" => Condition::CpuIdle {
                below_pct: threshold,
                minutes,
            },
            // comm is cut at 15 bytes, so compare against what /proc shows.
            "process-gone" => Condition::ProcessGone {
                name: name.trim().chars().take(15).collect(),
            },
            "game-over" => Condition::GameOver,
            _ => {
                return Err(invalid(format!(
                    "unknown condition {kind:?}; use net-idle, cpu-idle, process-gone or game-over"
                )))
            }
        };
        cond.validate().map_err(invalid)?;
        self.gate.authorize(&hdr, action::POWER).await?;
        let (ok, why) = self.backend.capability(a).await;
        if !ok {
            return Err(zbus::fdo::Error::NotSupported(why));
        }
        let observer = Arc::clone(&self.observer);
        let now = tokio::task::spawn_blocking(move || observer.read())
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        let w = self
            .watches
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .add(a, cond, note, &now)
            .map_err(invalid)?;
        tracing::info!(
            id = w.id,
            action = a.name(),
            kind,
            "conditional power action armed"
        );
        self.watches_changed.notify_one();
        Ok(watch_to_wire(&w))
    }

    async fn watches(&self) -> Vec<PowerWatch> {
        self.watches
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .list()
            .iter()
            .map(watch_to_wire)
            .collect()
    }

    async fn cancel_watch(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        id: u32,
    ) -> zbus::fdo::Result<bool> {
        self.gate.authorize(&hdr, action::POWER).await?;
        self.watches
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cancel(id)
            .map_err(zbus::fdo::Error::Failed)
    }

    async fn usage(&self) -> PowerUsage {
        let observer = Arc::clone(&self.observer);
        let energy = Arc::clone(&self.energy);
        tokio::task::spawn_blocking(move || {
            let snap = observer.sampler.snapshot();
            let gpus: Vec<f64> = snap
                .gpus
                .iter()
                .filter(|g| !g.asleep && !g.power_w.is_nan())
                .map(|g| g.power_w)
                .collect();
            let meter = energy.lock().unwrap_or_else(|e| e.into_inner());
            PowerUsage {
                cpu_w: snap.cpu.power_w,
                gpu_w: if gpus.is_empty() {
                    f64::NAN
                } else {
                    gpus.iter().sum()
                },
                battery_w: crate::energy::battery_discharge_w(&observer.root).unwrap_or(f64::NAN),
                cpu_energy_recorded: meter.available(),
                days: meter
                    .days()
                    .into_iter()
                    .map(|(date, cpu_wh)| DayEnergy { date, cpu_wh })
                    .collect(),
            }
        })
        .await
        .unwrap_or(PowerUsage {
            cpu_w: f64::NAN,
            gpu_w: f64::NAN,
            battery_w: f64::NAN,
            cpu_energy_recorded: false,
            days: Vec::new(),
        })
    }

    async fn battery(&self) -> BatteryCare {
        battery_to_wire(&self.battery.lock().unwrap_or_else(|e| e.into_inner()))
    }

    async fn set_battery_care(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        mode: &str,
        limit: u8,
    ) -> zbus::fdo::Result<()> {
        use crate::battery::Care;
        let care = match mode {
            "off" => Care::Off,
            "protect" => Care::Protect { limit },
            "full-once" => Care::FullOnce { limit },
            _ => {
                return Err(invalid(format!(
                    "unknown mode {mode:?}; use off, protect or full-once"
                )))
            }
        };
        self.gate.authorize(&hdr, action::POWER).await?;
        let battery = Arc::clone(&self.battery);
        tokio::task::spawn_blocking(move || {
            battery.lock().unwrap_or_else(|e| e.into_inner()).set(care)
        })
        .await
        .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
        .map_err(invalid)?;
        tracing::info!(mode, limit, "battery protection set");
        Ok(())
    }

    /// Wakes the computer at `at_unix_ms` (from sleep, or from off where the
    /// firmware allows it). Replaces any alarm already set.
    async fn set_wake_alarm(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        at_unix_ms: u64,
    ) -> zbus::fdo::Result<()> {
        if at_unix_ms <= now_ms() + 30_000 {
            return Err(invalid("pick a time at least a minute from now".into()));
        }
        self.gate.authorize(&hdr, action::POWER).await?;
        match &self.rtc {
            Some(r) if r.available() => {
                r.set(at_unix_ms / 1000).map_err(zbus::fdo::Error::Failed)?
            }
            Some(_) => {
                return Err(zbus::fdo::Error::NotSupported(
                    "This computer has no wake-capable hardware clock.".into(),
                ))
            }
            None => {
                *self
                    .simulated_alarm
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(at_unix_ms / 1000);
                if let Backend::Simulated(log) = &*self.backend {
                    log.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(format!("wake-alarm {}", at_unix_ms / 1000));
                }
            }
        }
        tracing::info!(at_unix_ms, "wake alarm set");
        Ok(())
    }

    async fn clear_wake_alarm(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        self.gate.authorize(&hdr, action::POWER).await?;
        match &self.rtc {
            Some(r) if r.available() => r.clear().map_err(zbus::fdo::Error::Failed)?,
            Some(_) => {}
            None => {
                *self
                    .simulated_alarm
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = None
            }
        }
        Ok(())
    }

    /// The armed wake alarm in Unix milliseconds, 0 when none.
    #[zbus(property)]
    async fn wake_alarm(&self) -> u64 {
        let s = match &self.rtc {
            Some(r) => r.get(),
            None => *self
                .simulated_alarm
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        };
        s.map(|s| s * 1000).unwrap_or(0)
    }

    #[zbus(property)]
    async fn wake_alarm_supported(&self) -> bool {
        self.rtc.as_ref().is_none_or(|r| r.available())
    }

    #[zbus(signal)]
    pub async fn upcoming(
        emitter: &SignalEmitter<'_>,
        job: PowerJob,
        seconds: u32,
    ) -> zbus::Result<()>;

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
pub async fn run(
    conn: zbus::Connection,
    schedule: SharedSchedule,
    backend: Arc<Backend>,
    changed: Arc<Notify>,
) {
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
                if let Ok(iface) = conn
                    .object_server()
                    .interface::<_, PowerIface>(ug_proto::DAEMON_OBJECT_PATH)
                    .await
                {
                    let _ = PowerIface::upcoming(iface.signal_emitter(), to_wire(&job), secs).await;
                }
                tracing::info!(
                    id = job.id,
                    action = job.action.name(),
                    secs,
                    "power action coming up"
                );
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
            tracing::warn!(
                id = j.id,
                action = j.action.name(),
                "skipped: it fell due while the machine was asleep"
            );
        }
        // Several jobs due together: the first one decides the machine's fate
        // (a shutdown ends everything after it), so run them in order and stop
        // at the first that takes the machine away.
        for j in due {
            tracing::info!(
                id = j.id,
                action = j.action.name(),
                "running scheduled power action"
            );
            if let Err(e) = backend.perform(j.action).await {
                tracing::error!(id = j.id, error = %e, "scheduled power action failed");
                continue;
            }
            if matches!(
                j.action,
                Action::PowerOff | Action::Reboot | Action::RebootFirmware
            ) {
                break;
            }
        }
    }
}

/// How often conditions are checked while any exist.
const WATCH_EVERY: std::time::Duration = std::time::Duration::from_secs(10);

/// Checks conditional actions every ten seconds while any exist; parks
/// otherwise, so a machine with none does no work for them.
pub async fn watch(
    watches: SharedWatches,
    observer: Arc<Observer>,
    backend: Arc<Backend>,
    changed: Arc<Notify>,
) {
    loop {
        if watches.lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
            changed.notified().await;
            continue;
        }
        let obs_reader = Arc::clone(&observer);
        let Ok(o) = tokio::task::spawn_blocking(move || obs_reader.read()).await else {
            continue;
        };
        let fired = watches
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .observe(&o, now_ms());
        for w in fired {
            tracing::info!(
                id = w.id,
                action = w.action.name(),
                "condition met; running power action"
            );
            if let Err(e) = backend.perform(w.action).await {
                tracing::error!(id = w.id, error = %e, "conditional power action failed");
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(WATCH_EVERY) => {}
            _ = changed.notified() => {}
        }
    }
}

/// Books CPU energy every ten minutes (see `energy`); does nothing on a
/// machine without a readable RAPL counter.
pub async fn meter(energy: Arc<Mutex<crate::energy::EnergyMeter>>) {
    let every = std::time::Duration::from_secs(600);
    loop {
        let e = Arc::clone(&energy);
        let available = tokio::task::spawn_blocking(move || {
            let mut m = e.lock().unwrap_or_else(|e| e.into_inner());
            m.tick(&crate::energy::local_date());
            m.available()
        })
        .await
        .unwrap_or(false);
        if !available {
            tracing::info!("no readable RAPL counter; daily CPU energy is not recorded");
            return;
        }
        tokio::time::sleep(every).await;
    }
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Login1Sleep {
    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

/// Keeps battery protection applied: once at start, after every resume
/// (firmware may forget it), and every five minutes while a one-time full
/// charge is running, to switch protection back on when it is done.
pub async fn keep_battery(battery: SharedBattery, system: Option<zbus::Connection>) {
    use futures_lite::StreamExt;
    let reapply = |b: &SharedBattery| {
        let b = Arc::clone(b);
        async move {
            let r = tokio::task::spawn_blocking(move || {
                b.lock().unwrap_or_else(|e| e.into_inner()).apply()
            })
            .await;
            if let Ok(Err(e)) = r {
                tracing::warn!(error = %e, "could not apply battery protection");
            }
        }
    };
    reapply(&battery).await;
    let mut resumes = match &system {
        Some(c) => match Login1SleepProxy::new(c).await {
            Ok(p) => p.receive_prepare_for_sleep().await.ok(),
            Err(_) => None,
        },
        None => None,
    };
    loop {
        let full_once = matches!(
            battery.lock().unwrap_or_else(|e| e.into_inner()).care,
            crate::battery::Care::FullOnce { .. }
        );
        let tick = async {
            if full_once {
                tokio::time::sleep(std::time::Duration::from_secs(300)).await
            } else {
                std::future::pending::<()>().await
            }
        };
        let resume = async {
            match resumes.as_mut() {
                Some(s) => loop {
                    match s.next().await {
                        Some(sig) if sig.args().is_ok_and(|a| !a.start) => break,
                        Some(_) => continue,
                        None => std::future::pending::<()>().await,
                    }
                },
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            _ = tick => {}
            _ = resume => tracing::info!("resumed; applying battery protection again"),
        }
        reapply(&battery).await;
    }
}
