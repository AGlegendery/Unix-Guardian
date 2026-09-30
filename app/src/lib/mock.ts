// Browser preview only (vite dev without Tauri): replays readings captured
// from the development laptop, gently varied so the page moves. The UI labels
// itself as a preview whenever this is in use; nothing here reaches a build
// that runs inside Tauri.

import data from "./preview-data.json";
import type { BatteryCare, InstalledGame, Overview, PlaytimeEntry, PowerAction, PowerJob, PowerUsage, PowerWatch, WatchKind, Profile, RunningGame, Snapshot } from "./api";

const base = data.snapshot as unknown as Snapshot;
const profiles = structuredClone(data.profiles) as Profile[];
const jobs: PowerJob[] = [];
let alarm = 0;
const bat: BatteryCare = { supported: true, method: "vendor-long-life", adjustable: false, mode: "off", limit: 80, capacity: 100, status: "Full", reason: "" };
const watches: PowerWatch[] = [];
let t = 0;

function jitter(v: number | null, spread: number, lo = 0, hi = 100): number | null {
  if (v == null) return null;
  return Math.min(hi, Math.max(lo, v + (Math.sin(t / 3 + v) + Math.random() - 0.5) * spread));
}

function next(): Snapshot {
  t += 1;
  const s: Snapshot = structuredClone(base);
  s.ts_unix_ms = Date.now();
  s.cpu.load_pct = jitter(base.cpu.load_pct, 14);
  s.cpu.temp_c = jitter(base.cpu.temp_c, 3, 30, 100);
  s.cpu.power_w = jitter(base.cpu.power_w, 9, 5, 120);
  s.cpu.per_core_load = base.cpu.per_core_load.map((c) => jitter(c, 30));
  s.net.rx_bps = Math.max(0, base.net.rx_bps * (0.4 + Math.random()));
  s.net.tx_bps = Math.max(0, base.net.tx_bps * (0.4 + Math.random()));
  return s;
}

let timer: ReturnType<typeof setInterval> | null = null;
const listeners = new Set<(s: Snapshot) => void>();

export const mockApi = {
  preview: true as const,
  overview: async (): Promise<Overview> => ({
    inventory: data.inventory as unknown as Overview["inventory"],
    daemon_error: null,
    daemon_version: "0.1.0",
    agent_error: null,
    gamemode_active: false,
    active_gpu: "0000:01:00.0",
  }),
  snapshot: async () => next(),
  monitorStart: async (intervalMs: number) => {
    if (timer) clearInterval(timer);
    timer = setInterval(() => {
      const s = next();
      listeners.forEach((f) => f(s));
    }, intervalMs);
  },
  monitorStop: async () => {
    if (timer) clearInterval(timer);
    timer = null;
  },
  gamemodeActive: async () => false,
  gamesRunning: async (): Promise<RunningGame[]> => [],
  gamesInstalled: async () => data.installed as InstalledGame[],
  profiles: async () => structuredClone(profiles),
  // The preview keeps changes in memory so the page can be tried out.
  setGameProfile: async (key: string, profile: string) => {
    for (const p of profiles) p.games = p.games.filter((g) => g !== key);
    if (profile) profiles.find((p) => p.id === profile)?.games.push(key);
  },
  setProfileOption: async (profile: string, option: string, value: boolean) => {
    const p = profiles.find((x) => x.id === profile) as unknown as Record<string, unknown> | undefined;
    if (p) p[option] = value;
  },
  playtime: async (): Promise<PlaytimeEntry[]> => [],
  // Power in the preview: nothing is ever done; the queue lives in memory.
  powerCaps: async () =>
    (["power-off", "reboot", "suspend", "hibernate", "hybrid-sleep", "suspend-then-hibernate", "reboot-firmware"] as PowerAction[]).map((action) => ({
      action,
      available: !action.includes("hibernate") && action !== "hybrid-sleep",
      reason: action.includes("hibernate") || action === "hybrid-sleep" ? "Hibernation is not set up on this system." : "",
    })),
  powerNow: async () => {},
  powerSchedule: async (action: PowerAction, at_unix_ms: number, repeat: string) => {
    const job: PowerJob = { id: jobs.length + 1, action, at_unix_ms, repeat: repeat as PowerJob["repeat"], note: "" };
    jobs.push(job);
    return job;
  },
  powerJobs: async () => [...jobs],
  powerWhen: async (action: PowerAction, kind: WatchKind, threshold: number, minutes: number, name: string) => {
    if (kind === "game-over") throw "no game is running right now";
    const w: PowerWatch = { id: watches.length + 1, action, kind, threshold, minutes, name, note: "" };
    watches.push(w);
    return w;
  },
  powerWatches: async () => [...watches],
  powerUnwatch: async (id: number) => {
    const i = watches.findIndex((w) => w.id === id);
    if (i >= 0) watches.splice(i, 1);
    return i >= 0;
  },
  // Invented for the preview only; the page labels preview data as such.
  powerUsage: async (): Promise<PowerUsage> => ({
    cpu_w: base.cpu.power_w,
    gpu_w: null,
    battery_w: null,
    cpu_energy_recorded: true,
    days: Array.from({ length: 7 }, (_, i) => {
      const d = new Date(Date.now() - (6 - i) * 864e5);
      return { date: d.toISOString().slice(0, 10), cpu_wh: [210, 340, 180, 420, 390, 260, 150][i] };
    }),
  }),
  battery: async () => ({ ...bat }),
  setBatteryCare: async (mode: string, limit: number) => {
    bat.mode = mode as BatteryCare["mode"];
    bat.limit = limit;
  },
  wakeAlarm: async (): Promise<[number, boolean]> => [alarm, true],
  setWakeAlarm: async (at: number) => {
    alarm = at;
  },
  clearWakeAlarm: async () => {
    alarm = 0;
  },
  powerCancel: async (id: number) => {
    const i = jobs.findIndex((j) => j.id === id);
    if (i >= 0) jobs.splice(i, 1);
    return i >= 0;
  },
  onSample: async (f: (s: Snapshot) => void) => {
    listeners.add(f);
    return () => listeners.delete(f);
  },
};
