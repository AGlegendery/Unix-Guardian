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

export interface PlaytimeEntry {
  key: string;
  name: string;
  seconds: number;
  sessions: number;
  last_played_unix_ms: number;
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
  wakeAlarm: () => invoke<[number, boolean]>("wake_alarm"),
  setWakeAlarm: (atUnixMs: number) => invoke<void>("set_wake_alarm", { atUnixMs }),
  clearWakeAlarm: () => invoke<void>("clear_wake_alarm"),
  playtime: (sinceUnixMs: number) => invoke<PlaytimeEntry[]>("playtime", { sinceUnixMs }),
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
