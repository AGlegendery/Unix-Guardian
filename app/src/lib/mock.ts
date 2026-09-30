// Browser preview only (vite dev without Tauri): replays readings captured
// from the development laptop, gently varied so the page moves. The UI labels
// itself as a preview whenever this is in use; nothing here reaches a build
// that runs inside Tauri.

import data from "./preview-data.json";
import type { InstalledGame, Overview, PlaytimeEntry, PowerAction, PowerJob, Profile, RunningGame, Snapshot } from "./api";

const base = data.snapshot as unknown as Snapshot;
const profiles = structuredClone(data.profiles) as Profile[];
const jobs: PowerJob[] = [];
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
    (["power-off", "reboot", "suspend", "hibernate", "hybrid-sleep", "suspend-then-hibernate"] as PowerAction[]).map((action) => ({
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
