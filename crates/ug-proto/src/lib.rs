//! The Unix Guardian D-Bus contract.
//!
//! Everything that crosses a process boundary is defined here and nowhere else:
//! the wire types, the interface traits, and the well-known names. The daemon
//! implements these interfaces, the agent and the UI consume them, and `ug-cli`
//! is a thin shell over the same calls.
//!
//! # Versioning
//!
//! Interfaces only ever gain methods. A breaking change means a new interface
//! (`Daemon2`), never a changed signature on `Daemon1`. Clients check
//! [`DaemonProxy::protocol_version`] before relying on anything newer than the
//! release they were built against.
//!
//! # Missing readings
//!
//! D-Bus has no option type, and a snapshot is mostly numbers that a given
//! machine may not expose — an Intel laptop has no `gpu_busy_percent`, a VM has
//! no RAPL. Rather than a parallel set of "has_x" booleans, unavailable readings
//! travel as sentinels: `f64::NAN` for floats, `0` for counts, `""` for strings.
//! Use the accessors ([`CpuSample::temp`] and friends), which hand back `Option`,
//! instead of testing the raw fields.

pub mod types;

pub use types::*;

/// Well-known bus name of the privileged daemon (system bus).
pub const DAEMON_BUS_NAME: &str = "org.unixguardian.Daemon1";
/// Object path the daemon's interfaces hang off.
pub const DAEMON_OBJECT_PATH: &str = "/org/unixguardian/Daemon1";
/// Well-known bus name of the per-session agent (session bus).
pub const AGENT_BUS_NAME: &str = "org.unixguardian.Agent1";
/// Object path the agent's interfaces hang off.
pub const AGENT_OBJECT_PATH: &str = "/org/unixguardian/Agent1";

/// Protocol version this build speaks. Bumped when methods are added.
pub const PROTOCOL_VERSION: u32 = 1;

/// Errors that cross the bus.
///
/// Each variant maps to a distinct D-Bus error name so callers can branch on the
/// cause rather than string-matching a message. [`Unsupported`](UgError::Unsupported)
/// is the common one and is not a failure: it is how the daemon tells the UI that
/// this machine cannot do the thing, so the control should be shown disabled with
/// the reason rather than hidden or shown broken.
#[derive(Debug, thiserror::Error)]
pub enum UgError {
    /// The hardware, kernel, or distribution does not support this operation.
    /// Carries a human-readable reason meant to be shown to the user verbatim.
    #[error("{0}")]
    Unsupported(String),
    /// polkit denied the caller.
    #[error("not authorized: {0}")]
    NotAuthorized(String),
    /// The referenced GPU, profile, or job does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// The request was well-formed but the value is out of the allowed range.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    /// Reading or writing a kernel interface failed.
    #[error("system error: {0}")]
    System(String),
}

impl From<UgError> for zbus::fdo::Error {
    fn from(e: UgError) -> Self {
        match e {
            UgError::NotAuthorized(m) => zbus::fdo::Error::AccessDenied(m),
            UgError::NotFound(m) => zbus::fdo::Error::FileNotFound(m),
            UgError::InvalidArgument(m) => zbus::fdo::Error::InvalidArgs(m),
            UgError::Unsupported(m) => zbus::fdo::Error::NotSupported(m),
            UgError::System(m) => zbus::fdo::Error::Failed(m),
        }
    }
}

/// Convenience alias for fallible daemon operations.
pub type Result<T> = std::result::Result<T, UgError>;

/// Client proxies for the daemon's interfaces.
pub mod daemon {
    use crate::types::*;

    /// Top-level daemon interface: identity, health, and the telemetry snapshot.
    #[zbus::proxy(
        interface = "org.unixguardian.Daemon1.System",
        default_service = "org.unixguardian.Daemon1",
        default_path = "/org/unixguardian/Daemon1"
    )]
    pub trait Daemon {
        /// One consistent read of every sensor the machine exposes.
        ///
        /// This is the only call the monitoring UI makes, once a second. It never
        /// fails on missing hardware — absent readings come back as sentinels.
        fn snapshot(&self) -> zbus::Result<Snapshot>;

        /// Static description of the machine: CPU model, topology, GPUs, distro.
        /// Cheap to call but meant to be fetched once and cached.
        fn inventory(&self) -> zbus::Result<Inventory>;

        /// Change the daemon's log filter at runtime, e.g. `"debug"` or
        /// `"ug_gpu=trace,info"`. Survives until the next restart.
        fn set_log_filter(&self, filter: &str) -> zbus::Result<()>;

        /// Protocol version this daemon speaks. See module docs on versioning.
        #[zbus(property)]
        fn protocol_version(&self) -> zbus::Result<u32>;

        /// Package version of the running daemon, e.g. `"0.1.0"`.
        #[zbus(property)]
        fn version(&self) -> zbus::Result<String>;

        /// Emitted once per sampling interval, but only while at least one client
        /// has called [`subscribe`](DaemonProxy::subscribe). With no subscribers
        /// the daemon does no periodic work at all.
        #[zbus(signal)]
        fn sample(&self, snapshot: Snapshot) -> zbus::Result<()>;

        /// Start (or join) the sampling loop. Reference-counted: the loop stops
        /// when the last subscriber unsubscribes or drops off the bus.
        fn subscribe(&self, interval_ms: u32) -> zbus::Result<()>;

        /// Release this client's claim on the sampling loop.
        fn unsubscribe(&self) -> zbus::Result<()>;
    }

    /// The privileged half of game mode.
    ///
    /// Requires the `org.unixguardian.gamemode` polkit action, which active
    /// local sessions have without a password: it runs automatically when a
    /// game starts, and everything it does is reverted when the game ends —
    /// or when the calling agent disappears, or on the daemon's next start
    /// after a crash.
    #[zbus::proxy(
        interface = "org.unixguardian.Daemon1.GameMode",
        default_service = "org.unixguardian.Daemon1",
        default_path = "/org/unixguardian/Daemon1"
    )]
    pub trait GameMode {
        /// Turns game mode on, or adds processes to it if it is already on.
        fn enter(&self, request: &GameModeRequest) -> zbus::Result<GameModeReport>;

        /// Turns game mode off and restores everything it changed.
        fn leave(&self) -> zbus::Result<GameModeReport>;

        #[zbus(property)]
        fn active(&self) -> zbus::Result<bool>;
    }

    /// Shutting down, restarting and sleeping, now or on a schedule.
    ///
    /// Carried out through logind. `Now`, `Schedule` and `Cancel` need the
    /// `org.unixguardian.power.control` polkit action, which active local
    /// sessions have without a password (as with logind itself). A development
    /// daemon never acts: it records what it would have done in `Simulated`.
    #[zbus::proxy(
        interface = "org.unixguardian.Daemon1.Power",
        default_service = "org.unixguardian.Daemon1",
        default_path = "/org/unixguardian/Daemon1"
    )]
    pub trait Power {
        /// Every action with whether it is possible here and, if not, why.
        fn capabilities(&self) -> zbus::Result<Vec<PowerCapability>>;

        /// Does it now.
        fn now(&self, action: &str) -> zbus::Result<()>;

        /// Queues it for `at_unix_ms`, repeating `once`, `daily` or `weekly`.
        /// Survives daemon restarts; a one-off time missed while the daemon
        /// was down is dropped, never run late.
        fn schedule(&self, action: &str, at_unix_ms: u64, repeat: &str, note: &str) -> zbus::Result<PowerJob>;

        /// The queue, soonest first.
        fn jobs(&self) -> zbus::Result<Vec<PowerJob>>;

        /// Removes a job; false if there was none with that id.
        fn cancel(&self, id: u32) -> zbus::Result<bool>;

        /// Sent 60 seconds before a job runs (or at once if scheduled closer),
        /// so a desktop can offer to cancel it.
        #[zbus(signal)]
        fn upcoming(&self, job: PowerJob, seconds: u32) -> zbus::Result<()>;

        /// Does `action` once a condition has held for its whole window:
        /// `net-idle` (receive rate below `threshold` B/s for `minutes`),
        /// `cpu-idle` (load below `threshold` % for `minutes`),
        /// `process-gone` (the process `name` has exited), `game-over` (the
        /// running game has ended).
        fn when(&self, action: &str, kind: &str, threshold: f64, minutes: u32, name: &str, note: &str) -> zbus::Result<PowerWatch>;

        fn watches(&self) -> zbus::Result<Vec<PowerWatch>>;

        fn cancel_watch(&self, id: u32) -> zbus::Result<bool>;

        /// Draw now (CPU, GPU, whole machine on battery) and CPU energy per
        /// day. Only what is measured: the rest of the machine has no sensor.
        fn usage(&self) -> zbus::Result<PowerUsage>;

        /// Battery protection: what is supported and what is set.
        fn battery(&self) -> zbus::Result<BatteryCare>;

        /// `off` (charge to 100 %), `protect` (stop at `limit` %, or the
        /// vendor's fixed limit) or `full-once` (100 % once, then protect).
        fn set_battery_care(&self, mode: &str, limit: u8) -> zbus::Result<()>;

        /// Wakes the computer at this time; replaces any alarm set.
        fn set_wake_alarm(&self, at_unix_ms: u64) -> zbus::Result<()>;

        fn clear_wake_alarm(&self) -> zbus::Result<()>;

        /// The armed alarm in Unix milliseconds; 0 when none.
        #[zbus(property)]
        fn wake_alarm(&self) -> zbus::Result<u64>;

        #[zbus(property)]
        fn wake_alarm_supported(&self) -> zbus::Result<bool>;

        /// Development runs only: what would have been done, in order.
        #[zbus(property)]
        fn simulated(&self) -> zbus::Result<Vec<String>>;
    }

    /// Read and write GPU state: clocks, power limits, fans.
    ///
    /// Write methods each require the `org.unixguardian.gpu.tune` polkit action.
    #[zbus::proxy(
        interface = "org.unixguardian.Daemon1.Gpu",
        default_service = "org.unixguardian.Daemon1",
        default_path = "/org/unixguardian/Daemon1"
    )]
    pub trait Gpu {
        /// Every GPU the machine has, discrete and integrated, in PCI order.
        fn list(&self) -> zbus::Result<Vec<GpuInfo>>;

        /// What this GPU actually allows, given its generation, driver, and
        /// kernel parameters. The UI drives control visibility off this — a
        /// control whose capability is false is shown disabled with
        /// [`GpuCapabilities::notes`] as the explanation.
        fn capabilities(&self, pci: &str) -> zbus::Result<GpuCapabilities>;

        /// The GPU that tuning and monitoring act on by default.
        #[zbus(property)]
        fn active_gpu(&self) -> zbus::Result<String>;
        #[zbus(property)]
        fn set_active_gpu(&self, pci: &str) -> zbus::Result<()>;
    }
}

/// Client proxies for the per-user agent's interfaces (session bus).
pub mod agent {
    use crate::types::*;

    /// Games: what is installed, what is running, and correcting the detector.
    #[zbus::proxy(
        interface = "org.unixguardian.Agent1.Games",
        default_service = "org.unixguardian.Agent1",
        default_path = "/org/unixguardian/Agent1"
    )]
    pub trait Games {
        /// Installed games from every launcher found, as of the last scan.
        fn installed(&self) -> zbus::Result<Vec<InstalledGameInfo>>;

        /// Games running now.
        fn running(&self) -> zbus::Result<Vec<RunningGameInfo>>;

        /// Re-reads every launcher's library. Returns how many games it found.
        fn rescan(&self) -> zbus::Result<u32>;

        /// "This is not a game": stops treating an executable as one, now and
        /// in future. Takes the executable name, e.g. `"Render.exe"`.
        fn ignore_executable(&self, name: &str) -> zbus::Result<()>;

        #[zbus(signal)]
        fn game_started(&self, game: RunningGameInfo) -> zbus::Result<()>;

        /// `played_s` counts until the game's last process exited.
        #[zbus(signal)]
        fn game_stopped(&self, game: RunningGameInfo, played_s: u64) -> zbus::Result<()>;
    }

    /// Playtime history.
    #[zbus::proxy(
        interface = "org.unixguardian.Agent1.Playtime",
        default_service = "org.unixguardian.Agent1",
        default_path = "/org/unixguardian/Agent1"
    )]
    pub trait Playtime {
        /// Time per game since `since_unix_ms` (0 for all time), most played
        /// first. Sessions spanning the boundary count only their later part.
        fn summary(&self, since_unix_ms: u64) -> zbus::Result<Vec<PlaytimeEntry>>;

        /// Deletes every recorded session of one game.
        fn forget(&self, key: &str) -> zbus::Result<u32>;

        /// Every session that ended after `since_unix_ms` (0 for all), oldest
        /// first, for export.
        fn sessions(&self, since_unix_ms: u64) -> zbus::Result<Vec<PlaytimeSession>>;
    }

    /// Monitoring history the agent records once a minute (90 days).
    #[zbus::proxy(
        interface = "org.unixguardian.Agent1.Monitor",
        default_service = "org.unixguardian.Agent1",
        default_path = "/org/unixguardian/Agent1"
    )]
    pub trait Monitor {
        /// Readings between two times, averaged into at most `points` buckets.
        fn history(&self, since_unix_ms: u64, until_unix_ms: u64, points: u32) -> zbus::Result<Vec<HistoryPoint>>;

        /// Bytes received and sent per local day since `since_unix_ms`.
        fn traffic(&self, since_unix_ms: u64) -> zbus::Result<Vec<DayTraffic>>;
    }

    /// Ping to the router, the DNS server and the game's server. Measuring
    /// runs only while someone asks for it (within the last minute) or a game
    /// is running.
    #[zbus::proxy(
        interface = "org.unixguardian.Agent1.Ping",
        default_service = "org.unixguardian.Agent1",
        default_path = "/org/unixguardian/Agent1"
    )]
    pub trait Ping {
        /// Samples since `since_unix_ms` for every target, plus the marks.
        fn recent(&self, since_unix_ms: u64) -> zbus::Result<PingReport>;

        /// Stats for the `window_s` seconds before and after `mark_unix_ms`.
        fn compare(&self, target: &str, mark_unix_ms: u64, window_s: u32) -> zbus::Result<PingComparison>;
    }

    /// Game profiles and which game uses which.
    #[zbus::proxy(
        interface = "org.unixguardian.Agent1.Profiles",
        default_service = "org.unixguardian.Agent1",
        default_path = "/org/unixguardian/Agent1"
    )]
    pub trait Profiles {
        /// Every profile, as loaded from the built-ins and the user's files.
        fn list(&self) -> zbus::Result<Vec<ProfileInfo>>;

        /// Makes a game (by key, e.g. `steam:730`) run with a profile; an empty
        /// `profile` returns it to the one the game database suggests. Takes
        /// effect for a running game on the next detection tick.
        fn set_game_profile(&self, key: &str, profile: &str) -> zbus::Result<()>;

        /// Turns one on/off setting of a profile on or off, e.g.
        /// `("quiet", "cpu_priority", false)`. Written to the user's profile
        /// file; running games pick it up on the next tick.
        fn set_option(&self, profile: &str, option: &str, value: bool) -> zbus::Result<()>;
    }
}
