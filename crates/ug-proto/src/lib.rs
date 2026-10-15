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

/// When a D-Bus error says the other side sends or expects different
/// types (a program updated while the agent or daemon kept running the old
/// version), what to tell the user instead of the raw signature.
pub fn outdated_peer(error: &str) -> Option<&'static str> {
    error.contains("Signature mismatch").then_some(
        "Unix Guardian was updated, but an agent or service from before the update is still running. \
         Restart it: systemctl --user restart ug-agent (and for the service: sudo systemctl restart unix-guardiand), \
         or log out and in again.",
    )
}

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

        /// Disk throughput, battery power, per-core clocks, load average.
        fn extra(&self) -> zbus::Result<ExtraSample>;

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
        fn schedule(
            &self,
            action: &str,
            at_unix_ms: u64,
            repeat: &str,
            note: &str,
        ) -> zbus::Result<PowerJob>;

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
        fn when(
            &self,
            action: &str,
            kind: &str,
            threshold: f64,
            minutes: u32,
            name: &str,
            note: &str,
        ) -> zbus::Result<PowerWatch>;

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

        /// Sleep variant and the lid and power-button actions.
        fn sleep_settings(&self) -> zbus::Result<SleepSettings>;

        /// Sleeps the `s2idle` or `deep` way from now on; `keep` applies it
        /// again at every boot.
        fn set_sleep_mode(&self, mode: &str, keep: bool) -> zbus::Result<()>;

        /// What logind does for `lid-battery`, `lid-ac`, `lid-docked` or
        /// `power-key`: `ignore`, `lock`, `suspend`, `hibernate` or `poweroff`.
        fn set_button_action(&self, which: &str, action: &str) -> zbus::Result<()>;

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

        /// Power limit and clock offsets: limits, current values, what is
        /// kept, and a setting that crashed. May wake a sleeping GPU.
        fn tune_state(&self, pci: &str) -> zbus::Result<GpuTuneState>;

        /// Applies a setting on trial; undone after 30 s unless kept.
        fn try_tune(&self, pci: &str, tune: &GpuTune) -> zbus::Result<()>;

        /// Keeps the setting on trial.
        fn keep_tune(&self, pci: &str) -> zbus::Result<()>;

        /// Runs the stability test on the setting being tried; an unstable
        /// one is undone at once. Returns (stable, what happened).
        fn test_tune(&self, pci: &str, seconds: u32) -> zbus::Result<(bool, String)>;

        /// Searches for a setting by measuring: `faster` or `cooler`.
        fn auto_tune(&self, pci: &str, goal: &str) -> zbus::Result<()>;
        fn auto_tune_status(&self) -> zbus::Result<AutoTuneStatus>;
        fn stop_auto_tune(&self) -> zbus::Result<()>;

        /// Undoes the setting on trial now (no authorisation: undoing is
        /// always allowed).
        fn cancel_tune(&self, pci: &str) -> zbus::Result<()>;

        /// Back to the card's own defaults, now and from now on.
        fn reset_tune(&self, pci: &str) -> zbus::Result<()>;

        /// The GPU that tuning and monitoring act on by default.
        #[zbus(property)]
        fn active_gpu(&self) -> zbus::Result<String>;
        #[zbus(property)]
        fn set_active_gpu(&self, pci: &str) -> zbus::Result<()>;
    }

    /// Fan curves and the laptop firmware's fan modes. Changes need the
    /// `org.unixguardian.cooling` polkit action.
    #[zbus::proxy(
        interface = "org.unixguardian.Daemon1.Cooling",
        default_service = "org.unixguardian.Daemon1",
        default_path = "/org/unixguardian/Daemon1"
    )]
    pub trait Cooling {
        fn state(&self) -> zbus::Result<CoolingState>;
        /// Drives a fan by (°C, %) points.
        fn set_curve(&self, id: &str, points: &[(f64, f64)]) -> zbus::Result<()>;
        /// Hands a fan back to automatic control.
        fn set_auto(&self, id: &str) -> zbus::Result<()>;
        /// The laptop firmware's fan mode.
        fn set_fan_mode(&self, mode: i32) -> zbus::Result<()>;
    }

    /// Installing a release the user chose, and going back.
    #[zbus::proxy(
        interface = "org.unixguardian.Daemon1.Update",
        default_service = "org.unixguardian.Daemon1",
        default_path = "/org/unixguardian/Daemon1"
    )]
    pub trait Update {
        /// The version a rollback returns to; "" when there is none.
        fn previous(&self) -> zbus::Result<String>;
        /// Checks the tarball's signature and installs it (polkit).
        fn install(&self, tarball: &str, signature: &str) -> zbus::Result<String>;
        /// Puts the version from before the last update back (polkit).
        fn rollback(&self) -> zbus::Result<String>;
    }

    /// The graphics driver troubleshooter's changes. Diagnosis itself runs in
    /// the caller (`ug_driver::diagnose`), so it works with the daemon down.
    #[zbus::proxy(
        interface = "org.unixguardian.Daemon1.Driver",
        default_service = "org.unixguardian.Daemon1",
        default_path = "/org/unixguardian/Daemon1"
    )]
    pub trait Driver {
        /// Exactly what a fix will do here, or why it cannot be done.
        fn plan(&self, fix_id: &str) -> zbus::Result<DriverPlan>;
        /// Starts a fix (polkit, password every time); returns the
        /// operation's id. Progress is in `Ops`.
        fn apply(&self, fix_id: &str) -> zbus::Result<String>;
        /// Every fix applied, newest first.
        fn ops(&self) -> zbus::Result<Vec<DriverOp>>;
        /// Undoes an applied fix (polkit).
        fn rollback(&self, op_id: &str) -> zbus::Result<()>;
        /// After the restart a change needed: the user checked it and keeps
        /// it (it is then never undone by itself).
        fn keep(&self, op_id: &str) -> zbus::Result<()>;
        /// This boot's kernel log lines about firmware (only those), for
        /// callers that may not read the journal themselves.
        fn firmware_log(&self) -> zbus::Result<String>;
    }

    /// Internet control: traffic per app (and, next, blocking and limits).
    /// Counting happens in Unix Guardian's own nftables table; other
    /// firewall rules are never touched.
    #[zbus::proxy(
        interface = "org.unixguardian.Daemon1.Net",
        default_service = "org.unixguardian.Daemon1",
        default_path = "/org/unixguardian/Daemon1"
    )]
    pub trait Net {
        /// Whether counting works here, and why not.
        fn status(&self) -> zbus::Result<NetControlStatus>;

        /// "Slow down background apps": on, the shared upload and download
        /// caps (kbit/s) and `always` or `gaming`.
        fn background(&self) -> zbus::Result<(bool, u32, u32, String)>;

        /// Turns "slow down background apps" on or off (polkit, as rules).
        fn set_background(
            &self,
            on: bool,
            up_kbps: u32,
            down_kbps: u32,
            when: &str,
        ) -> zbus::Result<()>;

        /// Every app and service with its traffic, busiest first. Asking
        /// makes the next minute's rates update every two seconds.
        fn apps(&self) -> zbus::Result<Vec<AppTraffic>>;

        /// Every rule, with whether it applies now.
        fn rules(&self) -> zbus::Result<Vec<NetRule>>;

        /// Blocks an app, or caps its upload and/or download in kbit/s
        /// (0 = no cap), `always` or only while `gaming`. Needs the
        /// `org.unixguardian.net.control` polkit action.
        fn set_rule(
            &self,
            app: &str,
            block: bool,
            up_kbps: u32,
            down_kbps: u32,
            when: &str,
        ) -> zbus::Result<()>;

        /// Removes an app's rule; false if it had none.
        fn remove_rule(&self, app: &str) -> zbus::Result<bool>;
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

        /// A Steam game's launch options and what Unix Guardian added to them.
        fn launch_options(&self, appid: u32) -> zbus::Result<LaunchInfo>;
        /// The launch options this choice would give (nothing is written).
        /// `gpu` is a PCI address or "" for the system's choice.
        fn preview_launch_options(
            &self,
            appid: u32,
            gpu: &str,
            overlay: bool,
        ) -> zbus::Result<String>;
        /// Writes them into Steam's settings (Steam must be closed; backed up
        /// first). Returns (options, backup path).
        fn set_launch_options(
            &self,
            appid: u32,
            gpu: &str,
            overlay: bool,
        ) -> zbus::Result<(String, String)>;
        /// Puts the game's own launch options back.
        fn undo_launch_options(&self, appid: u32) -> zbus::Result<(String, String)>;

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

        /// Programs whose time is counted although they are not games. Their
        /// playtime keys are `app:<exe>`.
        fn tracked_apps(&self) -> zbus::Result<Vec<TrackedApp>>;
        /// Starts counting a program's time (never a game, never tuned).
        fn track_app(&self, exe: &str, name: &str) -> zbus::Result<()>;
        /// Stops counting it; its recorded time stays.
        fn untrack_app(&self, exe: &str) -> zbus::Result<bool>;
        /// Programs open now that could be tracked: those with a desktop
        /// entry, and Windows programs.
        fn running_programs(&self) -> zbus::Result<Vec<TrackedApp>>;
        /// "Not a game": a game recognised only by its behaviour (`exe:` key)
        /// becomes a tracked app, with the time recorded so far. Returns the
        /// sessions moved.
        fn move_to_apps(&self, key: &str) -> zbus::Result<u32>;

        /// Reads Steam's own playtime and adds what Unix Guardian had not
        /// counted itself to the all-time totals. Returns (games, seconds
        /// added). Importing again replaces the earlier import.
        fn import_steam(&self) -> zbus::Result<(u32, u64)>;
        /// What earlier imports added, per game (`sessions` is 0).
        fn imported(&self) -> zbus::Result<Vec<PlaytimeEntry>>;
        /// Removes everything imported; returns how many games had some.
        fn forget_imported(&self) -> zbus::Result<u32>;
    }

    /// Monitoring history the agent records once a minute (90 days).
    #[zbus::proxy(
        interface = "org.unixguardian.Agent1.Monitor",
        default_service = "org.unixguardian.Agent1",
        default_path = "/org/unixguardian/Agent1"
    )]
    pub trait Monitor {
        /// Time on battery and on the charger, average draw, screen-on time
        /// and battery health estimates between two times.
        fn battery_report(
            &self,
            since_unix_ms: u64,
            until_unix_ms: u64,
        ) -> zbus::Result<BatteryReport>;

        /// Readings between two times, averaged into at most `points` buckets.
        fn history(
            &self,
            since_unix_ms: u64,
            until_unix_ms: u64,
            points: u32,
        ) -> zbus::Result<Vec<HistoryPoint>>;

        /// Bytes received and sent per local day since `since_unix_ms`.
        fn traffic(&self, since_unix_ms: u64) -> zbus::Result<Vec<DayTraffic>>;

        /// Bytes received and sent in equal buckets from `since_unix_ms`
        /// (an hour each from local midnight: a day's hours).
        fn traffic_buckets(
            &self,
            since_unix_ms: u64,
            until_unix_ms: u64,
            bucket_ms: u64,
        ) -> zbus::Result<Vec<TrafficBucket>>;

        /// Each app's internet use between two times, most used first.
        fn app_traffic(
            &self,
            since_unix_ms: u64,
            until_unix_ms: u64,
        ) -> zbus::Result<Vec<AppUsage>>;

        /// Starts the traffic counts again from now (totals, days and apps).
        fn reset_traffic(&self) -> zbus::Result<bool>;

        /// When the traffic counts were last reset (0: never).
        fn traffic_reset_at(&self) -> zbus::Result<u64>;

        /// The running game's FPS, read from MangoHud's log.
        fn fps(&self) -> zbus::Result<FpsReading>;

        /// Adds (true) or removes (false) Unix Guardian's logging block in
        /// the user's MangoHud.conf; the first change keeps a backup.
        fn set_up_fps(&self, on: bool) -> zbus::Result<()>;

        /// Finished game sessions' frame statistics since `since_unix_ms`.
        fn fps_sessions(&self, since_unix_ms: u64) -> zbus::Result<Vec<FpsSession>>;

        /// Deletes every recorded FPS session; returns how many.
        fn forget_fps_sessions(&self) -> zbus::Result<u32>;
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
        fn compare(
            &self,
            target: &str,
            mark_unix_ms: u64,
            window_s: u32,
        ) -> zbus::Result<PingComparison>;
    }

    /// The tray icon the agent shows.
    #[zbus::proxy(
        interface = "org.unixguardian.Agent1.Tray",
        default_service = "org.unixguardian.Agent1",
        default_path = "/org/unixguardian/Agent1"
    )]
    pub trait Tray {
        /// Puts the icon back in the panel (after "Quit tray icon").
        fn show(&self) -> zbus::Result<()>;
        /// Hides the icon until `show`, across logins.
        fn hide(&self) -> zbus::Result<()>;
        /// Closes the windows and terminal UIs and stops the agent.
        fn quit(&self) -> zbus::Result<()>;
        #[zbus(property)]
        fn visible(&self) -> zbus::Result<bool>;
    }

    /// Global shortcuts (the desktop's GlobalShortcuts portal) and the
    /// actions behind them.
    #[zbus::proxy(
        interface = "org.unixguardian.Agent1.Shortcuts",
        default_service = "org.unixguardian.Agent1",
        default_path = "/org/unixguardian/Agent1"
    )]
    pub trait Shortcuts {
        fn state(&self) -> zbus::Result<ShortcutsState>;
        /// Turns them on (the desktop may ask the user to confirm the keys)
        /// or off. Remembered.
        fn set_enabled(&self, on: bool) -> zbus::Result<()>;
        /// Does what a shortcut does, now; returns what happened.
        fn run(&self, id: &str) -> zbus::Result<String>;
        /// Uses `keys` ("Meta+Shift+G", "LOGO+SHIFT+g"…; "" for the
        /// suggested keys) for shortcut `id`. Remembered; returns what
        /// happened (set in the desktop, asked for, or saved for later).
        fn set_keys(&self, id: &str, keys: &str) -> zbus::Result<String>;
        /// Where there is no portal: the shortcuts in the desktop's own
        /// keyboard settings (GNOME, XFCE).
        fn desktop_keys(&self) -> zbus::Result<DesktopKeys>;
        /// Writes them there (`ug shortcut <id>` on each key) or removes
        /// what was written; returns what happened.
        fn set_up_desktop_keys(&self, on: bool) -> zbus::Result<String>;
    }

    /// New versions from GitHub Releases.
    #[zbus::proxy(
        interface = "org.unixguardian.Agent1.Update",
        default_service = "org.unixguardian.Agent1",
        default_path = "/org/unixguardian/Agent1"
    )]
    pub trait Update {
        /// The last check's result (none is made here).
        fn state(&self) -> zbus::Result<UpdateState>;
        /// Checks now and returns the result.
        fn check(&self) -> zbus::Result<UpdateState>;
        /// Downloads the new release and has the service install it (it
        /// asks for the password); `State` follows the steps.
        fn install(&self) -> zbus::Result<()>;
        /// Goes back to the version before the last update (password).
        fn rollback(&self) -> zbus::Result<String>;
    }

    /// The agent's own settings.
    #[zbus::proxy(
        interface = "org.unixguardian.Agent1.Settings",
        default_service = "org.unixguardian.Agent1",
        default_path = "/org/unixguardian/Agent1"
    )]
    pub trait Settings {
        fn state(&self) -> zbus::Result<SettingsState>;
        /// Saves them; refused with the reason when a value is out of range.
        fn set(&self, settings: &AgentSettings) -> zbus::Result<()>;
    }

    /// The in-game overlay (MangoHud), set up and styled by Unix Guardian.
    #[zbus::proxy(
        interface = "org.unixguardian.Agent1.Overlay",
        default_service = "org.unixguardian.Agent1",
        default_path = "/org/unixguardian/Agent1"
    )]
    pub trait Overlay {
        fn state(&self) -> zbus::Result<OverlayState>;

        /// Saves the settings and rewrites MangoHud's configuration; a
        /// running game picks the change up without restarting.
        fn set(&self, settings: &OverlaySettings) -> zbus::Result<()>;

        /// Adds (true) or removes (false) Unix Guardian's block in the
        /// user's MangoHud.conf, and the session variable if it was set.
        fn set_up(&self, on: bool) -> zbus::Result<()>;
    }

    /// Recording the screen and the game's sound (gpu-screen-recorder).
    #[zbus::proxy(
        interface = "org.unixguardian.Agent1.Recorder",
        default_service = "org.unixguardian.Agent1",
        default_path = "/org/unixguardian/Agent1"
    )]
    pub trait Recorder {
        fn state(&self) -> zbus::Result<RecorderState>;
        /// Saves the settings (used from the next start).
        fn set(&self, settings: &RecorderSettings) -> zbus::Result<()>;
        /// Starts a recording; returns the file it goes to.
        fn start(&self) -> zbus::Result<String>;
        /// Stops a recording or the replay buffer; returns the file written
        /// ("" for the buffer).
        fn stop(&self) -> zbus::Result<String>;
        /// Starts the replay buffer (the last `replay_seconds` in memory).
        fn start_replay(&self) -> zbus::Result<()>;
        /// Writes the replay buffer to a file; returns it.
        fn save_replay(&self) -> zbus::Result<String>;
        /// Installs gpu-screen-recorder for this user (Flatpak from Flathub);
        /// `false` removes it again.
        fn set_up(&self, on: bool) -> zbus::Result<()>;
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

        /// Whether game mode was set by hand (`on`, `off` or empty: it
        /// follows the games) and the profile it uses when on by hand.
        fn game_mode(&self) -> zbus::Result<(String, String)>;

        /// Game mode on now with a profile (empty: the last one chosen), or
        /// off. On applies at the next tick, with or without a game.
        fn set_game_mode(&self, on: bool, profile: &str) -> zbus::Result<()>;

        /// The apps a profile pauses while a game runs, by program name.
        fn set_pause_apps(&self, profile: &str, apps: &[&str]) -> zbus::Result<()>;

        /// A new profile of the user's own, a copy of `from` (empty:
        /// balanced) to change from there.
        fn create(&self, profile: &str, from: &str) -> zbus::Result<()>;

        /// Deletes one of the user's own profiles; its games go back to the
        /// profile the game database suggests.
        fn delete(&self, profile: &str) -> zbus::Result<()>;
    }
}
