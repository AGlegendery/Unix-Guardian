//! Structured logging, with a filter that can be changed without a restart.
//!
//! Debugging a hardware problem usually means turning on trace logging for one
//! module on a machine that is already misbehaving. Restarting the daemon to do
//! that would discard the state that is being investigated, so the filter is
//! installed behind a reload handle and driven over D-Bus instead.

use std::sync::{Arc, OnceLock};
use tracing_subscriber::{filter::EnvFilter, prelude::*, reload};

type ReloadHandle = reload::Handle<EnvFilter, tracing_subscriber::Registry>;

static HANDLE: OnceLock<ReloadHandle> = OnceLock::new();

/// Installs the subscriber.
///
/// `default_filter` applies unless `UG_LOG` is set. Calling this twice is a
/// no-op beyond the first, so tests that initialise logging do not fight.
pub fn init(default_filter: &str) {
    let filter =
        EnvFilter::try_from_env("UG_LOG").unwrap_or_else(|_| EnvFilter::new(default_filter));
    let (layer, handle) = reload::Layer::new(filter);

    // systemd stamps its own timestamp on anything it captures from stderr, so
    // printing one under a unit shows it twice.
    let under_systemd = std::env::var_os("JOURNAL_STREAM").is_some();
    let fmt = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(true)
        .with_ansi(!under_systemd);

    let registry = tracing_subscriber::registry().with(layer);
    let installed = if under_systemd {
        registry.with(fmt.without_time()).try_init()
    } else {
        registry.with(fmt).try_init()
    };

    if installed.is_ok() {
        let _ = HANDLE.set(handle);
    }
}

/// Returns a callback that swaps the active filter.
///
/// An unparseable filter is rejected and logged rather than applied, so a typo
/// over D-Bus cannot silence the daemon.
pub fn reload_handle() -> Arc<dyn Fn(&str) + Send + Sync> {
    Arc::new(|spec: &str| match EnvFilter::try_new(spec) {
        Ok(f) => match HANDLE.get() {
            Some(h) => {
                if let Err(e) = h.reload(f) {
                    tracing::error!(error = %e, "could not apply log filter");
                }
            }
            None => tracing::warn!("logging is not initialised; filter unchanged"),
        },
        Err(e) => tracing::warn!(spec, error = %e, "invalid log filter; keeping the current one"),
    })
}
