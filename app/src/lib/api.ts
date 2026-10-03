// Typed wrappers over the Rust commands (src-tauri/src/bus.rs). The shapes
// mirror ug-proto's wire types as serde_json emits them: a reading the machine
// does not report arrives as null (the daemon's NaN sentinel).

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface CpuSample {
  load_pct: number | null;
  temp_c: number | null;
  power_w: number | null;
  freq_avg_mhz: number;
  freq_max_mhz: number;
  per_core_load: (number | null)[];
  governor: string;
  epp: string;
  driver: string;
}

export interface GpuSample {
  pci: string;
  temp_c: number | null;
  temp_hotspot_c: number | null;
  temp_mem_c: number | null;
  load_pct: number | null;
  power_w: number | null;
  power_limit_w: number | null;
  core_clock_mhz: number;
  mem_clock_mhz: number;
  vram_used_mb: number;
  fan_pct: number | null;
  fan_rpm: number;
  asleep: boolean;
}

export interface MemSample {
  total_kb: number;
  available_kb: number;
  swap_total_kb: number;
  swap_used_kb: number;
  zram_used_kb: number;
}

export interface NetSample {
  iface: string;
  rx_bps: number;
  tx_bps: number;
  rx_total_bytes: number;
  tx_total_bytes: number;
}

export interface Snapshot {
  ts_unix_ms: number;
  cpu: CpuSample;
  gpus: GpuSample[];
  mem: MemSample;
  net: NetSample;
}

export interface ExtraSample {
  ts_unix_ms: number;
  per_core_mhz: number[];
  disk_read_bps: number;
  disk_write_bps: number;
  disk_busy_pct: number;
  battery_pct: number | null;
  battery_w: number | null;
  battery_status: string;
  on_ac: boolean;
  load_avg_1: number | null;
  processes: number;
}

export interface FpsReading {
  state: "live" | "idle" | "not-set-up" | "not-installed";
  game: string;
  fps: number | null;
  frametime_ms: number | null;
  low_1pct_fps: number | null;
  recent: [number, number][];
  install_hint: string;
  mangohud_version: string;
  live_unsupported: boolean;
}

export interface PingStats {
  sent: number;
  received: number;
  median_ms: number | null;
  jitter_ms: number | null;
  p95_ms: number | null;
  loss_pct: number;
}
export interface PingSample { ts_unix_ms: number; rtt_ms: number | null }
export interface PingTarget {
  id: "gateway" | "internet" | "game";
  label: string;
  addr: string;
  samples: PingSample[];
  stats: PingStats;
  problem: string;
  via: string;
  through_vpn: boolean;
}
export interface PingMark { ts_unix_ms: number; kind: string }
export interface PingReport { targets: PingTarget[]; marks: PingMark[] }
export interface PingComparison { before: PingStats; after: PingStats; verdict: "flatter" | "same" | "worse" | "not-enough-data" }

export interface AppTraffic {
  id: string;
  name: string;
  kind: "app" | "service" | "session";
  uid: number;
  exe: string;
  pids: number[];
  rx_bps: number;
  tx_bps: number;
  rx_bytes: number;
  tx_bytes: number;
  state: "" | "blocked" | "limited";
  category: string;
  has_game: boolean;
}
export interface NetRule { app: string; block: boolean; up_kbps: number; down_kbps: number; when: "always" | "gaming"; active: boolean }
export interface NetControlStatus {
  available: boolean;
  problem: string;
  simulated: boolean;
  firewalls: string[];
  shaping_problems: string[];
  iface: string;
  vpn: string;
  vpn_tun: boolean;
}

export interface GpuTune { power_limit_w: number; core_offset_mhz: number; mem_offset_mhz: number }
export interface GpuTuneState {
  pci: string;
  power_limit: boolean;
  power_limit_w: number;
  power_default_w: number;
  power_min_w: number;
  power_max_w: number;
  core_offset: boolean;
  core_offset_mhz: number;
  core_min_mhz: number;
  core_max_mhz: number;
  mem_offset: boolean;
  mem_offset_mhz: number;
  mem_min_mhz: number;
  mem_max_mhz: number;
  notes: string[];
  applied: GpuTune;
  crashed: GpuTune;
}

export interface FanInfo { id: string; label: string; rpm: number; pct: number | null; temp_c: number | null; mode: "auto" | "curve"; curve: [number, number][] }
export interface CoolingState { fans: FanInfo[]; fan_mode: number; fan_modes: [number, string][]; notes: string[]; simulated: boolean }

export interface AutoTuneStatus { running: boolean; step: number; steps: number; message: string; summary: string; ok: boolean }

export interface GpuOffload { pci: string; vendor: string; boot_vga: boolean; launch_options: string }
/** The GPU programs start on; `pending` until the next login. */
export interface GpuDefault { chosen: string; active: string; pending: boolean }

export interface OverlaySettings {
  show: "gaming" | "always" | "off";
  layout: "card" | "compact";
  position: "top-left" | "top-right" | "bottom-left" | "bottom-right" | "top-center";
  size: "small" | "medium" | "large";
  opacity_pct: number;
  toggle_key: string;
  every_game: boolean;
}
export interface OverlayState {
  settings: OverlaySettings;
  installed: boolean;
  set_up: boolean;
  install_hint: string;
  every_game_active: boolean;
  game_running: boolean;
}
export interface DriverFact { key: string; value: string }
export interface DriverFinding {
  id: string;
  severity: "critical" | "important" | "advice";
  category: "graphics" | "wifi" | "bluetooth" | "audio" | "input" | "camera" | "printing" | "sleep" | "firmware";
  title_en: string; title_fa: string;
  what_en: string; what_fa: string;
  why_en: string; why_fa: string;
  how_en: string; how_fa: string;
  fix_id: string;
  /** Unix Guardian can apply it itself, with a backup and undo. */
  fix_auto: boolean;
  commands: string[];
  doc_url: string;
}
export interface DriverReport { facts: DriverFact[]; findings: DriverFinding[]; distro: string; can_change: boolean }
export interface DriverStep { kind: "write-file" | "remove-file" | "run" | "enable-units"; target: string; detail: string; undo: string }
export interface DriverPlan { fix_id: string; title: string; steps: DriverStep[]; needs_reboot: boolean; refused: string }
export interface DriverOp {
  id: string; fix_id: string; title: string; started_unix_ms: number;
  state: "running" | "done" | "failed" | "undoing" | "undone" | "interrupted";
  log: string[]; needs_reboot: boolean;
}
export interface AgentSettings {
  notifications: boolean;
  /** Seconds between looks for running games, 1 to 10. */
  scan_interval_s: number;
  close_window_on_game: boolean;
}
export interface SettingsState {
  settings: AgentSettings;
  agent_version: string;
  config_dir: string;
  data_dir: string;
  /** Set by `ug-agent --scan-interval`; shown, not changeable. */
  scan_interval_fixed: boolean;
}
export interface ShortcutInfo {
  id: "open-window" | "toggle-overlay" | "toggle-gamemode";
  description: string;
  /** As the desktop describes it ("Meta+Shift+O"); "" when not bound. */
  trigger: string;
  /** The keys asked for: the user's (`chosen`) or the suggested ones. */
  keys: string;
  chosen: boolean;
}
export interface ShortcutsState {
  enabled: boolean;
  status: "off" | "connecting" | "waiting" | "active" | "unavailable" | "failed";
  problem: string;
  shortcuts: ShortcutInfo[];
  /** Game mode turned "on" or "off" by hand; "" when it follows the games. */
  gamemode_by_hand: "" | "on" | "off";
}
export interface FpsSession {
  game: string;
  started_unix_ms: number;
  duration_s: number;
  frames: number;
  avg_fps: number;
  low_1pct_fps: number;
  low_01pct_fps: number;
  p99_frametime_ms: number;
  stutters: number;
}

export interface GpuInfo {
  pci: string;
  name: string;
  vendor: "nvidia" | "amd" | "intel" | "unknown";
  device_id: number;
  driver: string;
  driver_version: string;
  vram_total_mb: number;
  boot_vga: boolean;
  card_index: number;
}

export interface CpuTopology {
  model: string;
  logical: number;
  physical: number;
  sockets: number;
  smt: boolean;
  l3_domains: { id: number; size_kb: number; cpus: number[] }[];
  has_vcache: boolean;
  p_cores: number[];
  e_cores: number[];
}

export interface Inventory {
  cpu: CpuTopology;
  gpus: GpuInfo[];
  distro_id: string;
  distro_name: string;
  kernel: string;
  mem_total_kb: number;
  privileged: boolean;
}

export interface Overview {
  inventory: Inventory | null;
  daemon_error: string | null;
  daemon_version: string | null;
  agent_error: string | null;
  gamemode_active: boolean | null;
  active_gpu: string | null;
}

export interface RunningGame {
  key: string;
  name: string;
  evidence: string;
  profile: string;
  pids: number[];
  blocked_note: string;
  started_unix_ms: number;
  gpu: string;
}

export interface InstalledGame {
  id: string;
  name: string;
  launcher: string;
}

export interface Profile {
  id: string;
  builtin: boolean;
  cpu_performance: boolean;
  cpu_no_idle_states: boolean;
  cpu_preferred_cores: boolean;
  cpu_priority: boolean;
  memory_tuning: boolean;
  pause_services: boolean;
  network_tuning: boolean;
  power_profile: boolean;
  pause_apps: string[];
  games: string[];
}

export interface PowerCapability {
  action: PowerAction;
  available: boolean;
  reason: string;
}

export type PowerAction =
  | "power-off" | "reboot" | "suspend" | "hibernate" | "hybrid-sleep" | "suspend-then-hibernate" | "reboot-firmware";

export interface PowerJob {
  id: number;
  action: PowerAction;
  at_unix_ms: number;
  repeat: "once" | "daily" | "weekly";
  note: string;
}

export type WatchKind = "net-idle" | "cpu-idle" | "process-gone" | "game-over";

export interface PowerWatch {
  id: number;
  action: PowerAction;
  kind: WatchKind;
  threshold: number;
  minutes: number;
  name: string;
  note: string;
}

export interface PowerUsage {
  cpu_w: number | null;
  gpu_w: number | null;
  battery_w: number | null;
  cpu_energy_recorded: boolean;
  days: { date: string; cpu_wh: number }[];
}

export interface BatteryCare {
  supported: boolean;
  method: string;
  adjustable: boolean;
  mode: "off" | "protect" | "full-once";
  limit: number;
  capacity: number;
  status: string;
  reason: string;
}

export interface HistoryPoint {
  ts_unix_ms: number;
  cpu_temp_c: number | null;
  cpu_load_pct: number | null;
  cpu_w: number | null;
  gpu_temp_c: number | null;
  gpu_load_pct: number | null;
  gpu_w: number | null;
  mem_used_kb: number;
}

export interface DayTraffic {
  date: string;
  rx_bytes: number;
  tx_bytes: number;
}

export interface PlaytimeEntry {
  key: string;
  name: string;
  seconds: number;
  sessions: number;
  last_played_unix_ms: number;
}

export interface PlaytimeSession {
  key: string;
  name: string;
  started_unix_ms: number;
  ended_unix_ms: number;
}

/** A Steam game's launch options and Unix Guardian's part in them. */
export interface LaunchInfo {
  appid: number;
  name: string;
  current: string;
  own: string;
  /** PCI address of the GPU Unix Guardian set; "" for none. */
  gpu: string;
  overlay: boolean;
  changed_outside: boolean;
  flatpak: boolean;
  steam_running: boolean;
  /** Why the overlay cannot be added; "" when it can. */
  overlay_problem: string;
}

/** A program timed as an app (never a game), or one open now. */
export interface TrackedApp {
  exe: string;
  name: string;
}

const tauriApi = {
  preview: false as boolean,
  overview: () => invoke<Overview>("overview"),
  snapshot: () => invoke<Snapshot>("snapshot"),
  monitorStart: (intervalMs: number) => invoke<void>("monitor_start", { intervalMs }),
  monitorStop: () => invoke<void>("monitor_stop"),
  gamemodeActive: () => invoke<boolean>("gamemode_active"),
  gamesRunning: () => invoke<RunningGame[]>("games_running"),
  gamesInstalled: () => invoke<InstalledGame[]>("games_installed"),
  profiles: () => invoke<Profile[]>("profiles"),
  setGameProfile: (key: string, profile: string) => invoke<void>("set_game_profile", { key, profile }),
  setProfileOption: (profile: string, option: string, value: boolean) =>
    invoke<void>("set_profile_option", { profile, option, value }),
  powerCaps: () => invoke<PowerCapability[]>("power_caps"),
  powerNow: (action: PowerAction) => invoke<void>("power_now", { action }),
  powerSchedule: (action: PowerAction, atUnixMs: number, repeat: string) =>
    invoke<PowerJob>("power_schedule", { action, atUnixMs, repeat }),
  powerJobs: () => invoke<PowerJob[]>("power_jobs"),
  powerCancel: (id: number) => invoke<boolean>("power_cancel", { id }),
  powerWhen: (action: PowerAction, kind: WatchKind, threshold: number, minutes: number, name: string) =>
    invoke<PowerWatch>("power_when", { action, kind, threshold, minutes, name }),
  powerWatches: () => invoke<PowerWatch[]>("power_watches"),
  powerUnwatch: (id: number) => invoke<boolean>("power_unwatch", { id }),
  powerUsage: () => invoke<PowerUsage>("power_usage"),
  history: (sinceUnixMs: number, untilUnixMs: number, points: number) =>
    invoke<HistoryPoint[]>("history", { sinceUnixMs, untilUnixMs, points }),
  traffic: (sinceUnixMs: number) => invoke<DayTraffic[]>("traffic", { sinceUnixMs }),
  extra: () => invoke<ExtraSample>("extra"),
  fps: () => invoke<FpsReading>("fps"),
  setUpFps: (on: boolean) => invoke<void>("set_up_fps", { on }),
  pingRecent: (sinceUnixMs: number) => invoke<PingReport>("ping_recent", { sinceUnixMs }),
  pingCompare: (target: string, markUnixMs: number, windowS: number) =>
    invoke<PingComparison>("ping_compare", { target, markUnixMs, windowS }),
  netStatus: () => invoke<NetControlStatus>("net_status"),
  netApps: () => invoke<AppTraffic[]>("net_apps"),
  netRules: () => invoke<NetRule[]>("net_rules"),
  netSetRule: (app: string, block: boolean, upKbps: number, downKbps: number, when: "always" | "gaming") =>
    invoke<void>("net_set_rule", { app, block, upKbps, downKbps, when }),
  netRemoveRule: (app: string) => invoke<boolean>("net_remove_rule", { app }),
  overlayState: () => invoke<OverlayState>("overlay_state"),
  overlaySet: (settings: OverlaySettings) => invoke<void>("overlay_set", { settings }),
  overlaySetUp: (on: boolean) => invoke<void>("overlay_set_up", { on }),
  shortcutsState: () => invoke<ShortcutsState>("shortcuts_state"),
  shortcutsSetEnabled: (on: boolean) => invoke<void>("shortcuts_set_enabled", { on }),
  shortcutsRun: (id: string) => invoke<string>("shortcuts_run", { id }),
  /** Keys as "Meta+Shift+G"; "" for the suggested ones. Says what happened. */
  shortcutsSetKeys: (id: string, keys: string) => invoke<string>("shortcuts_set_keys", { id, keys }),
  fpsSessions: (sinceUnixMs: number) => invoke<FpsSession[]>("fps_sessions", { sinceUnixMs }),
  forgetFpsSessions: () => invoke<number>("forget_fps_sessions"),
  gpuOffload: () => invoke<GpuOffload[]>("gpu_offload"),
  gpuDefault: () => invoke<GpuDefault>("gpu_default"),
  setGpuDefault: (pci: string) => invoke<GpuDefault>("set_gpu_default", { pci }),
  coolingState: () => invoke<CoolingState>("cooling_state"),
  setFanCurve: (id: string, points: [number, number][]) => invoke<void>("set_fan_curve", { id, points }),
  setFanAuto: (id: string) => invoke<void>("set_fan_auto", { id }),
  setFanMode: (mode: number) => invoke<void>("set_fan_mode", { mode }),
  powerProfile: () => invoke<string>("power_profile"),
  setPowerProfile: (profile: string) => invoke<void>("set_power_profile", { profile }),
  gpuTuneState: (pci: string) => invoke<GpuTuneState>("gpu_tune_state", { pci }),
  gpuTryTune: (pci: string, tune: GpuTune) => invoke<void>("gpu_try_tune", { pci, tune }),
  gpuKeepTune: (pci: string) => invoke<void>("gpu_keep_tune", { pci }),
  gpuAutoTune: (pci: string, goal: "faster" | "cooler") => invoke<void>("gpu_auto_tune", { pci, goal }),
  gpuAutoTuneStatus: () => invoke<AutoTuneStatus>("gpu_auto_tune_status"),
  gpuStopAutoTune: () => invoke<void>("gpu_stop_auto_tune"),
  gpuTestTune: (pci: string, seconds: number) => invoke<[boolean, string]>("gpu_test_tune", { pci, seconds }),
  gpuCancelTune: (pci: string) => invoke<void>("gpu_cancel_tune", { pci }),
  gpuResetTune: (pci: string) => invoke<void>("gpu_reset_tune", { pci }),
  openTerminal: () => invoke<void>("open_terminal_interface"),
  /** null: no agent, or no tray. */
  trayVisible: () => invoke<boolean | null>("tray_visible"),
  trayShow: () => invoke<void>("tray_show"),
  trayHide: () => invoke<void>("tray_hide"),
  driverDiagnose: () => invoke<DriverReport>("driver_diagnose"),
  driverPlan: (fixId: string) => invoke<DriverPlan>("driver_plan", { fixId }),
  driverApply: (fixId: string) => invoke<string>("driver_apply", { fixId }),
  driverOps: () => invoke<DriverOp[]>("driver_ops"),
  driverRollback: (opId: string) => invoke<void>("driver_rollback", { opId }),
  settingsState: () => invoke<SettingsState>("settings_state"),
  settingsSet: (settings: AgentSettings) => invoke<void>("settings_set", { settings }),
  reportBug: (context: string) => invoke<void>("report_bug", { context }),
  battery: () => invoke<BatteryCare>("battery"),
  setBatteryCare: (mode: string, limit: number) => invoke<void>("set_battery_care", { mode, limit }),
  wakeAlarm: () => invoke<[number, boolean]>("wake_alarm"),
  setWakeAlarm: (atUnixMs: number) => invoke<void>("set_wake_alarm", { atUnixMs }),
  clearWakeAlarm: () => invoke<void>("clear_wake_alarm"),
  playtime: (sinceUnixMs: number) => invoke<PlaytimeEntry[]>("playtime", { sinceUnixMs }),
  playtimeSessions: (sinceUnixMs: number) => invoke<PlaytimeSession[]>("playtime_sessions", { sinceUnixMs }),
  playtimeForget: (key: string) => invoke<number>("playtime_forget", { key }),
  /** Writes a CSV into Downloads; returns its path. */
  playtimeExport: (sinceUnixMs: number) => invoke<string>("playtime_export", { sinceUnixMs }),
  trackedApps: () => invoke<TrackedApp[]>("playtime_tracked_apps"),
  launchOptions: (appid: number) => invoke<LaunchInfo>("steam_launch_options", { appid }),
  previewLaunchOptions: (appid: number, gpu: string, overlay: boolean) => invoke<string>("steam_preview_launch_options", { appid, gpu, overlay }),
  /** Returns [options, backup path]. */
  setLaunchOptions: (appid: number, gpu: string, overlay: boolean) => invoke<[string, string]>("steam_set_launch_options", { appid, gpu, overlay }),
  undoLaunchOptions: (appid: number) => invoke<[string, string]>("steam_undo_launch_options", { appid }),
  trackApp: (exe: string, name: string) => invoke<void>("playtime_track_app", { exe, name }),
  untrackApp: (exe: string) => invoke<boolean>("playtime_untrack_app", { exe }),
  runningPrograms: () => invoke<TrackedApp[]>("playtime_running_programs"),
  /** A guessed game (key exe:…) becomes a tracked app; returns sessions moved. */
  moveToApps: (key: string) => invoke<number>("playtime_move_to_apps", { key }),
  /** Returns [games, seconds added]. */
  importSteam: () => invoke<[number, number]>("playtime_import_steam"),
  playtimeImported: () => invoke<PlaytimeEntry[]>("playtime_imported"),
  forgetImported: () => invoke<number>("playtime_forget_imported"),
  onSample: (f: (s: Snapshot) => void): Promise<UnlistenFn> => listen<Snapshot>("sample", (e) => f(e.payload)),
};

/** Inside Tauri, the real bridge; in a plain browser (design preview), a
 * replay of captured readings that says so on screen. */
export const api: typeof tauriApi =
  "__TAURI_INTERNALS__" in window ? tauriApi : ((await import("./mock")).mockApi as unknown as typeof tauriApi);

/** The commands reject with the Rust side's sentence; anything else is a bug. */
export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}
