// Browser preview only (vite dev without Tauri): replays readings captured
// from the development laptop, gently varied so the page moves. The UI labels
// itself as a preview whenever this is in use; nothing here reaches a build
// that runs inside Tauri.

import data from "./preview-data.json";
import type { AppTraffic, BatteryCare, DayTraffic, ExtraSample, FpsReading, NetRule, PingReport, HistoryPoint, InstalledGame, Overview, PlaytimeEntry, PowerAction, PowerJob, PowerUsage, PowerWatch, WatchKind, Profile, RunningGame, Snapshot } from "./api";

const base = data.snapshot as unknown as Snapshot;
const profiles = structuredClone(data.profiles) as Profile[];
const jobs: PowerJob[] = [];
let alarm = 0;
const bat: BatteryCare = { supported: true, method: "vendor-long-life", adjustable: false, mode: "off", limit: 80, capacity: 100, status: "Full", reason: "" };
const watches: PowerWatch[] = [];
let t = 0;
let fpsSetUp = false;
const netRules: NetRule[] = [];
const pingStart = Date.now();
function pingSeries(base: number, spread: number, spikes: number, from: number): { ts_unix_ms: number; rtt_ms: number | null }[] {
  const out = [];
  for (let ts = from - (from % 1000); ts <= Date.now(); ts += 1000) {
    const i = ts / 1000;
    const calm = ts > pingStart + 20_000; // the preview "turns shaping on" after 20 s
    const spike = !calm && Math.sin(i * 1.7) > 1 - spikes ? spread * 4 : 0;
    out.push({ ts_unix_ms: ts, rtt_ms: i % 97 === 0 ? null : base + Math.abs(Math.sin(i)) * (calm ? spread / 4 : spread) + spike });
  }
  return out;
}
function stats(s: { rtt_ms: number | null }[]) {
  const r = s.map((x) => x.rtt_ms).filter((v): v is number => v != null).sort((a, b) => a - b);
  const pick = (q: number) => r[Math.round((r.length - 1) * q)] ?? null;
  let j = 0;
  for (let k = 1; k < r.length; k++) j += Math.abs(r[k] - r[k - 1]);
  return { sent: s.length, received: r.length, median_ms: pick(0.5), jitter_ms: r.length > 1 ? j / (r.length - 1) / 3 : null, p95_ms: pick(0.95), loss_pct: s.length ? (100 * (s.length - r.length)) / s.length : 0 };
}
const mockApps = (): AppTraffic[] => {
  const w = () => 0.5 + Math.random();
  const rule = (n: string) => netRules.find((r) => r.app.toLowerCase() === n.toLowerCase());
  const mk = (name: string, kind: AppTraffic["kind"], category: string, rx: number, tx: number): AppTraffic => {
    const r = rule(name);
    const state = r ? (r.block ? "blocked" : "limited") : "";
    const cap = (v: number, k: number) => (r?.block ? 0 : k ? Math.min(v, (k * 1000) / 8) : v);
    return { id: `x/${name}`, name, kind, uid: 1000, exe: name, pids: [1], rx_bps: cap(rx * w(), r?.down_kbps ?? 0), tx_bps: cap(tx * w(), r?.up_kbps ?? 0), rx_bytes: rx * 900, tx_bytes: tx * 900, state, category, has_game: false };
  };
  return [
    mk("steam", "app", "launcher", 4_200_000, 40_000),
    mk("qbittorrent", "app", "torrent", 600_000, 350_000),
    mk("firefox-esr", "app", "", 120_000, 9_000),
    mk("Discord", "app", "", 30_000, 12_000),
    mk("packagekit", "service", "updates", 0, 0),
    mk("dropbox", "app", "sync", 2_000, 1_000),
  ].sort((a, b) => b.rx_bps + b.tx_bps - (a.rx_bps + a.tx_bps));
};

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
  // Synthetic curves for the preview; the page labels preview data.
  history: async (since: number, until: number, points: number): Promise<HistoryPoint[]> =>
    Array.from({ length: points }, (_, i) => {
      const ts = since + ((until - since) * i) / (points - 1);
      const w = Math.sin(i / 7) * 0.5 + 0.5;
      return {
        ts_unix_ms: ts,
        cpu_temp_c: 45 + w * 25,
        cpu_load_pct: 10 + w * 60,
        cpu_w: 15 + w * 45,
        gpu_temp_c: i % 40 < 12 ? 55 + w * 15 : null,
        gpu_load_pct: i % 40 < 12 ? 40 + w * 50 : null,
        gpu_w: i % 40 < 12 ? 20 + w * 40 : null,
        mem_used_kb: 18e6 + w * 4e6,
      };
    }),
  traffic: async (): Promise<DayTraffic[]> =>
    Array.from({ length: 7 }, (_, i) => ({
      date: new Date(Date.now() - (6 - i) * 864e5).toISOString().slice(0, 10),
      rx_bytes: [2.1, 5.4, 1.2, 8.8, 3.3, 0.9, 4.2][i] * 1e9,
      tx_bytes: [0.3, 0.6, 0.2, 1.1, 0.4, 0.1, 0.5][i] * 1e9,
    })),
  extra: async (): Promise<ExtraSample> => ({
    ts_unix_ms: Date.now(),
    per_core_mhz: base.cpu.per_core_load.map((_, i) => Math.round(1800 + Math.random() * 2200 + (i < 8 ? 400 : 0))),
    disk_read_bps: Math.random() < 0.7 ? Math.random() * 3e6 : 40e6 * Math.random(),
    disk_write_bps: Math.random() * 2e6,
    disk_busy_pct: Math.random() * 20,
    battery_pct: 100,
    battery_w: 0,
    battery_status: "Full",
    on_ac: true,
    load_avg_1: 1.2 + Math.random(),
    processes: 612,
  }),
  fps: async (): Promise<FpsReading> => {
    const now = Date.now();
    const recent: [number, number][] = Array.from({ length: 50 }, (_, i) => [now - (49 - i) * 200, 138 + Math.sin((now / 200 + i) / 4) * 8 - (i % 23 === 0 ? 50 : 0)]);
    return { state: fpsSetUp ? "live" : "not-set-up", game: "cs2", fps: 141, frametime_ms: 7.1, low_1pct_fps: 88, recent, install_hint: "" };
  },
  setUpFps: async (on: boolean) => {
    fpsSetUp = on;
  },
  pingRecent: async (since: number): Promise<PingReport> => {
    const from = Math.max(since, pingStart - 60_000);
    const gw = pingSeries(4, 3, 0.02, from);
    const net = pingSeries(38, 20, 0.08, from);
    return {
      targets: [
        { id: "gateway", label: "Router", addr: "192.168.1.1", samples: gw, stats: stats(gw.slice(-60)), problem: "" },
        { id: "internet", label: "DNS server", addr: "1.1.1.1", samples: net, stats: stats(net.slice(-60)), problem: "" },
      ],
      marks: [{ ts_unix_ms: pingStart + 20_000, kind: "gamemode-on" }].filter((m) => m.ts_unix_ms < Date.now()),
    };
  },
  pingCompare: async () => {
    const r = pingSeries(38, 20, 0.08, pingStart - 60_000);
    const cut = pingStart + 20_000;
    const b = stats(r.filter((x) => x.ts_unix_ms < cut));
    const a = stats(r.filter((x) => x.ts_unix_ms >= cut));
    return { before: b, after: a, verdict: a.received < 10 ? "not-enough-data" : "flatter" } as const;
  },
  netStatus: async () => ({ available: true, problem: "", simulated: false, firewalls: [], shaping_problems: [], iface: "wlp8s0" }),
  netApps: async () => mockApps(),
  netRules: async () => netRules.map((r) => ({ ...r })),
  netSetRule: async (app: string, block: boolean, up: number, down: number, when: "always" | "gaming") => {
    const i = netRules.findIndex((r) => r.app.toLowerCase() === app.toLowerCase());
    if (i >= 0) netRules.splice(i, 1);
    netRules.push({ app, block, up_kbps: up, down_kbps: down, when, active: when === "always" });
  },
  netRemoveRule: async (app: string) => {
    const i = netRules.findIndex((r) => r.app.toLowerCase() === app.toLowerCase());
    if (i >= 0) netRules.splice(i, 1);
    return i >= 0;
  },
  openTerminal: async () => {
    throw "The preview cannot open a terminal. In the app this opens ug tui.";
  },
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
