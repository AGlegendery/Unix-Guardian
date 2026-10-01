//! The D-Bus surface the daemon serves.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;
use ug_proto::{GpuCapabilities, GpuInfo, Inventory, Snapshot, PROTOCOL_VERSION};
use zbus::object_server::SignalEmitter;

use crate::polkit::{action, Gate};
use crate::sampler::Sampler;

/// Tracks which bus clients want the sampling signal, and wakes the loop when
/// the first one arrives.
///
/// The point is that a machine with no UI open does no periodic work at all —
/// not a cheap tick, none. The loop parks on [`Notify`] rather than polling.
///
/// Subscriptions are keyed by the caller's unique bus name rather than counted,
/// because a client that dies without calling `Unsubscribe` — killed by
/// SIGTERM, crashed, or a UI whose window was closed mid-call — would otherwise
/// pin the loop on for the life of the daemon. [`Subscribers::client_gone`] is
/// driven by `NameOwnerChanged`, so a vanished client is released the moment
/// the bus notices, with no cooperation from the client required.
#[derive(Default)]
pub struct Subscribers {
    /// Unique bus name → requested interval in milliseconds. A client that
    /// subscribes twice holds one entry; the latest interval wins.
    clients: Mutex<HashMap<String, u32>>,
    wake: Notify,
}

impl Subscribers {
    pub fn add(&self, client: &str, interval_ms: u32) {
        // 250 ms is below what any sensor updates at; an hour is long enough
        // that a client asking for more is confused rather than frugal.
        let clamped = interval_ms.clamp(250, 3_600_000);
        let mut clients = self.lock();
        let was_idle = clients.is_empty();
        clients.insert(client.to_string(), clamped);
        drop(clients);
        if was_idle {
            tracing::debug!(client, interval_ms = clamped, "first subscriber; sampling loop starting");
            self.wake.notify_one();
        }
    }

    /// Releases a client's subscription. Removing a client that never
    /// subscribed is a no-op, so a stray `Unsubscribe` is harmless.
    pub fn remove(&self, client: &str) {
        let mut clients = self.lock();
        if clients.remove(client).is_some() && clients.is_empty() {
            tracing::debug!(client, "last subscriber left; sampling loop parking");
        }
    }

    /// Called when a unique name drops off the bus.
    pub fn client_gone(&self, client: &str) {
        if self.lock().contains_key(client) {
            tracing::debug!(client, "subscriber disconnected without unsubscribing");
            self.remove(client);
        }
    }

    pub fn active(&self) -> bool {
        !self.lock().is_empty()
    }

    /// The fastest interval any current subscriber asked for, so the most
    /// demanding client is served and slower ones simply see extra samples.
    pub fn interval(&self) -> std::time::Duration {
        let ms = self.lock().values().copied().min().unwrap_or(1000);
        std::time::Duration::from_millis(ms.max(250) as u64)
    }

    /// Parks until at least one client is subscribed.
    pub async fn wait_for_interest(&self) {
        loop {
            // Register interest before checking, so a subscription that lands
            // between the check and the await cannot be missed.
            let notified = self.wake.notified();
            if self.active() {
                return;
            }
            notified.await;
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, u32>> {
        self.clients.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// `org.unixguardian.Daemon1.System`.
pub struct SystemIface {
    pub sampler: Arc<Sampler>,
    pub subscribers: Arc<Subscribers>,
    pub reload: Arc<dyn Fn(&str) + Send + Sync>,
    pub gate: Gate,
}

#[zbus::interface(name = "org.unixguardian.Daemon1.System")]
impl SystemIface {
    /// One consistent read of every sensor.
    async fn snapshot(&self) -> Snapshot {
        self.sampler.snapshot()
    }

    /// Disk, battery, per-core clocks and load average. Rates are since the
    /// previous call by any client, so poll it at the same pace as Snapshot.
    async fn extra(&self) -> ug_proto::ExtraSample {
        self.sampler.extra()
    }

    /// Static machine description.
    async fn inventory(&self) -> Inventory {
        self.sampler.inventory()
    }

    /// Changes the log filter without a restart, e.g. `"ug_gpu=trace,info"`.
    ///
    /// Admin-only: trace logging from a root daemon can fill the journal and
    /// record more about the machine than an unprivileged user should be able
    /// to switch on.
    async fn set_log_filter(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        filter: &str,
    ) -> zbus::fdo::Result<()> {
        self.gate.authorize(&hdr, action::DAEMON_DEBUG).await?;
        tracing::info!(filter, "log filter changed over D-Bus");
        (self.reload)(filter);
        Ok(())
    }

    /// Joins the sampling loop. Released automatically if the caller leaves
    /// the bus without calling `Unsubscribe`.
    async fn subscribe(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        interval_ms: u32,
    ) -> zbus::fdo::Result<()> {
        let client = sender(&hdr)?;
        self.subscribers.add(&client, interval_ms);
        Ok(())
    }

    /// Leaves the sampling loop.
    async fn unsubscribe(&self, #[zbus(header)] hdr: zbus::message::Header<'_>) -> zbus::fdo::Result<()> {
        let client = sender(&hdr)?;
        self.subscribers.remove(&client);
        Ok(())
    }

    #[zbus(property)]
    async fn protocol_version(&self) -> u32 {
        PROTOCOL_VERSION
    }

    #[zbus(property)]
    async fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }

    /// Emitted once per interval while anyone is subscribed.
    #[zbus(signal)]
    pub async fn sample(emitter: &SignalEmitter<'_>, snapshot: Snapshot) -> zbus::Result<()>;
}

/// The caller's unique bus name, e.g. `:1.42`.
fn sender(hdr: &zbus::message::Header<'_>) -> zbus::fdo::Result<String> {
    hdr.sender()
        .map(|s| s.to_string())
        .ok_or_else(|| zbus::fdo::Error::Failed("message has no sender".into()))
}

/// `org.unixguardian.Daemon1.Gpu`.
pub struct GpuIface {
    pub sampler: Arc<Sampler>,
    /// The auto-tune in progress or last finished, and its cancel switch.
    pub auto: Arc<Mutex<ug_proto::AutoTuneStatus>>,
    pub auto_cancel: Arc<std::sync::atomic::AtomicBool>,
    pub simulated: bool,
    pub tuner: crate::gputune::SharedTuner,
    pub gate: Gate,
    /// PCI address of the GPU tuning acts on. Chosen at start-up as the first
    /// discrete card, which is the right default on a hybrid laptop where the
    /// firmware-posted GPU is the integrated one.
    pub active: std::sync::Mutex<String>,
}

#[zbus::interface(name = "org.unixguardian.Daemon1.Gpu")]
impl GpuIface {
    /// Every GPU, discrete and integrated, in PCI order.
    async fn list(&self) -> Vec<GpuInfo> {
        self.sampler.gpu_infos()
    }

    /// What this GPU permits here, with reasons for what it does not.
    async fn capabilities(&self, pci: &str) -> zbus::fdo::Result<GpuCapabilities> {
        self.sampler
            .gpu_capabilities(pci)
            .ok_or_else(|| zbus::fdo::Error::FileNotFound(format!("no GPU at {pci}")))
    }

    /// Limits and current values of the power limit and clock offsets, and
    /// what Unix Guardian keeps applied. May wake a sleeping discrete GPU.
    async fn tune_state(&self, pci: &str) -> zbus::fdo::Result<ug_proto::GpuTuneState> {
        let t = Arc::clone(&self.tuner);
        let pci = pci.to_string();
        tokio::task::spawn_blocking(move || t.lock().unwrap_or_else(|e| e.into_inner()).state(&pci))
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
            .ok_or_else(|| zbus::fdo::Error::FileNotFound("no tunable GPU at that address".into()))
    }

    /// Applies a setting on trial: undone after 30 seconds unless
    /// `KeepTune` is called. Needs `org.unixguardian.gpu.tune`.
    async fn try_tune(&self, #[zbus(header)] hdr: zbus::message::Header<'_>, pci: &str, tune: ug_proto::GpuTune) -> zbus::fdo::Result<()> {
        self.gate.authorize(&hdr, action::GPU_TUNE).await?;
        let t = Arc::clone(&self.tuner);
        let pci = pci.to_string();
        tokio::task::spawn_blocking(move || t.lock().unwrap_or_else(|e| e.into_inner()).try_tune(&pci, tune, std::time::Instant::now()))
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
            .map_err(zbus::fdo::Error::Failed)
    }

    /// Keeps the setting on trial (and applies it again after every wake).
    async fn keep_tune(&self, #[zbus(header)] hdr: zbus::message::Header<'_>, pci: &str) -> zbus::fdo::Result<()> {
        self.gate.authorize(&hdr, action::GPU_TUNE).await?;
        let t = Arc::clone(&self.tuner);
        let pci = pci.to_string();
        tokio::task::spawn_blocking(move || t.lock().unwrap_or_else(|e| e.into_inner()).confirm(&pci))
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
            .map_err(zbus::fdo::Error::Failed)
    }

    /// Runs the stability test (`seconds`, 10–1800) on the setting being
    /// tried, keeping the trial alive meanwhile; an unstable setting is
    /// undone at once. Returns (stable, what happened).
    async fn test_tune(&self, #[zbus(header)] hdr: zbus::message::Header<'_>, pci: &str, seconds: u32) -> zbus::fdo::Result<(bool, String)> {
        self.gate.authorize(&hdr, action::GPU_TUNE).await?;
        crate::gputune::test_trial(Arc::clone(&self.tuner), pci.to_string(), seconds).await.map_err(zbus::fdo::Error::Failed)
    }

    /// Searches for a setting by measuring (`faster`: highest stable core
    /// offset; `cooler`: lowest power limit at the same clock). Runs in the
    /// background (minutes); watch AutoTuneStatus. The result is left on
    /// trial for two minutes for the user to keep.
    async fn auto_tune(&self, #[zbus(header)] hdr: zbus::message::Header<'_>, pci: &str, goal: &str) -> zbus::fdo::Result<()> {
        self.gate.authorize(&hdr, action::GPU_TUNE).await?;
        if !matches!(goal, "faster" | "cooler") {
            return Err(zbus::fdo::Error::InvalidArgs("goal is faster or cooler".into()));
        }
        {
            let mut s = self.auto.lock().unwrap_or_else(|e| e.into_inner());
            if s.running {
                return Err(zbus::fdo::Error::Failed("an auto-tune is already running".into()));
            }
            *s = ug_proto::AutoTuneStatus { running: true, message: "Starting…".into(), ..Default::default() };
        }
        self.auto_cancel.store(false, std::sync::atomic::Ordering::SeqCst);
        let mut env = crate::autotune::Real {
            tuner: Arc::clone(&self.tuner),
            sampler: Arc::clone(&self.sampler),
            pci: pci.to_string(),
            status: Arc::clone(&self.auto),
            cancel: Arc::clone(&self.auto_cancel),
            simulated: self.simulated,
        };
        let (status, tuner, pci, goal) = (Arc::clone(&self.auto), Arc::clone(&self.tuner), pci.to_string(), goal.to_string());
        tokio::task::spawn_blocking(move || {
            let r = crate::autotune::run(&mut env, &goal);
            let mut s = status.lock().unwrap_or_else(|e| e.into_inner());
            s.running = false;
            match r {
                Ok(o) => {
                    tuner.lock().unwrap_or_else(|e| e.into_inner()).extend(&pci, std::time::Duration::from_secs(120));
                    s.ok = true;
                    s.summary = o.summary;
                    s.message = "Done. Keep the result within two minutes, or it is undone.".into();
                }
                Err(e) => {
                    let _ = tuner.lock().unwrap_or_else(|e| e.into_inner()).cancel(&pci);
                    s.ok = false;
                    s.summary = e;
                    s.message = "Stopped; the card is back as it was.".into();
                }
            }
        });
        Ok(())
    }

    async fn auto_tune_status(&self) -> ug_proto::AutoTuneStatus {
        self.auto.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Stops a running auto-tune; the card goes back as it was.
    async fn stop_auto_tune(&self) {
        self.auto_cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Undoes the setting on trial now.
    async fn cancel_tune(&self, pci: &str) -> zbus::fdo::Result<()> {
        let t = Arc::clone(&self.tuner);
        let pci = pci.to_string();
        tokio::task::spawn_blocking(move || t.lock().unwrap_or_else(|e| e.into_inner()).cancel(&pci))
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
            .map_err(zbus::fdo::Error::Failed)
    }

    /// The card's own defaults, now and from now on.
    async fn reset_tune(&self, #[zbus(header)] hdr: zbus::message::Header<'_>, pci: &str) -> zbus::fdo::Result<()> {
        self.gate.authorize(&hdr, action::GPU_TUNE).await?;
        let t = Arc::clone(&self.tuner);
        let pci = pci.to_string();
        tokio::task::spawn_blocking(move || t.lock().unwrap_or_else(|e| e.into_inner()).reset(&pci))
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
            .map_err(zbus::fdo::Error::Failed)
    }

    #[zbus(property)]
    async fn active_gpu(&self) -> String {
        self.active.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    #[zbus(property)]
    async fn set_active_gpu(&self, pci: &str) -> zbus::fdo::Result<()> {
        if !self.sampler.gpu_infos().iter().any(|g| g.pci == pci) {
            return Err(zbus::fdo::Error::InvalidArgs(format!(
                "no GPU at {pci}; call List() for the available addresses"
            )));
        }
        *self.active.lock().unwrap_or_else(|e| e.into_inner()) = pci.to_string();
        tracing::info!(pci, "active GPU changed");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_idle_daemon_has_no_subscribers() {
        assert!(!Subscribers::default().active());
    }

    #[test]
    fn the_loop_runs_until_the_last_client_leaves() {
        let s = Subscribers::default();
        s.add(":1.10", 1000);
        s.add(":1.11", 1000);
        s.remove(":1.10");
        assert!(s.active(), "one client is still watching");
        s.remove(":1.11");
        assert!(!s.active());
    }

    #[test]
    fn a_client_that_dies_without_unsubscribing_is_released() {
        let s = Subscribers::default();
        s.add(":1.42", 1000);
        s.client_gone(":1.42");
        assert!(!s.active(), "a killed client must not pin the loop on forever");
    }

    #[test]
    fn a_disconnect_of_someone_else_does_not_release_our_client() {
        let s = Subscribers::default();
        s.add(":1.42", 1000);
        s.client_gone(":1.99");
        assert!(s.active());
    }

    #[test]
    fn subscribing_twice_is_one_subscription() {
        let s = Subscribers::default();
        s.add(":1.42", 1000);
        s.add(":1.42", 1000);
        s.remove(":1.42");
        assert!(!s.active(), "a repeat Subscribe must not need two Unsubscribes");
    }

    #[test]
    fn a_stray_unsubscribe_is_harmless() {
        let s = Subscribers::default();
        s.remove(":1.7");
        s.add(":1.8", 1000);
        s.remove(":1.7");
        assert!(s.active());
    }

    #[test]
    fn the_fastest_requested_interval_wins() {
        let s = Subscribers::default();
        s.add(":1.1", 2000);
        s.add(":1.2", 500);
        assert_eq!(s.interval(), std::time::Duration::from_millis(500));
        s.remove(":1.2");
        assert_eq!(s.interval(), std::time::Duration::from_millis(2000));
    }

    #[test]
    fn absurd_intervals_are_clamped() {
        let s = Subscribers::default();
        s.add(":1.1", 0);
        assert_eq!(s.interval(), std::time::Duration::from_millis(250));
        s.add(":1.1", u32::MAX);
        assert_eq!(s.interval(), std::time::Duration::from_millis(3_600_000));
    }
}
