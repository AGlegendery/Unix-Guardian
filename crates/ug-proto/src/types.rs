//! Wire types shared by every Unix Guardian process.
//!
//! See the crate docs for the sentinel convention: `NaN` / `0` / `""` mean "this
//! machine does not report that", and the `Option`-returning accessors are the
//! supported way to read them.

use serde::{Deserialize, Serialize};
use zbus::zvariant::Type;

/// Returns `None` for the float sentinel, `Some(v)` otherwise.
#[inline]
fn opt_f64(v: f64) -> Option<f64> {
    if v.is_nan() {
        None
    } else {
        Some(v)
    }
}

/// Returns `None` for the empty-string sentinel.
#[inline]
fn opt_str(s: &str) -> Option<&str> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Which driver stack a GPU is on. Determines which tuning paths exist at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[zvariant(signature = "s")]
#[serde(rename_all = "lowercase")]
pub enum Vendor {
    /// `nvidia` or `nvidia-open` kernel module; telemetry and tuning via NVML.
    Nvidia,
    /// `amdgpu`; telemetry and tuning via sysfs.
    Amd,
    /// `i915` / `xe`; telemetry only.
    Intel,
    /// A GPU we recognise as a display device but have no backend for.
    Unknown,
}

impl Vendor {
    /// Maps a PCI vendor ID (`/sys/.../device/vendor`) to a backend.
    pub fn from_pci_id(id: u16) -> Self {
        match id {
            0x10de => Vendor::Nvidia,
            0x1002 | 0x1022 => Vendor::Amd,
            0x8086 => Vendor::Intel,
            _ => Vendor::Unknown,
        }
    }
}

impl std::fmt::Display for Vendor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Vendor::Nvidia => "NVIDIA",
            Vendor::Amd => "AMD",
            Vendor::Intel => "Intel",
            Vendor::Unknown => "Unknown",
        })
    }
}

/// A GPU as the machine presents it, independent of what it can be told to do.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct GpuInfo {
    /// PCI address in `0000:01:00.0` form. The stable identity of a GPU across
    /// reboots and driver reloads; every other call takes this as the key.
    pub pci: String,
    /// Marketing name, e.g. `"NVIDIA GeForce RTX 3050 Laptop GPU"`.
    pub name: String,
    pub vendor: Vendor,
    /// PCI device ID, useful for matching generation-specific quirks.
    pub device_id: u32,
    /// Kernel module currently bound, e.g. `"nvidia"`, `"amdgpu"`, `"i915"`.
    pub driver: String,
    /// Driver version as the stack reports it; `""` when unknown.
    pub driver_version: String,
    /// Total video memory in MiB; `0` when the driver does not report it.
    pub vram_total_mb: u64,
    /// True for the GPU the firmware posted — normally the integrated one on a
    /// hybrid laptop, which is exactly the case where it must not be tuned.
    pub boot_vga: bool,
    /// `/sys/class/drm/cardN` index, or `-1` if the GPU has no DRM node.
    pub card_index: i32,
}

/// What a GPU will actually let us change on this machine.
///
/// Capability is not a property of the model alone: the same card reports
/// different capabilities depending on driver version, kernel boot parameters
/// (`amdgpu.ppfeaturemask`), and whether the daemon is running privileged.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct GpuCapabilities {
    /// Power limit is writable, and `power_limit_min_w`/`_max_w` are meaningful.
    pub power_limit: bool,
    /// Core and memory clock offsets can be applied.
    pub clock_offset: bool,
    /// An absolute voltage or voltage offset can be set. False on NVIDIA — the
    /// Linux driver exposes no voltage/frequency curve at all.
    pub voltage: bool,
    /// Fan speed can be driven manually.
    pub fan_control: bool,
    /// A multi-point fan curve can be handed to the hardware directly, rather
    /// than emulated by the daemon's control loop.
    pub fan_curve: bool,
    pub power_limit_min_w: f64,
    pub power_limit_max_w: f64,
    /// Inclusive bounds for the core clock offset, in MHz.
    pub core_offset_min_mhz: i32,
    pub core_offset_max_mhz: i32,
    /// Human-readable reasons for whatever is false above, e.g.
    /// `"amdgpu.ppfeaturemask is not set — add it to your boot parameters"`.
    /// Shown verbatim next to the disabled control.
    pub notes: Vec<String>,
}

/// One read of the CPU package.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CpuSample {
    /// Busy fraction across all cores, 0–100.
    pub load_pct: f64,
    /// Package temperature in °C, or `NaN`.
    pub temp_c: f64,
    /// Package power draw in watts from RAPL, or `NaN` where RAPL is absent.
    pub power_w: f64,
    /// Mean current frequency across online cores, MHz; `0` if unreadable.
    pub freq_avg_mhz: u32,
    /// Highest current frequency across cores, MHz — the number that shows a
    /// single-threaded boost the average hides.
    pub freq_max_mhz: u32,
    /// Per-core load, 0–100, in kernel CPU order.
    pub per_core_load: Vec<f64>,
    /// Active scaling governor, e.g. `"performance"`.
    pub governor: String,
    /// Energy/performance preference, e.g. `"balance_power"`; `""` if absent.
    pub epp: String,
    /// Scaling driver, e.g. `"intel_pstate"`, `"amd_pstate_epp"`.
    pub driver: String,
}

impl CpuSample {
    pub fn temp(&self) -> Option<f64> {
        opt_f64(self.temp_c)
    }
    pub fn power(&self) -> Option<f64> {
        opt_f64(self.power_w)
    }
    pub fn epp(&self) -> Option<&str> {
        opt_str(&self.epp)
    }
}

/// One read of a single GPU.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct GpuSample {
    /// PCI address, matching [`GpuInfo::pci`].
    pub pci: String,
    /// Core temperature in °C, or `NaN`.
    pub temp_c: f64,
    /// Hotspot/junction temperature in °C — the number that actually throttles
    /// the card. `NaN` on hardware that does not expose it.
    pub temp_hotspot_c: f64,
    /// Memory junction temperature in °C, or `NaN`.
    pub temp_mem_c: f64,
    /// Utilisation 0–100, or `NaN`.
    pub load_pct: f64,
    /// Board power draw in watts, or `NaN`.
    pub power_w: f64,
    /// Currently enforced power limit in watts, or `NaN`.
    pub power_limit_w: f64,
    pub core_clock_mhz: u32,
    pub mem_clock_mhz: u32,
    pub vram_used_mb: u64,
    /// Fan duty 0–100, or `NaN` on a passively cooled or unreported card.
    pub fan_pct: f64,
    /// Fan speed in RPM; `0` when unreported.
    pub fan_rpm: u32,
    /// The GPU is runtime-suspended (PCI D3) and was deliberately not read.
    ///
    /// On a hybrid laptop the discrete GPU sleeps whenever nothing is rendering
    /// on it, and any query — NVML or `amdgpu` sysfs alike — wakes it, which
    /// takes over a second and keeps it powered for as long as polling
    /// continues. A monitor that woke it every second would quietly cost the
    /// user several watts of battery. When this is true, `load_pct` is `0`
    /// (a suspended GPU is idle by definition) and every other reading is a
    /// sentinel; the UI should say "sleeping", not "no data".
    pub asleep: bool,
}

impl GpuSample {
    pub fn temp(&self) -> Option<f64> {
        opt_f64(self.temp_c)
    }
    pub fn hotspot(&self) -> Option<f64> {
        opt_f64(self.temp_hotspot_c)
    }
    pub fn load(&self) -> Option<f64> {
        opt_f64(self.load_pct)
    }
    pub fn power(&self) -> Option<f64> {
        opt_f64(self.power_w)
    }
    pub fn fan(&self) -> Option<f64> {
        opt_f64(self.fan_pct)
    }
}

/// One read of system memory.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct MemSample {
    pub total_kb: u64,
    /// `MemAvailable` — what a new allocation can actually get, which is the
    /// number worth showing, not `MemFree`.
    pub available_kb: u64,
    pub swap_total_kb: u64,
    pub swap_used_kb: u64,
    /// Compressed data currently held in zram, KiB; `0` when zram is not in use.
    pub zram_used_kb: u64,
}

impl MemSample {
    /// Used memory in KiB, derived the way a user reads it: total minus available.
    pub fn used_kb(&self) -> u64 {
        self.total_kb.saturating_sub(self.available_kb)
    }
    /// Used fraction 0–100.
    pub fn used_pct(&self) -> f64 {
        if self.total_kb == 0 {
            return f64::NAN;
        }
        self.used_kb() as f64 * 100.0 / self.total_kb as f64
    }
}

/// One read of the default-route network interface.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct NetSample {
    /// Interface carrying the default route, e.g. `"wlan0"`; `""` when offline.
    pub iface: String,
    /// Receive rate in bytes per second, averaged over the sampling interval.
    pub rx_bps: u64,
    pub tx_bps: u64,
    pub rx_total_bytes: u64,
    pub tx_total_bytes: u64,
}

/// Everything sampled at one instant. The unit the monitoring UI consumes.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct Snapshot {
    /// Wall-clock time of the read, milliseconds since the Unix epoch.
    pub ts_unix_ms: u64,
    pub cpu: CpuSample,
    /// One entry per GPU, in the same order as [`GpuInfo`] from `Gpu.List`.
    pub gpus: Vec<GpuSample>,
    pub mem: MemSample,
    pub net: NetSample,
}

/// CPU layout, including the parts that decide whether core pinning is worth
/// offering at all.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct CpuTopology {
    /// Model name from `/proc/cpuinfo`.
    pub model: String,
    /// Online logical CPUs.
    pub logical: u32,
    /// Physical cores, counting an SMT pair as one.
    pub physical: u32,
    pub sockets: u32,
    /// True when SMT/Hyper-Threading is enabled.
    pub smt: bool,
    /// One entry per L3 domain. On AMD these are the CCDs; pinning a game to the
    /// right one is the whole point of reading this.
    pub l3_domains: Vec<L3Domain>,
    /// True when any L3 domain is large enough to be 3D V-Cache (>= 96 MiB),
    /// which is when "lock the game to the cache CCD" becomes meaningful.
    pub has_vcache: bool,
    /// Performance cores on an Intel hybrid CPU (from `/sys/devices/cpu_core`);
    /// empty on non-hybrid parts.
    pub p_cores: Vec<u32>,
    /// Efficiency cores on an Intel hybrid CPU (from `/sys/devices/cpu_atom`).
    pub e_cores: Vec<u32>,
}

/// One L3 cache domain and the logical CPUs that share it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct L3Domain {
    /// Kernel's cache ID for this domain.
    pub id: u32,
    /// L3 size in KiB.
    pub size_kb: u64,
    /// Logical CPUs sharing this L3.
    pub cpus: Vec<u32>,
}

/// Static description of the machine. Fetched once and cached by clients.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct Inventory {
    pub cpu: CpuTopology,
    pub gpus: Vec<GpuInfo>,
    /// `ID` from `/etc/os-release`, e.g. `"arch"`, `"debian"`, `"fedora"`.
    pub distro_id: String,
    /// `PRETTY_NAME` from `/etc/os-release`.
    pub distro_name: String,
    /// `uname -r`.
    pub kernel: String,
    /// Total RAM in KiB.
    pub mem_total_kb: u64,
    /// True when the daemon is running with the privileges its write paths need.
    /// The UI shows read-only mode rather than failing calls when this is false.
    pub privileged: bool,
}

/// A game that is running now, as the agent reports it.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct RunningGameInfo {
    /// Stable identity, e.g. `steam:730`, `lutris:3`, `exe:somegame`.
    pub key: String,
    pub name: String,
    /// How the game was recognised: `steam_app_id`, `game_mode`, `library`,
    /// `database` or `heuristic`.
    pub evidence: String,
    /// Profile in effect: `competitive`, `balanced` or `quiet`.
    pub profile: String,
    pub pids: Vec<i32>,
    /// Non-empty when this game's anti-cheat is known to refuse Linux.
    pub blocked_note: String,
    pub started_unix_ms: u64,
    /// PCI address of the GPU the game renders on (from the device files it
    /// uses, never by asking the GPU); empty until it draws.
    pub gpu: String,
}

/// A game some launcher reports as installed.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct InstalledGameInfo {
    pub id: String,
    pub name: String,
    /// `steam`, `lutris`, `heroic` or `bottles`.
    pub launcher: String,
}

/// Time spent in one game over a period.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PlaytimeEntry {
    pub key: String,
    pub name: String,
    pub seconds: u64,
    pub sessions: u32,
    pub last_played_unix_ms: u64,
}

/// One recorded play session, for exporting history.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PlaytimeSession {
    pub key: String,
    pub name: String,
    pub started_unix_ms: u64,
    /// For a game still running, the last heartbeat (at most a minute ago).
    pub ended_unix_ms: u64,
}

/// A game profile, with the games the user has assigned to it.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ProfileInfo {
    pub id: String,
    /// One of the shipped profiles (`competitive`, `balanced`, `quiet`). They
    /// can be changed through a user file but not removed.
    pub builtin: bool,
    pub cpu_performance: bool,
    pub cpu_no_idle_states: bool,
    pub cpu_preferred_cores: bool,
    pub cpu_priority: bool,
    pub memory_tuning: bool,
    pub pause_services: bool,
    pub network_tuning: bool,
    pub pause_apps: Vec<String>,
    /// Game keys the user chose this profile for. Games that use it because
    /// the database suggests it are not listed.
    pub games: Vec<String>,
}

/// What the agent asks the daemon to do while games run. Built from the
/// game's profile; each flag maps to one reversible change.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct GameModeRequest {
    pub cpu_performance: bool,
    pub cpu_no_idle_states: bool,
    pub cpu_priority: bool,
    pub memory_tuning: bool,
    pub pause_services: bool,
    pub network_tuning: bool,
    /// Game processes. The daemon refuses any that do not belong to the caller.
    pub pids: Vec<i32>,
}

/// What game mode changed, and what it could not, as sentences for the UI.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct GameModeReport {
    pub applied: Vec<String>,
    pub skipped: Vec<String>,
}

/// Whether this machine can do one power action, and if not, why not.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PowerCapability {
    /// `power-off`, `reboot`, `suspend`, `hibernate`, `hybrid-sleep` or
    /// `suspend-then-hibernate`.
    pub action: String,
    pub available: bool,
    /// Empty when available; otherwise a sentence to show beside the
    /// disabled control.
    pub reason: String,
}

/// A scheduled power action.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PowerJob {
    pub id: u32,
    pub action: String,
    pub at_unix_ms: u64,
    /// `once`, `daily` or `weekly`.
    pub repeat: String,
    pub note: String,
}

/// A power action waiting for a condition.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PowerWatch {
    pub id: u32,
    pub action: String,
    /// `net-idle`, `cpu-idle`, `process-gone` or `game-over`.
    pub kind: String,
    /// net-idle: bytes per second; cpu-idle: percent; otherwise 0.
    pub threshold: f64,
    /// How long the condition must hold (net-idle, cpu-idle); otherwise 0.
    pub minutes: u32,
    /// process-gone: the process name; otherwise empty.
    pub name: String,
    pub note: String,
}

/// Energy the CPU package used on one local day.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DayEnergy {
    /// `YYYY-MM-DD`, local time.
    pub date: String,
    pub cpu_wh: f64,
}

/// Power draw now and energy used so far.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PowerUsage {
    /// CPU package, watts; NaN without RAPL.
    pub cpu_w: f64,
    /// Awake GPUs that report power, watts; NaN when none does.
    pub gpu_w: f64,
    /// Whole machine while on battery, watts; NaN on AC.
    pub battery_w: f64,
    /// Whether per-day CPU energy is being recorded on this machine.
    pub cpu_energy_recorded: bool,
    pub days: Vec<DayEnergy>,
}

/// Battery protection: what the laptop supports and what is set.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BatteryCare {
    pub supported: bool,
    /// `threshold`, `vendor-long-life`, `vendor-conservation` or empty.
    pub method: String,
    /// True when any limit can be chosen; false when the firmware fixes it.
    pub adjustable: bool,
    /// `off`, `protect` or `full-once`.
    pub mode: String,
    pub limit: u8,
    /// Battery charge now, 0–100; 255 without a battery.
    pub capacity: u8,
    /// `Charging`, `Discharging`, `Full`, `Not charging`… or empty.
    pub status: String,
    /// Why it is unsupported, when it is.
    pub reason: String,
}

/// One point of the monitoring history (averaged over its bucket). NaN where
/// nothing was measured.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct HistoryPoint {
    pub ts_unix_ms: u64,
    pub cpu_temp_c: f64,
    pub cpu_load_pct: f64,
    pub cpu_w: f64,
    /// The GPU that was working at the time (the discrete one only while awake).
    pub gpu_temp_c: f64,
    pub gpu_load_pct: f64,
    pub gpu_w: f64,
    pub mem_used_kb: u64,
}

/// Internet traffic on one local day.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DayTraffic {
    /// `YYYY-MM-DD`, local time.
    pub date: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

/// Ping statistics over a window of samples.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct PingStats {
    pub sent: u32,
    pub received: u32,
    /// NaN when no reply arrived.
    pub median_ms: f64,
    /// Mean difference between consecutive replies (RFC 3550 style).
    pub jitter_ms: f64,
    pub p95_ms: f64,
    pub loss_pct: f64,
}

/// One echo: when it was sent and its round trip, NaN if it was lost.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Type)]
pub struct PingSample {
    pub ts_unix_ms: u64,
    pub rtt_ms: f64,
}

/// A host the agent pings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct PingTarget {
    /// `gateway`, `internet` or `game`.
    pub id: String,
    /// `Router`, `DNS server`, or the game's name.
    pub label: String,
    pub addr: String,
    pub samples: Vec<PingSample>,
    /// Over the last 60 seconds.
    pub stats: PingStats,
    /// Why this host cannot be measured; empty when it can.
    pub problem: String,
    /// The interface the echoes leave by, e.g. `wlp8s0` or a VPN's `tun0`.
    pub via: String,
    /// They go through a VPN's TUN device, so this is the ping through the
    /// VPN, not the direct one.
    pub through_vpn: bool,
}

/// A moment the chart splits at, e.g. when game mode or shaping went on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct PingMark {
    pub ts_unix_ms: u64,
    /// `gamemode-on`, `gamemode-off`, `shaping-on`, `shaping-off`.
    pub kind: String,
}

/// What the ping chart shows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct PingReport {
    pub targets: Vec<PingTarget>,
    pub marks: Vec<PingMark>,
}

/// Before and after a mark, computed from real samples.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct PingComparison {
    pub before: PingStats,
    pub after: PingStats,
    /// `flatter`, `same`, `worse`, or `not-enough-data`.
    pub verdict: String,
}

/// Internet use of one app or service, counted by its cgroup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct AppTraffic {
    /// The cgroup path, relative to the cgroup root; stable while it runs.
    pub id: String,
    /// `Firefox`, `Steam`, `packagekit`.
    pub name: String,
    /// `app`, `service` (system or user service) or `session` (a login
    /// session: programs started from a terminal outside an app's own group).
    pub kind: String,
    /// Owner: the user whose slice it runs in, 0 for system services.
    pub uid: u32,
    /// The first process's command name, e.g. `firefox-esr`.
    pub exe: String,
    pub pids: Vec<i32>,
    pub rx_bps: u64,
    pub tx_bps: u64,
    /// Since the service started counting it.
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    /// `blocked`, `limited`, or empty.
    pub state: String,
    /// What kind of background task it is known to be: `torrent`, `sync`,
    /// `updates`, `launcher`, `backup`, `streaming`; empty when unknown.
    pub category: String,
    /// The running game is inside this group (e.g. a game Steam started runs
    /// in Steam's group), so limiting it would limit the game too.
    pub has_game: bool,
}

/// A user's rule for one app's internet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct NetRule {
    /// The app's name as `Apps` reports it.
    pub app: String,
    pub block: bool,
    /// Kilobits per second; 0 = no cap.
    pub up_kbps: u32,
    pub down_kbps: u32,
    /// `always` or `gaming` (only while game mode is on).
    pub when: String,
    /// Whether it applies right now.
    pub active: bool,
}

/// Whether internet control works here, and why not.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct NetControlStatus {
    pub available: bool,
    /// Why it is unavailable; empty when it works.
    pub problem: String,
    /// A development service: numbers are invented and nothing is changed.
    pub simulated: bool,
    /// Other firewalls found (`firewalld`, `ufw`); ours coexists with them.
    pub firewalls: Vec<String>,
    /// Why a cap is not in effect right now (another tool shapes the
    /// interface); empty when every rule applies.
    pub shaping_problems: Vec<String>,
    /// The interface caps are applied to: the one the internet is reached
    /// through, a VPN's TUN device when one carries the traffic.
    pub iface: String,
    /// A running VPN or proxy app (`v2rayn`), or empty. Its traffic with the
    /// apps it carries is not counted again.
    pub vpn: String,
    /// The internet is reached through that VPN's TUN device.
    pub vpn_tun: bool,
}

/// Readings beyond the snapshot, for the live monitor. Fetched with
/// `System.Extra` so older clients keep the snapshot they know.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct ExtraSample {
    pub ts_unix_ms: u64,
    /// Current clock of each CPU, MHz, in kernel order; 0 where unreadable.
    pub per_core_mhz: Vec<u32>,
    /// Summed over whole disks (partitions are not counted twice).
    pub disk_read_bps: u64,
    pub disk_write_bps: u64,
    /// Time the busiest disk spent working, 0–100.
    pub disk_busy_pct: f64,
    /// Battery charge 0–100, NaN without a battery.
    pub battery_pct: f64,
    /// Battery power in watts, NaN when unreported; positive = draining.
    pub battery_w: f64,
    /// `Charging`, `Discharging`, `Full`, `Not charging`, or empty.
    pub battery_status: String,
    pub on_ac: bool,
    pub load_avg_1: f64,
    pub processes: u32,
}

/// Frames per second of the running game, as MangoHud logs it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct FpsReading {
    /// `live` (a game is being measured), `idle` (set up, nothing running
    /// with MangoHud), `not-set-up` (MangoHud installed, logging not set up)
    /// or `not-installed`.
    pub state: String,
    /// The game's name from MangoHud's log file name.
    pub game: String,
    /// Average over the last second; NaN when not live.
    pub fps: f64,
    pub frametime_ms: f64,
    /// FPS at the slowest 1% of frames over the last ten seconds.
    pub low_1pct_fps: f64,
    /// About ten seconds of readings, oldest first: (Unix ms, FPS).
    pub recent: Vec<(u64, f64)>,
    /// The command that installs MangoHud here, for `not-installed`.
    pub install_hint: String,
    /// The installed MangoHud's version, e.g. `0.7.1`; empty if unknown.
    pub mangohud_version: String,
    /// This MangoHud writes its frame log only when the game closes (before
    /// 0.8), so there is no live FPS; sessions are still recorded.
    pub live_unsupported: bool,
}

/// How the in-game overlay looks and when it shows. Unix Guardian writes
/// MangoHud's configuration from this; the user never edits that file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct OverlaySettings {
    /// `gaming` (only while a game runs), `always` (every app MangoHud is
    /// loaded into) or `off` (hidden; FPS is still recorded).
    pub show: String,
    /// `card` (full: CPU, GPU, memory, frame-time graph) or `compact` (one
    /// line: FPS and temperatures).
    pub layout: String,
    /// `top-left`, `top-right`, `bottom-left`, `bottom-right`, `top-center`.
    pub position: String,
    /// `small`, `medium`, `large`.
    pub size: String,
    /// Background opacity, 0–100.
    pub opacity_pct: u8,
    /// MangoHud key names, e.g. `Alt_L+o` (shows and hides the overlay
    /// while the game has focus).
    pub toggle_key: String,
    /// Load MangoHud into every Vulkan game (`MANGOHUD=1` in the session;
    /// takes effect at the next login).
    pub every_game: bool,
}

impl Default for OverlaySettings {
    fn default() -> Self {
        OverlaySettings {
            show: "gaming".into(),
            layout: "card".into(),
            position: "top-left".into(),
            size: "medium".into(),
            opacity_pct: 50,
            toggle_key: "Alt_L+o".into(),
            every_game: false,
        }
    }
}

/// The overlay's settings with what this machine allows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct OverlayState {
    pub settings: OverlaySettings,
    pub installed: bool,
    /// Unix Guardian's block is in MangoHud.conf.
    pub set_up: bool,
    /// The command that installs MangoHud here.
    pub install_hint: String,
    /// `MANGOHUD=1` is in this session's environment now (a login after
    /// turning `every_game` on).
    pub every_game_active: bool,
    /// A game is running, so `gaming` means shown right now.
    pub game_running: bool,
}

/// One game session's frame statistics, from MangoHud's per-frame log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct FpsSession {
    pub game: String,
    pub started_unix_ms: u64,
    pub duration_s: u64,
    pub frames: u64,
    pub avg_fps: f64,
    /// FPS at the 99th percentile frame time (the standard benchmark 1% low).
    pub low_1pct_fps: f64,
    /// FPS at the 99.9th percentile frame time.
    pub low_01pct_fps: f64,
    pub p99_frametime_ms: f64,
    /// Frames that took more than twice the median frame time.
    pub stutters: u64,
}

/// What can be tuned on one GPU right now, and where it stands. Reading it
/// may wake a sleeping discrete GPU, so it is only asked for on request.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct GpuTuneState {
    pub pci: String,
    pub power_limit: bool,
    pub power_limit_w: f64,
    pub power_default_w: f64,
    pub power_min_w: f64,
    pub power_max_w: f64,
    pub core_offset: bool,
    pub core_offset_mhz: i32,
    pub core_min_mhz: i32,
    pub core_max_mhz: i32,
    pub mem_offset: bool,
    pub mem_offset_mhz: i32,
    pub mem_min_mhz: i32,
    pub mem_max_mhz: i32,
    /// Why anything above is unavailable, in words for the user.
    pub notes: Vec<String>,
    /// What Unix Guardian has applied (and keeps applying), if anything.
    pub applied: GpuTune,
    /// The last setting was being tried when the machine or the service
    /// stopped unexpectedly, so it was not applied again.
    pub crashed: GpuTune,
}

/// A tuning request. `power_limit_w` 0 keeps the card's default; offsets 0
/// are none.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct GpuTune {
    pub power_limit_w: f64,
    pub core_offset_mhz: i32,
    pub mem_offset_mhz: i32,
}

impl GpuTune {
    pub fn is_default(&self) -> bool {
        self.power_limit_w == 0.0 && self.core_offset_mhz == 0 && self.mem_offset_mhz == 0
    }
}
