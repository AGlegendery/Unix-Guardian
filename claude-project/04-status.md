# 04 — Status (as of 2026-09-30)

## Where the code is

- Repository: `/home/aglegend/SSDBkup/IMPORTANT/ProgramingTable/unix-guardian`
  (on the user's laptop). Remote `origin` = the user's **private** repo
  https://github.com/AGlegendery/Unix-Guardian. **Nothing pushed yet**: the
  laptop has no GitHub credentials (no `gh`, no SSH key, no token), so the user
  pushes, or explicitly asks for a deploy key. CI has never run.
- Branches: `main` (renamed from `master` to match the CI trigger) now has P2
  merged. `p2-game-mode` is kept until pushed.

| Commit | Branch | What |
|---|---|---|
| `d0ac949` | main | Foundation: daemon, sensors, GPU read path, CLI (P0 + P1) |
| `e4aa5e9` | p2-game-mode | Game library scanning and running-game detection (P2.1–2.3) |
| `278cb62` | p2-game-mode | Per-user agent: detection loop, profiles, playtime, pinning, pausing |
| `2fbfa46` | p2-game-mode | Game mode end to end: reversible tuning in the daemon, reconciled by the agent |
| `9eccd01` | p2-game-mode | This knowledge pack (`claude-project/`, `CLAUDE.md`), `scripts/agent-e2e.sh`, `scripts/gamemode-e2e.sh` (also run by CI) |
| `2fb83ee` | p2-game-mode | Single-instance fix for daemon and agent; agent user unit + D-Bus activation + autostart, installed by `dev-install.sh`; `ug profile list/set/reset`; `ug playtime --csv`; keys in `ug game list` |

- **182 tests pass** (`cargo test --workspace`): ug-hw 41, ug-games 39, ug-gpu
  35, ug-agent 19, ug-core 20, ug-daemon 12, ug-tune 12, ug-cli 3, doc-test 1.
  `scripts/smoke.sh` (daemon + CLI over a private D-Bus bus) passes.
- `dbus-run-session -- ./scripts/agent-e2e.sh` (18 checks) and `./scripts/gamemode-e2e.sh` (13 checks, incl. "no polkit prompt / no real unit touched" and "a second daemon refuses to start mid-game") pass on the laptop.
- ~10,600 lines of Rust in 10 crates.
- **Not run locally:** `cargo fmt --check`, `cargo clippy -D warnings` (not
  installed on the dev machine — Debian's Rust 1.94 has no rustup), `cargo deny`.
  Expect CI to flag some style issues on the first push.

## How things were verified

| Verified on the real laptop | Verified only against fixtures/scratch | Never exercised |
|---|---|---|
| **Real install (2026-09-30):** daemon as root on the system bus (6.2 MB, RAPL CPU watts visible, sleeping dGPU not woken); agent under `systemd --user` (5.4 MB); fake CS2 → game mode in ~1 s with **no polkit prompt** (the agent in `user@.service` counts as the active session): 12 policies performance/performance, swappiness 10, max_map_count raised, compaction 0, 7 timers held, game nice −10 + autogroup −10; game exit → every value and timer **identical to the pre-install snapshot**; agent SIGKILL mid-game → daemon reverted at once, systemd restarted the agent, game mode back in ~2 s, then exact restore | | |
| sensors, CPU topology, RAPL absence, network (live) | AMD GPU read path (synthetic RDNA1–3 layouts) | **daemon SIGKILL mid-game** as root (recovery from `/var/lib/unix-guardian/gamemode.json`; needs sudo) |
| NVML telemetry, GPU names, capabilities | Steam / Lutris / Heroic library parsing (format samples) | a polkit **prompt** for an `auth_admin` action (`SetLogFilter`) |
| dGPU runtime-PM skip (0 ms added active time) | game-mode tuning writes (scratch copy of the laptop's `/sys`) | a **real game** (no Steam/Lutris/Heroic installed; a fake `SteamAppId` process was used) |
| NVML lazy load / 30 s release (6.2 → 21.7 → 6.4 MB) | crash recovery of the tuner (unit tests) | desktop **notifications** (tests used `--quiet`/private bus) |
| subscription release on SIGTERM/SIGKILL | | XDG autostart fallback (session without a systemd user manager) |
| detection on the live process table (518 procs, no false positives, 1.1 ms/scan) | | CI on GitHub, fmt/clippy/deny |
| Bottles library (live), agent end-to-end on a private bus: detection → P-core pinning → app pause/resume → GameMode registration → playtime | | the Tauri UI (not started) |
| game mode agent→daemon end-to-end on a private bus: enter → exact restore on exit; agent SIGKILL → daemon reverts; 0 polkit prompts / 0 real units touched | | |

## Roadmap task status

Legend: ✅ done · 🟨 partial · ⛔ blocked · ⬜ not started. Task texts are in
10-roadmap.fa.md; deviations are explained in 02-decisions.md §C.

### P0 — setup
| Task | Status | Notes |
|---|---|---|
| 0.1 repo/workspace | 🟨 | Workspace, toolchain/fmt/clippy/deny configs, GPL-3 LICENSE, README. Missing: CONTRIBUTING, CODE_OF_CONDUCT, SECURITY.md, issue templates, git-cliff CHANGELOG; README is English-only. |
| 0.2 CI | 🟨 | `.github/workflows/ci.yml` written (fmt, clippy, cargo-deny, tests on Arch/Fedora/Debian trixie/Ubuntu 24.04, smoke test). Never run: no remote. No cargo-audit, no front-end jobs. |
| 0.3 local VMs | ⬜ | |
| 0.4 ug-proto | ✅ | Introspection XML not yet exported to `docs/dbus/`. |
| 0.5 ug-testfs | ✅ | Plus a real fixture (`dev-laptop-rtx3050`) and `scripts/capture-fixture.sh`. |
| 0.6 Tauri skeleton | ⛔ | Needs `sudo apt install libwebkit2gtk-4.1-dev libsoup-3.0-dev`. `pnpm` is broken on the machine; `npm` works. |
| 0.7 logging | ✅ | Filter reloadable over D-Bus (`SetLogFilter`). |

### P1 — daemon/agent skeleton and monitoring
| Task | Status | Notes |
|---|---|---|
| 1.1 ug-daemon | ✅* | *Verified on the session bus only. Unit/D-Bus/polkit files validated (`xmllint`, `systemd-analyze verify`). Real install path: `scripts/dev-install.sh` (untested). |
| 1.2 ug-agent | ✅* | `packaging/systemd/ug-agent.service` (user unit, `graphical-session.target`), D-Bus activation file, XDG autostart fallback (`X-systemd-skip=true`); a second agent exits 0 and leaves the first in charge. `dev-install.sh` installs and globally enables it (install/uninstall checked in a Debian container with a stubbed `systemctl`). *Not yet run under a real `systemd --user`. |
| 1.3 ug-hw | ✅ | Plus Intel hybrid P/E cores. |
| 1.4 ug-gpu (read) | ✅ NVIDIA/Intel · 🟨 AMD | Plus runtime-PM skip and lazy NVML. AMD not run on hardware. |
| 1.5 monitor service | 🟨 | Daemon side done (subscribe/park/`Sample`). Agent-side ring buffer and per-minute SQLite history not done. |
| 1.6 network counter | ✅ | Via `/proc/net/route`. |
| 1.7 UI shell/monitoring/settings | ⛔ | Tauri. |
| 1.8 ug-cli | ✅ | `status/inventory/monitor/gpu/game/playtime/profile`. `ug profile list/set/reset` go through the new `Agent1.Profiles` interface; `ug game list` shows game keys. `ug hw dump` ≈ `ug inventory --json`. `ug profile apply` (manual game mode without a game) not done. |
| 1.9 resource-budget gate | 🟨 | Measured by hand (05-findings.md); no `scripts/perf-budget.sh` or CI gate. |

### P2 — game mode, detection, profiles, playtime
| Task | Status | Notes |
|---|---|---|
| 2.1 game database | ✅ | 64 titles, Steam IDs verified. "Add an unknown game with one click" not done. |
| 2.2 launchers | ✅ | Steam (native/Flatpak/Snap, all library folders), Lutris (native/Flatpak), Heroic (Epic/GOG/sideload, Flatpak), Bottles (native/Flatpak). No icons yet. |
| 2.3 detection | ✅ | Layers 1–4 automatic. Manual "this is a game" only through GameMode `RegisterGame` (no CLI/UI). Not yet tried on real Steam/Proton/Lutris/Heroic games. |
| 2.4 profiles + revert plan | ✅ | Flat profiles; persistent "always" mode not implemented (by choice, see §C.4). |
| 2.5 CPU | ✅ | Governor/EPP, idle-state hold, preferred cores (affinity), priority + autogroup. No `SCHED_BATCH` for others. |
| 2.6 memory | ✅ | 3 sysctls. THP deliberately untouched; `drop_caches`/zram hints not done. |
| 2.7 app management | 🟨 | Pause/resume by name verified live. Close, CPU-limit, app list with icons: not done. |
| 2.8 services | 🟨 | System timers/interruptible services with revert. Not done: user-session indexers (tracker, baloo), snap/flatpak auto-update holds, per-item "what is this?" texts. Never run as root on real systemd. |
| 2.9 playtime | ✅ | `ug playtime --csv` (one row per session, via `Agent1.Playtime.Sessions`). Not compared with Steam's counter. |
| 2.10 game-mode UI page | ⛔ | Tauri. |
| 2.11 notifications | 🟨 | Written with a "Not a game" button for heuristic detections; not seen on a real desktop yet. |

### P3–P10
P3 done (see git log); P4 in progress (above). ⬜ Not started: P5 GPU tuning & fans, P6
overlay & FPS, P7 recording & Telegram, P8 driver troubleshooter, P9 Linux
extras, P10 packaging & 1.0.

## P5 GPU (branch `p5-gpu`) — built, not merged, 2026-10-01

P6 is merged into `main` (`fc9a5a4`). On `p5-gpu`:
- ✅ 5.0 `ug_hw::gpu_use` (GPU a process really uses: /dev/nvidiaN or DRM
  fdinfo engine time), `ug run --gpu …` / `--steam`, running game's GPU in
  the agent and UIs. Verified: vkcube on NVIDIA (nvidia-smi), on Intel the
  dGPU stayed suspended (Mesa ICDs only + MESA_VK_DEVICE_SELECT). Mode
  switching (iGPU-only/dGPU-only) not offered: eDP is on the Intel card,
  Wayland, no MUX known.
- ✅ 5.1 AMD (power1_cap, pp_od_clk_voltage RDNA1-3) — fixtures only.
- ✅ 5.2 NVIDIA via NVML (power limit, GPC/MEM VF offsets with driver bounds).
  **Never written on the real RTX 3050 yet**: the user allowed a first test
  (60 → 50 W, no offset) and must run dev-install first.
- ✅ 5.3 `ug-stress` (wgpu/Vulkan compute, self-checking; exit 0/2/1) and
  Daemon1.Gpu.TestTune.
- ✅ 5.4 auto-tune faster/cooler (`autotune.rs`), before/after report.
- ✅ 5.5 presets (cooler / less power / stronger) in the window tuner.
- ✅ 5.6 `gputune.rs`: 30 s trial, gpu-pending.json crash watchdog,
  re-apply on wake (never wakes), 88 °C guard; polkit gpu.tune
  (auth_admin_keep). gpu-e2e 10 checks.
- ✅ 5.7 `cooling.rs`: hwmon pwm curves with three failsafes, IdeaPad
  fan_mode; Daemon1.Cooling; `ug fans`; Cooling tab (window + TUI). The
  laptop has no driveable fan.
- ✅ Agent holds power-profiles-daemon "performance" while gaming
  (profile option `power_profile`); verified on the laptop (platform
  profile balanced → performance → back, no prompt).
- ✅ 5.9 window tuner (Game mode → Graphics), TUI Graphics card tab.
- ⬜ 5.8 fan calibration ("learn from my system"): needs a driveable fan.
- ✅ Tray icon in the agent (`tray.rs`): StatusNotifierItem + dbusmenu on the
  agent's session connection, no GTK. Left click opens ug-app; menu has a
  usage preview (CPU, RAM, GPUs via the daemon, battery; read only while the
  menu is open), "Open terminal UI" (`ug tui` in $TERMINAL/konsole/…) and
  "Quit" (closes windows, hides the icon; agent and daemon keep running;
  `ug tray` shows it again). Programs start via `systemd-run --user` so they
  escape the agent unit's NoNewPrivileges. `--no-tray` disables it.
  Verified on KDE Wayland (registered, menu read, click, terminal, Quit).
- ✅ Branding (2026-10-03, `ab39d51`): UG icon (`assets/branding/ug-app-icon.svg`,
  Tauri icons, `packaging/applications/unix-guardian.desktop`), UG mark in the
  sidebar, G-gauge tray states (`health.rs`: normal/warning/critical/gamemode/
  offline, hysteresis, `tray.json` overrides; `scripts/gen-tray-icons.py`
  makes the SVGs and fallback pixmaps; dev-install puts them in hicolor),
  `GaugeLoader.svelte` (intro splash; busy loop stops after 20 s — no idle
  loop, WebKit rule). Verified: warning gauge shown in the KDE panel.
  Not done: GPU thermal-throttle reason (NVML) is not read; the window's
  Wayland app_id vs the .desktop name was not checked.

## P6 overlay (branch `p6-overlay`) — built, not merged, 2026-10-01

P4 is merged into `main` (`dc1fdfa`). On `p6-overlay`:
- ✅ 6.2/6.4 agent: `overlay.rs` (settings in `overlay.json` → Unix
  Guardian's marked block in MangoHud.conf; only-while-gaming rewrites it
  on game start/stop; every-game = `~/.config/environment.d/90-unix-guardian-mangohud.conf`),
  `Agent1.Overlay` (State/Set/SetUp). MangoHud logs every frame
  (`log_interval=0`); `fps::session_stats` turns a log idle 15 s into a
  stored session (avg, 1 %/0.1 % lows from p99/p99.9 frame time, capped at
  the average; stutters > 2× median) and deletes it. `ug overlay`, `ug fps`.
- ✅ 6.5 window Overlay page (preview with live readings) and FPS-tab
  session cards; TUI page 6 and FPS-tab sessions.
- ✅ Bug-report address unixguardian@atomicmail.io (window page, error
  boxes, TUI help/notices, CLI errors, README).
- ✅ VPN handling (on main): category `vpn`, loopback skipped for VPN
  groups, caps follow `ip route get` (TUN), proxy-mode note, ping
  direct/through-VPN and fake-reply detection. net-e2e 35 checks.
- ⬜ Not done: 6.1 writing Steam launch options / Lutris wrappers (no
  launcher on the dev laptop; the UI shows `mangohud %command%`), 6.3
  global shortcuts through the portal (MangoHud's in-game key is used),
  6.6 beta packaging.
- **Not verified on the laptop:** MangoHud is not installed there; nothing
  of P6 has run against a real game. v2rayN in TUN mode not checked yet.

## P4 (branch `p4-network`) — all tasks built, not yet merged (2026-09-30)

`pt-tui` is merged into `main`; P4 is on `p4-network`. Everything below is
committed; `cargo test` 280 pass; smoke/agent/gamemode/power e2e pass;
`scripts/net-e2e.sh` 27/27 via
`PATH=$PATH:/usr/sbin systemd-run --user --scope -q -p Delegate=yes unshare -rn ./scripts/test-bus.sh ./scripts/net-e2e.sh`.

- ✅ **4.6 ping**: `ug-agent/src/ping.rs` (unprivileged ICMP; router, DNS
  server, the game's server; 1/s only while asked or gaming; `Compare`
  verdict around game-mode marks). `ug ping`.
- ✅ **4.4 per-app traffic**: crate `ug-net` (`cgroups`, `nft`, `account`),
  `Daemon1.Net.Apps/Status`, `ug net`. Table `inet unix_guardian`.
- ✅ **4.1–4.3 block and caps**: `ug-net/rules.rs` (net.json, by app *name*,
  always / only while gaming), `nft` block sets + mark maps (only our
  `0x5547xxxx` marks go to ct mark), `ug-net/shaper.rs` (HTB on the
  default-route iface for upload; `ug-ifb0` + connmark + mirred for
  download; never replaces another tool's qdisc). polkit action
  `org.unixguardian.net.control`. `ug net block/limit/allow/rules`.
  Measured in the namespace: 4 Mbit/s cap → 2 MB in 4.2 s each way.
- ✅ **4.5 background tasks**: category catalog in `cgroups::category`,
  `has_game` flag; the agent offers "Limit while I play" by notification
  (once per app per game session).
- ✅ **4.7 network tuning**: profile setting `network_tuning` (Wi-Fi power
  save off via `iw`, TCP Fast Open, notsent_lowat 16384, BBR), reversible
  with crash recovery. Not done: ethernet coalescing, NIC IRQ affinity.
- ✅ **4.8 page** in the window (`app/src/pages/Ping.svelte`) and the TUI
  (page 5).
- Also this session: monitoring **Live** range + **FPS** tab (MangoHud log)
  + `System.Extra` readings; `ug-app --open page/tab/range`.

**Not verified on the real laptop yet** (needs `sudo ./scripts/dev-install.sh`):
blocking and caps on the real Wi-Fi, network tuning during a real game,
the ping page against the real agent. Known limits:
- A game started by Steam runs **inside Steam's cgroup**, so a rule on
  `steam` also hits the game (the UI shows "game inside"). Fix idea: move
  the game's pids into a child cgroup of the Steam unit.
- New app groups are picked up by a 5-second scan; an app's first seconds
  are not blocked or capped yet (inotify on cgroupfs would fix it).
- Apps behind a proxy/VPN app (the user runs v2rayN) are counted and capped
  as that app.
- CAKE and automatic bandwidth detection (roadmap 4.3 "ضد bufferbloat") are
  not built: only per-app HTB caps with fq_codel leaves.

## Next steps, in order

1. **Finish the real install (needs sudo).** Done and verified (see the table
   above), except: the first install left the daemon *disabled* at boot —
   `dev-install.sh` now enables it and installs the system-bus activation
   file; rerun `sudo ./scripts/dev-install.sh` once. Still to run: `sudo kill -9
   $(pgrep -x ug-daemon)` mid-game → daemon restarts → restored from
   `/var/lib/unix-guardian/gamemode.json`.
2. **Push** `main` and `p2-game-mode` to `origin` (user's credentials), then
   fix whatever CI's fmt/clippy/deny report.
3. **Fix the laptop's apt lists** (found 2026-09-30): every Parrot `Packages`
   file under `/var/lib/apt/lists` was 0 bytes while `InRelease` was intact, so
   `apt update` said "Hit" and apt saw no Parrot packages at all — no
   WebKitGTK dev packages, and no Parrot updates for weeks. Fix given to the
   user: `sudo rm /var/lib/apt/lists/mirrors.ocf.berkeley.edu_parrot_*`, then
   `sudo apt update`.
4. Install WebKitGTK dev packages → **Tauri shell (0.6)** → monitoring page
   (1.7) → game-mode page (2.10). Keep the "closed window = no WebView process"
   rule and measure RSS.
5. P2 leftovers: manual "mark as game", close/limit apps, session indexers,
   a real notification check, a test with an actual Steam game.
6. Then **P3 (system control)**.
