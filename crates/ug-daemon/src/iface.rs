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
