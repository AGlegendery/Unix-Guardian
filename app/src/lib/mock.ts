// Browser preview only (vite dev without Tauri): replays readings captured
// from the development laptop, gently varied so the page moves. The UI labels
// itself as a preview whenever this is in use; nothing here reaches a build
// that runs inside Tauri.

import data from "./preview-data.json";
import driverReport from "./preview-driver.json";
import type { AppUsage, TestlabProgress, TestlabReport, RecorderAction, RecorderSettings, RecorderState, AutoTuneStatus, CoolingState, GpuDefault, GpuTune, GpuTuneState, FpsSession, OverlaySettings, OverlayState, SettingsState, UpdateState, TrafficBucket, DriverReport, DriverPlan, DriverOp, ShortcutsState, DesktopKeys, AgentSettings, AppTraffic, BatteryCare, DayTraffic, ExtraSample, FpsReading, NetRule, PingReport, HistoryPoint, InstalledGame, Overview, PlaytimeEntry, PlaytimeSession, TrackedApp, LaunchInfo, PowerAction, PowerJob, PowerUsage, PowerWatch, WatchKind, Profile, RunningGame, Snapshot } from "./api";

const base = data.snapshot as unknown as Snapshot;
// The captured profiles predate the screen and notification settings.
const profiles: Profile[] = structuredClone(data.profiles).map((p) => ({ ...p, pause_apps: [] as string[], games: [] as string[], keep_screen_on: true, do_not_disturb: p.id === "competitive" }));
// `?profiles=N` adds N of the user's own profiles with long names, to try the
// profile picker with many.
for (let i = 0; i < Number(new URLSearchParams(location.search).get("profiles") ?? 0); i++)
  profiles.push({ ...structuredClone(profiles[1]), id: `my-long-profile-name-${i + 1}`, builtin: false });
const jobs: PowerJob[] = [];
let alarm = 0;
const bat: BatteryCare = { supported: true, method: "vendor-long-life", adjustable: false, mode: "off", limit: 80, capacity: 100, status: "Full", reason: "" };
const watches: PowerWatch[] = [];
let t = 0;
const driverOps: DriverOp[] = [];
// Game mode turned on by hand in the preview, and the profile it uses.
let hand: [string, string] = ["", "balanced"];
let agentSettings: AgentSettings = { notifications: true, scan_interval_s: 2, close_window_on_game: false, check_updates: true };
// Made-up hourly traffic for a local day: quiet nights, busy evenings, the
// same numbers every time for the same hour.
function mockHours(dayMs: number): TrafficBucket[] {
  const d = new Date(dayMs);
  d.setHours(0, 0, 0, 0);
  return Array.from({ length: 24 }, (_, h) => {
    const start = d.getTime() + h * 3_600_000;
    const seed = Math.abs(Math.sin(start / 3_600_000)) ;
    const busy = h < 7 ? 0.05 : h < 17 ? 0.4 : 1;
    const end = start > Date.now() ? 0 : 1;
    return { start_unix_ms: start, rx_bytes: Math.round(end * busy * (0.1 + seed) * 6e8), tx_bytes: Math.round(end * busy * (0.05 + seed / 4) * 1e8) };
  });
}
// Traffic before this was reset in the preview.
let trafficReset = 0;
const MOCK_APPS: [string, string, number, number][] = [
  ["steam", "launcher", 0.42, 0.02], ["firefox-esr", "browser", 0.25, 0.04], ["discord", "chat", 0.08, 0.05],
  ["telegram-desktop", "chat", 0.06, 0.03], ["qbittorrent", "download", 0.12, 0.3], ["flatpak", "updates", 0.05, 0.001],
  ["apt", "updates", 0.015, 0.0005], ["spotify", "media", 0.004, 0.0002],
];
// `?update=none` previews an up-to-date copy.
const updateNone = typeof location !== "undefined" && location.search.includes("update=none");
const update: UpdateState = {
  current: "0.1.0-beta.1",
  latest: updateNone ? "" : "v0.1.0-beta.2",
  name: updateNone ? "" : "Unix Guardian 0.1.0 beta 2",
  notes: updateNone ? "" : "## What's new\n\n- Updates install from the window, the terminal interface and `ug update --install`.\n- The driver troubleshooter knows more Wi-Fi cards.\n\n## Fixed\n\n- The Settings page no longer turns the daily update check back on.",
  url: "https://github.com/AGlegendery/Unix-Guardian/releases",
  published: "2026-10-04T10:00:00Z",
  kind: "tarball",
  how: "",
  can_install: !updateNone,
  checked_unix_ms: Date.now() - 3 * 3600_000,
  checking: false,
  step: "",
  result: "",
  previous: "",
  problem: "",
};
// `?recorder=missing` previews the recorder before Set up.
const recorderMissing = typeof location !== "undefined" && location.search.includes("recorder=missing");
const recorder: RecorderState = {
  engine: recorderMissing ? "" : "flatpak", can_set_up: recorderMissing, mode: "idle", since_unix_ms: 0, file: "", last_saved: "", problem: "", setup_pct: 0, setup_step: "",
  captures: ["portal", "DP-1", "eDP-1"],
  settings: { quality: "balanced", fps: 60, capture: "", codec: "auto", audio_output: true, audio_mic: false, replay_seconds: 60, folder: "" },
  mb_per_minute: 91, replay_mb: 91, folder: "/home/you/Videos/Unix Guardian",
};
const shortcuts: ShortcutsState = {
  enabled: false,
  status: "off",
  problem: "",
  gamemode_by_hand: "",
  shortcuts: [
    { id: "open-window", description: "Open Unix Guardian", trigger: "Meta+Shift+U", keys: "Meta+Shift+U", chosen: false },
    { id: "toggle-overlay", description: "Show or hide the in-game overlay", trigger: "Meta+Shift+O", keys: "Meta+Shift+O", chosen: false },
    { id: "toggle-gamemode", description: "Turn game mode on or off", trigger: "Meta+Shift+G", keys: "Meta+Shift+G", chosen: false },
    { id: "toggle-recording", description: "Start or stop recording", trigger: "Meta+Shift+R", keys: "Meta+Shift+R", chosen: false },
    { id: "save-replay", description: "Save the last seconds (replay)", trigger: "Meta+Shift+S", keys: "Meta+Shift+S", chosen: false },
  ],
};
const desktopKeys: DesktopKeys = { desktop: "", set_up: false, problem: "" };
let fpsSetUp = false;
let fanMode = 1;
let trayVisible = true;
let tracked: TrackedApp[] = [];
const launch = new Map<number, { own: string; gpu: string; overlay: boolean }>();
/** Roughly what the agent composes (Flatpak Steam: MANGOHUD=1). */
function composeMock(own: string, gpu: string, overlay: boolean): string {
  const env = [
    ...(gpu === "0000:01:00.0" ? ["__NV_PRIME_RENDER_OFFLOAD=1", "__GLX_VENDOR_LIBRARY_NAME=nvidia", "__VK_LAYER_NV_optimus=NVIDIA_only"] : []),
    ...(gpu === "0000:00:02.0" ? ["DRI_PRIME=pci-0000_00_02_0", "MESA_VK_DEVICE_SELECT=8086:468b!", "__GLX_VENDOR_LIBRARY_NAME=mesa"] : []),
    ...(overlay ? ["MANGOHUD=1"] : []),
  ];
  if (!env.length) return own;
  return [...env, own.includes("%command%") ? own : `%command%${own ? " " + own : ""}`].join(" ");
}
let ptImported: PlaytimeEntry[] = [];
let ptSessions: PlaytimeSession[] = (() => {
  const games: [string, string, number][] = [
    ["steam:730", "Counter-Strike 2", 95], ["steam:1245620", "Elden Ring", 70],
    ["steam:1091500", "Cyberpunk 2077", 45], ["lutris:osu", "osu!", 20],
    // Taken for a game by its behaviour, as on the dev laptop.
    ["exe:wondershare filmora", "Wondershare Filmora", 15],
  ];
  const out: PlaytimeSession[] = [];
  const now = Date.now();
  for (let d = 29; d >= 0; d--) {
    games.forEach(([key, name, mins], i) => {
      if ((d + i) % 3 === 2) return;
      const start = now - d * 86_400_000 - (i + 1) * 3 * 3_600_000;
      const len = mins * (0.5 + ((d * 7 + i * 3) % 10) / 10) * 60_000;
      out.push({ key, name, started_unix_ms: start, ended_unix_ms: Math.min(now - 60_000, start + len) });
    });
  }
  return out;
})();
let gpuDefault: GpuDefault = { chosen: "0000:00:02.0" };
let ppdProfile = "balanced";
const fanCurves: Record<string, [number, number][]> = {};
const zeroTune: GpuTune = { power_limit_w: 0, core_offset_mhz: 0, mem_offset_mhz: 0 };
let tuneKept: GpuTune = { ...zeroTune };
let tuneTrial: GpuTune | null = null;
let autoStatus: AutoTuneStatus = { running: false, step: 0, steps: 0, message: "", summary: "", ok: false };
let overlay: OverlaySettings = { show: "gaming", layout: "card", position: "top-left", size: "medium", opacity_pct: 50, toggle_key: "Alt_L+o" };
let fpsLog: FpsSession[] = [
  { game: "cs2", started_unix_ms: Date.now() - 3 * 3600e3, duration_s: 3120, frames: 440000, avg_fps: 141, low_1pct_fps: 96, low_01pct_fps: 71, p99_frametime_ms: 10.4, stutters: 4 },
  { game: "eldenring", started_unix_ms: Date.now() - 26 * 3600e3, duration_s: 5400, frames: 320000, avg_fps: 59.6, low_1pct_fps: 44, low_01pct_fps: 22, p99_frametime_ms: 22.7, stutters: 57 },
  { game: "hades", started_unix_ms: Date.now() - 4 * 86400e3, duration_s: 1800, frames: 250000, avg_fps: 138, low_1pct_fps: 131, low_01pct_fps: 120, p99_frametime_ms: 7.6, stutters: 0 },
];
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
// "Slow down background apps" in the preview.
let netBg: [boolean, number, number, "always" | "gaming"] = [false, 1000, 8000, "always"];
const mockApps = (): AppTraffic[] => {
  const w = () => 0.5 + Math.random();
  const rule = (n: string) => netRules.find((r) => r.app.toLowerCase() === n.toLowerCase());
  const mk = (name: string, kind: AppTraffic["kind"], category: string, rx: number, tx: number): AppTraffic => {
    const r = rule(name);
    const bg = netBg[0] && !r && category !== "vpn" && category !== "dns";
    const state = r ? (r.block ? "blocked" : "limited") : bg ? "background" : "";
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
    mk("v2rayn", "app", "vpn", 180_000, 40_000),
    mk("systemd-resolved", "service", "dns", 3_000, 1_000),
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
    gamemode_active: hand[0] === "on",
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
  gamemodeActive: async () => hand[0] === "on",
  gameModeHand: async (): Promise<[string, string]> => [...hand],
  setGameMode: async (on: boolean, profile: string) => {
    if (profile && !profiles.some((p) => p.id === profile)) throw `no profile named "${profile}"`;
    hand = [on ? "on" : "", profile || hand[1]];
  },
  gamesRunning: async (): Promise<RunningGame[]> => [],
  // The laptop has no Steam games; two made-up ones show the launch options.
  gamesInstalled: async () => [
    ...(data.installed as InstalledGame[]),
    { id: "steam:730", name: "Counter-Strike 2", launcher: "steam" },
    { id: "steam:1245620", name: "Elden Ring", launcher: "steam" },
  ],
  profiles: async () => structuredClone(profiles),
  // The preview keeps changes in memory so the page can be tried out.
  setGameProfile: async (key: string, profile: string) => {
    for (const p of profiles) p.games = p.games.filter((g) => g !== key);
    if (profile) profiles.find((p) => p.id === profile)?.games.push(key);
  },
  setPauseApps: async (profile: string, apps: string[]) => {
    const p = profiles.find((x) => x.id === profile);
    if (!p) throw `no profile named "${profile}"`;
    p.pause_apps = [...new Set(apps.map((a) => a.trim()).filter(Boolean))];
  },
  createProfile: async (profile: string, from: string) => {
    if (!/^[A-Za-z0-9_-]{1,40}$/.test(profile)) throw `"${profile}" is not a valid profile name (letters, digits, - and _ only)`;
    if (profiles.some((p) => p.id === profile)) throw `there is already a profile named "${profile}"`;
    const base = profiles.find((p) => p.id === (from || "balanced"))!;
    profiles.push({ ...structuredClone(base), id: profile, builtin: false, games: [] });
  },
  deleteProfile: async (profile: string) => {
    const i = profiles.findIndex((p) => p.id === profile && !p.builtin);
    if (i < 0) throw `${profile} is built in and cannot be deleted`;
    profiles.splice(i, 1);
    if (hand[1] === profile) hand = [hand[0], "balanced"];
  },
  setProfileOption: async (profile: string, option: string, value: boolean) => {
    const p = profiles.find((x) => x.id === profile) as unknown as Record<string, unknown> | undefined;
    if (p) p[option] = value;
  },
  // Made-up sessions over the last ten days, so the Playtime page has
  // something to show; summaries are computed from them like the agent does.
  playtime: async (since: number): Promise<PlaytimeEntry[]> => {
    const by = new Map<string, PlaytimeEntry>();
    for (const s of ptSessions) {
      if (s.ended_unix_ms <= since) continue;
      const secs = (s.ended_unix_ms - Math.max(s.started_unix_ms, since)) / 1000;
      const e = by.get(s.key) ?? { key: s.key, name: s.name, seconds: 0, sessions: 0, last_played_unix_ms: 0 };
      e.seconds += Math.round(secs);
      e.sessions += 1;
      e.last_played_unix_ms = Math.max(e.last_played_unix_ms, s.ended_unix_ms);
      by.set(s.key, e);
    }
    if (since === 0) {
      for (const i of ptImported) {
        const e = by.get(i.key);
        if (e) { e.seconds += i.seconds; e.last_played_unix_ms = Math.max(e.last_played_unix_ms, i.last_played_unix_ms); }
        else by.set(i.key, { ...i });
      }
    }
    return [...by.values()].sort((a, b) => b.seconds - a.seconds);
  },
  playtimeSessions: async (since: number): Promise<PlaytimeSession[]> => ptSessions.filter((s) => s.ended_unix_ms > since),
  playtimeForget: async (key: string) => {
    const n = ptSessions.filter((s) => s.key === key).length;
    ptSessions = ptSessions.filter((s) => s.key !== key);
    return n;
  },
  playtimeExport: async () => "~/Downloads/unix-guardian-playtime-preview.csv",
  trackedApps: async (): Promise<TrackedApp[]> => structuredClone(tracked),
  // Launch options in the preview: one made-up game with options of its own.
  launchOptions: async (appid: number): Promise<LaunchInfo> => {
    const l = launch.get(appid) ?? { own: appid === 730 ? "-novid" : "", gpu: "", overlay: false };
    return { appid, name: "", current: composeMock(l.own, l.gpu, l.overlay), own: l.own, gpu: l.gpu, overlay: l.overlay, changed_outside: false, flatpak: true, steam_running: false, overlay_problem: "" };
  },
  previewLaunchOptions: async (appid: number, gpu: string, overlay: boolean) => composeMock((launch.get(appid)?.own) ?? (appid === 730 ? "-novid" : ""), gpu, overlay),
  setLaunchOptions: async (appid: number, gpu: string, overlay: boolean): Promise<[string, string]> => {
    const own = launch.get(appid)?.own ?? (appid === 730 ? "-novid" : "");
    launch.set(appid, { own, gpu, overlay });
    return [composeMock(own, gpu, overlay), "~/.local/share/unix-guardian/backups/localconfig-42-preview.vdf"];
  },
  undoLaunchOptions: async (appid: number): Promise<[string, string]> => {
    const own = launch.get(appid)?.own ?? "";
    launch.delete(appid);
    return [own, "~/.local/share/unix-guardian/backups/localconfig-42-preview.vdf"];
  },
  trackApp: async (exe: string, name: string) => {
    const e = exe.trim().toLowerCase().replace(/\.exe$/, "");
    if (!e) throw "give a program's executable name, e.g. firefox or Filmora.exe";
    tracked = [...tracked.filter((a) => a.exe !== e), { exe: e, name: name.trim() || e }];
  },
  untrackApp: async (exe: string) => {
    const before = tracked.length;
    tracked = tracked.filter((a) => a.exe !== exe);
    return tracked.length !== before;
  },
  runningPrograms: async (): Promise<TrackedApp[]> =>
    [
      { exe: "firefox-esr", name: "Firefox ESR" },
      { exe: "chrome", name: "Google Chrome" },
      { exe: "konsole", name: "Konsole" },
      { exe: "telegram", name: "Telegram" },
      { exe: "wondershare filmora", name: "Wondershare Filmora" },
    ].filter((p) => !tracked.some((a) => a.exe === p.exe)),
  moveToApps: async (key: string) => {
    const exe = key.replace(/^exe:/, "");
    let n = 0;
    for (const s of ptSessions) if (s.key === key) { s.key = `app:${exe}`; n++; }
    const name = ptSessions.find((s) => s.key === `app:${exe}`)?.name ?? exe;
    tracked = [...tracked.filter((a) => a.exe !== exe), { exe, name }];
    return n;
  },
  importSteam: async (): Promise<[number, number]> => {
    ptImported = [{ key: "steam:730", name: "Counter-Strike 2", seconds: 412 * 3600, sessions: 0, last_played_unix_ms: Date.now() - 3 * 86_400_000 },
      { key: "steam:570", name: "Dota 2", seconds: 96 * 3600, sessions: 0, last_played_unix_ms: Date.now() - 200 * 86_400_000 }];
    return [2, ptImported.reduce((a, e) => a + e.seconds, 0)];
  },
  playtimeImported: async () => structuredClone(ptImported),
  forgetImported: async () => {
    const n = ptImported.length;
    ptImported = [];
    return n;
  },
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
        cpu_mhz: 2400 + w * 2000,
        gpu_mhz: i % 40 < 12 ? 900 + w * 900 : null,
        swap_used_kb: 2e5 + w * 1e5,
        disk_read_bps: (0.5 + w * 4) * 1e6,
        disk_write_bps: (0.2 + w) * 1e6,
        disk_busy_pct: 2 + w * 15,
        net_rx_bps: (0.05 + w * w * 2) * 1e6,
        net_tx_bps: (0.01 + w * 0.1) * 1e6,
        load_avg_1: 0.5 + w * 3,
        processes: 380 + w * 40,
        battery_pct: null,
        battery_w: null,
      };
    }),
  traffic: async (sinceUnixMs: number): Promise<DayTraffic[]> => {
    sinceUnixMs = Math.max(sinceUnixMs, trafficReset);
    const n = Math.min(90, Math.max(1, Math.ceil((Date.now() - sinceUnixMs) / 864e5)));
    return Array.from({ length: n }, (_, i) => {
      const day = new Date(Date.now() - (n - 1 - i) * 864e5);
      const hours = mockHours(day.getTime());
      return {
        date: `${day.getFullYear()}-${String(day.getMonth() + 1).padStart(2, "0")}-${String(day.getDate()).padStart(2, "0")}`,
        rx_bytes: hours.reduce((a, h) => a + h.rx_bytes, 0),
        tx_bytes: hours.reduce((a, h) => a + h.tx_bytes, 0),
      };
    });
  },
  trafficBuckets: async (since: number, until: number, bucket: number): Promise<TrafficBucket[]> => {
    const out: TrafficBucket[] = [];
    for (let t = since; t < until && out.length < 2000; t += bucket) {
      let rx = 0, tx = 0;
      for (const h of mockHours(t)) {
        if (h.start_unix_ms + 3_600_000 > t && h.start_unix_ms < t + bucket && h.start_unix_ms <= Date.now()) {
          const share = Math.min(1, bucket / 3_600_000);
          rx += h.rx_bytes * share;
          tx += h.tx_bytes * share;
        }
      }
      if (t < trafficReset) rx = tx = 0;
      out.push({ start_unix_ms: t, rx_bytes: Math.round(rx), tx_bytes: Math.round(tx) });
    }
    return out;
  },
  appTraffic: async (since: number, until: number): Promise<AppUsage[]> => {
    let rx = 0, tx = 0;
    for (const b of await mockApi.trafficBuckets(since, Math.min(until, Date.now()), 3_600_000)) {
      rx += b.rx_bytes;
      tx += b.tx_bytes;
    }
    return MOCK_APPS.map(([name, category, r, w]) => ({ name, category, rx_bytes: Math.round(rx * r), tx_bytes: Math.round(tx * w * 4) }))
      .filter((a) => a.rx_bytes + a.tx_bytes > 0)
      .sort((a, b) => b.rx_bytes + b.tx_bytes - (a.rx_bytes + a.tx_bytes));
  },
  resetTraffic: async () => {
    trafficReset = Date.now();
    return true;
  },
  trafficResetAt: async () => trafficReset,
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
    return { state: fpsSetUp ? "live" : "not-set-up", game: "cs2", fps: 141, frametime_ms: 7.1, low_1pct_fps: 88, recent, install_hint: "", mangohud_version: "0.8.1", live_unsupported: false };
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
        { id: "gateway", label: "Router", addr: "192.168.1.1", samples: gw, stats: stats(gw.slice(-60)), problem: "", via: "wlp8s0", through_vpn: false },
        { id: "internet", label: "DNS server", addr: "1.1.1.1", samples: net, stats: stats(net.slice(-60)), problem: "", via: "wlp8s0", through_vpn: false },
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
  netStatus: async () => ({ available: true, problem: "", simulated: false, firewalls: [], shaping_problems: [], iface: "wlp8s0", vpn: "v2rayn", vpn_tun: false }),
  netApps: async () => mockApps(),
  netRules: async () => netRules.map((r) => ({ ...r })),
  netSetRule: async (app: string, block: boolean, up: number, down: number, when: "always" | "gaming") => {
    const i = netRules.findIndex((r) => r.app.toLowerCase() === app.toLowerCase());
    if (i >= 0) netRules.splice(i, 1);
    netRules.push({ app, block, up_kbps: up, down_kbps: down, when, active: when === "always" });
  },
  netBackground: async () => [...netBg] as typeof netBg,
  netSetBackground: async (on: boolean, up: number, down: number, when: "always" | "gaming") => {
    if (on && !up && !down) throw "slowing background apps needs an upload or download cap";
    netBg = [on, up, down, when];
  },
  netRemoveRule: async (app: string) => {
    const i = netRules.findIndex((r) => r.app.toLowerCase() === app.toLowerCase());
    if (i >= 0) netRules.splice(i, 1);
    return i >= 0;
  },
  reportBug: async () => {
    throw "The preview cannot open a mail program. Write to unixguardian@atomicmail.io.";
  },
  overlayState: async (): Promise<OverlayState> => ({ settings: { ...overlay }, installed: true, set_up: fpsSetUp, install_hint: "", game_running: false }),
  overlaySet: async (s: OverlaySettings) => {
    overlay = { ...s };
  },
  overlaySetUp: async (on: boolean) => {
    fpsSetUp = on;
  },
  driverDiagnose: async (): Promise<DriverReport> => structuredClone(driverReport) as DriverReport,
  driverPlan: async (fixId: string): Promise<DriverPlan> => {
    const files: Record<string, [string, string]> = {
      "nvidia-suspend": ["/etc/modprobe.d/unix-guardian-nvidia-power.conf", "options nvidia NVreg_PreserveVideoMemoryAllocations=1 NVreg_TemporaryFilePath=/var/tmp"],
      "nvidia-drm-modeset": ["/etc/modprobe.d/unix-guardian-nvidia-drm.conf", "options nvidia-drm modeset=1 fbdev=1"],
      "nouveau-blacklist": ["/etc/modprobe.d/unix-guardian-blacklist-nouveau.conf", "blacklist nouveau\noptions nouveau modeset=0"],
    };
    const f = files[fixId];
    if (fixId === "rfkill-unblock-wifi")
      return { fix_id: fixId, title: "Switch Wi-Fi on", steps: [{ kind: "write-switch", target: "/sys/class/rfkill/rfkill0/soft", detail: "Writes 0 (takes effect at once).", undo: "The value it had is written back." }], needs_reboot: false, refused: "", snapshot: "" };
    if (!f) return { fix_id: fixId, title: fixId, steps: [], needs_reboot: false, refused: "Unix Guardian does not run this fix itself; its commands are shown to copy.", snapshot: "" };
    const steps: DriverPlan["steps"] = [{ kind: "write-file", target: f[0], detail: "# Written by Unix Guardian's driver troubleshooter.\n" + f[1] + "\n", undo: "The previous file is put back (or this one removed if there was none)." }];
    if (fixId === "nvidia-suspend")
      steps.push({ kind: "enable-units", target: "systemctl enable nvidia-suspend.service nvidia-resume.service nvidia-hibernate.service", detail: "Starts these services at sleep, wake and hibernate.", undo: "systemctl disable (only those that were off before)" });
    steps.push({ kind: "run", target: "update-initramfs -u -k all", detail: "Rebuilds the initramfs so the setting also applies early in boot. Takes up to a minute.", undo: "Run again after the files are put back." });
    return { fix_id: fixId, title: fixId, steps, needs_reboot: true, refused: "", snapshot: "" };
  },
  driverApply: async (fixId: string) => {
    const op: DriverOp = { id: `d${Date.now()}`, fix_id: fixId, title: fixId, started_unix_ms: Date.now(), state: "running", log: ["Started (preview: nothing is changed)"], needs_reboot: true, verify: "" };
    driverOps.unshift(op);
    setTimeout(() => op.log.push("Wrote the file (preview)"), 600);
    setTimeout(() => { op.log.push("Initramfs rebuilt (preview)", "Done. Restart for it to take effect."); op.state = "done"; op.verify = "restart"; }, 1800);
    return op.id;
  },
  driverOps: async () => structuredClone(driverOps),
  driverKeep: async (opId: string) => {
    const op = driverOps.find((o) => o.id === opId);
    if (op) op.verify = "kept";
  },
  driverRollback: async (opId: string) => {
    const op = driverOps.find((o) => o.id === opId || (!opId && o.state === "done"));
    if (!op) throw "There is no driver change to undo.";
    op.log.push("Removed the file (preview)");
    op.state = "undone";
  },
  updateState: async (): Promise<UpdateState> => ({ ...update }),
  updateCheck: async (): Promise<UpdateState> => {
    await new Promise((r) => setTimeout(r, 900));
    update.checked_unix_ms = Date.now();
    return { ...update };
  },
  updateInstall: async () => {
    if (!update.can_install) throw "There is nothing newer to install.";
    update.step = "downloading";
    setTimeout(() => (update.step = "installing"), 2500);
    setTimeout(() => {
      update.step = "";
      update.previous = update.current;
      update.current = update.latest.replace(/^v/, "");
      update.result = `Installed ${update.latest} (preview).`;
      update.latest = "";
      update.can_install = false;
    }, 5000);
  },
  updateRollback: async () => {
    if (!update.previous) throw "There is no earlier version to go back to.";
    const back = update.previous;
    update.previous = "";
    update.current = back;
    return `Back to ${back} (preview).`;
  },
  settingsState: async (): Promise<SettingsState> => ({
    settings: { ...agentSettings },
    agent_version: "0.1.0",
    config_dir: "~/.config/unix-guardian",
    data_dir: "~/.local/share/unix-guardian",
    scan_interval_fixed: false,
  }),
  settingsSet: async (s: AgentSettings) => {
    if (s.scan_interval_s < 1 || s.scan_interval_s > 10) throw `scan interval must be 1 to 10 seconds, not ${s.scan_interval_s}`;
    agentSettings = { ...s };
  },
  shortcutsState: async (): Promise<ShortcutsState> => ({ ...shortcuts, shortcuts: shortcuts.shortcuts.map((k) => ({ ...k, trigger: shortcuts.enabled ? k.trigger : "" })) }),
  shortcutsSetEnabled: async (on: boolean) => {
    shortcuts.enabled = on;
    shortcuts.status = on ? "active" : "off";
  },
  shortcutsDesktopKeys: async (): Promise<DesktopKeys> => ({ ...desktopKeys }),
  shortcutsSetUpDesktopKeys: async (on: boolean) => {
    desktopKeys.set_up = on;
    return on ? `5 shortcuts written into ${desktopKeys.desktop}'s keyboard settings.` : `Unix Guardian's shortcuts removed from ${desktopKeys.desktop}'s keyboard settings.`;
  },
  shortcutsSetKeys: async (id: string, keys: string) => {
    const k = shortcuts.shortcuts.find((s) => s.id === id);
    if (!k) throw `Unknown shortcut \`${id}\``;
    const suggested = { "open-window": "Meta+Shift+U", "toggle-overlay": "Meta+Shift+O", "toggle-gamemode": "Meta+Shift+G", "toggle-recording": "Meta+Shift+R", "save-replay": "Meta+Shift+S" }[k.id];
    const want = keys || suggested;
    if (!/(Ctrl|Alt|Meta)\+/.test(want) && !/^(Shift\+)?F\d+$/.test(want)) throw `${want} needs Ctrl, Alt or Meta with it, or it would take the key from everything you type.`;
    if (shortcuts.shortcuts.some((s) => s.id !== id && s.keys === want)) throw `${want} is already the shortcut for "${shortcuts.shortcuts.find((s) => s.keys === want)?.description}".`;
    Object.assign(k, { keys: want, trigger: want, chosen: keys !== "" });
    return shortcuts.enabled ? `${want} now does "${k.description}".` : `Saved: ${want}, used when the shortcuts are turned on.`;
  },
  recorderState: async (): Promise<RecorderState> => {
    const s = recorder.settings;
    const mbit = s.quality === "small" ? 6 : s.quality === "high" ? 24 : 12;
    const mb = Math.round(((mbit * (s.fps / 60) + (Number(s.audio_output) + Number(s.audio_mic)) * 0.128) * 60) / 8 * 10) / 10;
    return { ...recorder, settings: { ...s }, mb_per_minute: mb, replay_mb: Math.round((mb * s.replay_seconds) / 6) / 10 };
  },
  recorderSet: async (s: RecorderSettings) => {
    if (![30, 60, 120].includes(s.fps)) throw "frame rate is 30, 60 or 120";
    recorder.settings = { ...s };
  },
  recorderDo: async (what: RecorderAction) => {
    const file = `${recorder.folder}/Recording 2026-10-03 21-00-00.mp4`;
    if (what === "start") Object.assign(recorder, { mode: "recording", since_unix_ms: Date.now(), file });
    if (what === "replay") Object.assign(recorder, { mode: "replay", since_unix_ms: Date.now() });
    if (what === "stop") {
      const was = recorder.mode;
      Object.assign(recorder, { mode: "idle", since_unix_ms: 0, file: "", last_saved: was === "recording" ? file : recorder.last_saved });
      return was === "recording" ? file : "";
    }
    if (what === "save") return (recorder.last_saved = `${recorder.folder}/Replay 2026-10-03 21-01-00.mp4`);
    if (what === "remove") Object.assign(recorder, { engine: "", can_set_up: true });
    if (what === "setup") {
      // Seven steps, as Flatpak reports them, a few seconds in all.
      Object.assign(recorder, { mode: "installing", setup_pct: 0, setup_step: "" });
      let i = 0;
      const id = setInterval(() => {
        i += 1;
        if (i > 70) {
          clearInterval(id);
          Object.assign(recorder, { mode: "idle", engine: "flatpak", can_set_up: false, setup_pct: 0, setup_step: "" });
          return;
        }
        Object.assign(recorder, { setup_pct: Math.round((i * 100) / 70), setup_step: `${Math.min(7, Math.floor(i / 10) + 1)}/7` });
      }, 120);
    }
    return what === "start" ? file : "";
  },
  shortcutsRun: async (id: string) => {
    if (id === "toggle-gamemode") {
      shortcuts.gamemode_by_hand = shortcuts.gamemode_by_hand === "on" ? "" : "on";
      return shortcuts.gamemode_by_hand === "on" ? "Game mode on." : "Game mode off.";
    }
    return "Done.";
  },
  fpsSessions: async (since: number) => fpsLog.filter((s) => s.started_unix_ms >= since),
  forgetFpsSessions: async () => {
    const n = fpsLog.length;
    fpsLog = [];
    return n;
  },
  gpuDefault: async () => ({ ...gpuDefault }),
  setGpuDefault: async (pci: string) => {
    gpuDefault = { chosen: pci };
    return { ...gpuDefault };
  },
  gpuOffload: async () => [
    { pci: "0000:00:02.0", vendor: "intel", boot_vga: true, launch_options: "DRI_PRIME=pci-0000_00_02_0 MESA_VK_DEVICE_SELECT=8086:468b! __GLX_VENDOR_LIBRARY_NAME=mesa %command%" },
    { pci: "0000:01:00.0", vendor: "nvidia", boot_vga: false, launch_options: "__NV_PRIME_RENDER_OFFLOAD=1 __GLX_VENDOR_LIBRARY_NAME=nvidia __VK_LAYER_NV_optimus=NVIDIA_only %command%" },
  ],
  gpuTuneState: async (pci: string): Promise<GpuTuneState> => {
    const t = tuneTrial ?? tuneKept;
    return {
      pci, power_limit: true, power_limit_w: t.power_limit_w || 60, power_default_w: 60, power_min_w: 35, power_max_w: 95,
      core_offset: true, core_offset_mhz: t.core_offset_mhz, core_min_mhz: -200, core_max_mhz: 300,
      mem_offset: true, mem_offset_mhz: t.mem_offset_mhz, mem_min_mhz: -500, mem_max_mhz: 1000,
      notes: ["NVIDIA's Linux driver has no voltage curve: a lower power limit with a higher clock offset is how to get cooler and quieter at the same speed."],
      applied: { ...t }, crashed: { ...zeroTune },
    };
  },
  gpuTryTune: async (_pci: string, t: GpuTune) => {
    tuneTrial = { ...t };
  },
  gpuKeepTune: async () => {
    if (tuneTrial) tuneKept = tuneTrial;
    tuneTrial = null;
  },
  gpuAutoTune: async (_pci: string, goal: "faster" | "cooler") => {
    const steps = 5;
    autoStatus = { running: true, step: 1, steps, message: "Measuring the card as it is…", summary: "", ok: false };
    let s = 1;
    const id = setInterval(() => {
      s += 1;
      if (s > steps) {
        clearInterval(id);
        tuneTrial = goal === "faster" ? { ...zeroTune, core_offset_mhz: 120 } : { ...zeroTune, power_limit_w: 49 };
        autoStatus = { running: false, step: steps, steps, message: "Done. Keep the result within two minutes, or it is undone.", ok: true,
          summary: goal === "faster" ? "Stable at +120 MHz. Under the same load: +2 °C, +1 W, +118 MHz clock." : "49 W keeps the same speed. Under the same load: -6 °C, -11 W, -3 MHz clock." };
      } else {
        autoStatus = { ...autoStatus, step: s, message: goal === "faster" ? `Testing +${30 * (s - 1)} MHz…` : `Testing ${60 - 3 * s} W…` };
      }
    }, 900);
  },
  gpuAutoTuneStatus: async () => ({ ...autoStatus }),
  gpuStopAutoTune: async () => {},
  gpuTestTune: async (): Promise<[boolean, string]> => {
    await new Promise((r) => setTimeout(r, 2000));
    return [true, "Stable: 312 rounds, 159744 results checked, none wrong."];
  },
  gpuCancelTune: async () => {
    tuneTrial = null;
  },
  gpuResetTune: async () => {
    tuneTrial = null;
    tuneKept = { ...zeroTune };
  },
  coolingState: async (): Promise<CoolingState> => ({
    fans: [{ id: "amdgpu/pwm1", label: "Graphics card fan", rpm: 1450, pct: 40, temp_c: 64, mode: fanCurves["amdgpu/pwm1"] ? "curve" : "auto", curve: fanCurves["amdgpu/pwm1"] ?? [] }, { id: "dell_smm/pwm1", label: "dell_smm fan 1", rpm: 2900, pct: 50, temp_c: 58, mode: "firmware", curve: [] }],
    fan_mode: fanMode,
    fan_modes: [[0, "quiet"], [1, "standard"], [4, "cooling"]],
    notes: [],
    simulated: false,
  }),
  setFanCurve: async (id: string, points: [number, number][]) => {
    fanCurves[id] = points;
  },
  setFanAuto: async (id: string) => {
    delete fanCurves[id];
  },
  setFanMode: async (m: number) => {
    fanMode = m;
  },
  powerProfile: async () => ppdProfile,
  setPowerProfile: async (p: string) => {
    ppdProfile = p;
  },
  trayVisible: async () => trayVisible,
  trayShow: async () => {
    trayVisible = true;
  },
  trayHide: async () => {
    trayVisible = false;
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
  // The test lab: walks the real step ids, a step every 250 ms.
  testlabRun: async (): Promise<TestlabReport> => {
    const ids = ["recover", "env", "services", "sensors", "speed", "gpu", "cpu-bench", "gamemode", "profiles", "games", "playtime", "history", "ping", "power", "fans", "net", "overlay", "recorder", "misc", "drivers", "cleanup"];
    for (let i = 0; i < ids.length; i++) {
      labListeners.forEach((f) => f({ done: i, total: ids.length, id: ids[i], label: ids[i] }));
      await new Promise((r) => setTimeout(r, 250));
    }
    labListeners.forEach((f) => f({ done: ids.length, total: ids.length, id: "done", label: "Done" }));
    return { text: sampleReport, passed: 31, warned: 3, failed: 0, skipped: 2, leftovers: [], checks: [] };
  },
  testlabRecover: async (): Promise<string[]> => [],
  testlabSave: async (_text: string) => "/home/you/Downloads/unix-guardian-test-report-2026-10-04-15-40.txt",
  onTestlabProgress: async (f: (p: TestlabProgress) => void) => {
    labListeners.add(f);
    return () => labListeners.delete(f);
  },
};

const labListeners = new Set<(p: TestlabProgress) => void>();
const sampleReport = `Unix Guardian test lab report
=============================
Made 2026-10-04 15:40 UTC, took 41 s. Report format 1.
Personal details were removed (home folder, user and host names, disk IDs,
MAC and IP addresses). Please read it before sending.

RESULT: 31 passed, 3 warnings, 0 failed, 2 skipped

THIS COMPUTER
  App           Unix Guardian 0.1.0-beta.1 (window)
  Distribution  Parrot Security 7.3 (echo) 7.3 (id parrot, Apt, tier One)
  Kernel        7.0.13+parrot7-amd64
  Desktop       KDE on wayland
  CPU           12th Gen Intel(R) Core(TM) i5-12450HX — 12 threads, 8 cores
  GPU 2         NVIDIA GeForce RTX 3050 6GB Laptop GPU at 0000:01:00.0, driver nvidia 550.163.01

CHECKS
  [PASS] service: reachable — version 0.1.0-beta.1, protocol 1, 4 ms
  [PASS] sensors: CPU temperature — 52.0 °C
  [WARN] sensors: GPU 0000:00:02.0 — reads: nothing
  [PASS] game mode: turn on — applied: CPU set to performance on 12 cores
  [PASS] game mode: CPU settings back as they were — governor powersave
  [SKIP] recorder: gpu-screen-recorder — not set up

MEASUREMENTS
  CPU one thread   182 M rounds/s
  CPU all threads  1032 M rounds/s on 12 threads (5.7× one thread)

NOT TESTED AUTOMATICALLY (needs a manual test)
  - GPU tuning (power limit, clock offsets): asks for the administrator password

PUT BACK
  Everything the test turned on was turned off again.

Send this report to unixguardian@atomicmail.io
`;
