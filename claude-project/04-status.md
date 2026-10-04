# 04 — Status (as of 2026-10-03)

> Start with `HANDOFF.md` (current branch, work queue, where the last
> session stopped). The sections below are newest-first per phase; the
> table further down is from 2026-09-30.

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

- ✅ Card switch, **redesigned 2026-10-04 after it broke KDE** (see
  05-findings "Incident 2026-10-04"). The first version (2026-10-03,
  `edd4115`) wrote NVIDIA offload variables to
  `~/.config/environment.d/90-unix-guardian-gpu.conf`; its comment claimed
  they touch GLX and Vulkan only, which is wrong (`__NV_PRIME_RENDER_OFFLOAD`
  steers EGL too), so KWin and plasmashell rendered on the RTX 3050.
  Now (window + TUI switch, `ug gpu default`): the choice is
  `~/.config/unix-guardian/default-gpu` and applies only to programs Unix
  Guardian starts (`ug run` without `--gpu`), at once, no login needed.
  One app's menu entry: `ug gpu app <name> [--off]` writes a per-user
  `~/.local/share/applications/<name>.desktop` copy with
  `PrefersNonDefaultGPU=true` + `X-KDE-RunOnDiscreteGpu=true` (marked
  `X-UnixGuardian-Gpu=true`; only marked copies are removed). switcheroo-
  control runs on the laptop, so KDE honours it. Not in the window/TUI yet.
  The agent moves an old environment.d file to
  `~/.config/unix-guardian/retired/` at start, keeps its choice, unsets its
  variables in the user manager and notifies (agent-e2e checks it; it
  never touches `environment.d.disabled`). **Not yet tried on the real
  desktop across a real logout/login.**

- ✅ Playtime page (2026-10-03, P2.9 UI): window + TUI page 7 — today /
  week / all, tiles (time, games, most played), per-game share bars,
  two-press delete (whole history), 7-day chart (window), CSV export to
  Downloads (`ug_core::playtime`, shared with `ug playtime --csv`). Not
  compared with Steam's hours yet (roadmap acceptance).
- ✅ Tracked apps / not a game / Steam import (2026-10-03): any program can
  be timed as an app (`app:<exe>`, never tuned; Apps tab, picker of
  programs open now from desktop entries + Wine programs); a guessed game
  (`exe:`) moves to Apps with its time; "not a game" now survives a
  restart (`playtime.json`); Steam's own totals import into All time
  (only what UG did not count; re-import replaces). CLI `ug playtime
  apps|track|untrack|not-a-game|import-steam`; agent-e2e 27 checks. Real
  Steam import not tried with played games (the laptop's Flatpak Steam
  account has none yet). The user's real Filmora entry is still `exe:` until
  they press "Not a game".

- ✅ Settings page (2026-10-03, `17ea6ec`, roadmap 1.7 settings part):
  `Agent1.Settings` / `settings.rs` (notifications, scan interval 1–10 s
  with the loop's timer rebuilt live, close the window when a game starts),
  `ug settings`; window page (language, tray show/hide, agent settings,
  global shortcuts card, About) and TUI page 12. No theme or start-at-login
  control (see HANDOFF.md). agent-e2e 46 checks.

## P8 driver troubleshooter — first cut on `p5-gpu` (2026-10-03)

- ✅ 8.1 inventory (`ug-driver/inventory.rs`, files only, works without
  the service and unprivileged; root-only parameters fall back to
  cmdline/modprobe.d). Not read yet: compositor, outputs, boot loader.
- ✅ 8.2 rule engine (TOML, condition language, test that every named
  fact exists). No signed rule updates (10.6).
- 🟨 8.3 catalog: 21 rules (roadmap: ≥ 35), each with a fixture scenario
  test; the laptop's real state is a fixture.
- ⬜ 8.4 distro adapter: package installs are commands to copy, per
  apt/pacman/dnf/zypper.
- ⬜ 8.5 snapshots (snapper/timeshift/LVM/ZFS). Light backups of our own
  files exist.
- 🟨 8.6 safe executor: steps with saved undo, auto-undo on failure,
  interrupted ops undoable after restart, `driver-recovery.txt`, `ug
  driver rollback` (works from a TTY: root passes polkit, pkttyagent
  otherwise). Not done: pending-verify after reboot with auto-undo.
- ⬜ 8.7 boot parameter editor (nomodeset / ppfeaturemask are commands).
- ✅ 8.9 UI: window page + TUI page 9 + CLI. No driver-version picker.

## P6 overlay (branch `p6-overlay`) — built, not merged, 2026-10-01

P4 is merged into `main` (`dc1fdfa`). On `p6-overlay`:
- ✅ 6.2/6.4 agent: `overlay.rs` (settings in `overlay.json` → Unix
  Guardian's marked block in MangoHud.conf; only-while-gaming rewrites it
  on game start/stop; the session-wide every-game option (`MANGOHUD=1` in
  environment.d) was removed 2026-10-04, see 05-findings incident),
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
- ✅ 6.1 Steam launch options (2026-10-03, on `p5-gpu`): `ug_games::launch`
  edits `localconfig.vdf` in place (`vdf::set_string`/`remove_key`, every
  other byte kept), Steam must be closed, backup first (10 kept), read
  back, own options remembered (`steam-launch.json`), undo to the byte;
  GPU env (no host Vulkan paths in Flatpak Steam) + `mangohud` /
  `MANGOHUD=1` (Flatpak). Agent1.Games *LaunchOptions methods; `ug game
  launch`; window editor per Steam game (Game Mode → Your games); TUI Game
  mode → Steam launch options. Fixtures + private-bus e2e only: no Steam
  game on the laptop. ⬜ Lutris/Heroic wrappers not done.
- ✅ 6.3 global shortcuts (2026-10-03, `9ddb1c1`): agent `shortcuts.rs`
  binds open-window / toggle-overlay / toggle-gamemode through the XDG
  GlobalShortcuts portal (own connection, host Registry `unix-guardian`),
  off until turned on (`shortcuts.json`; KDE shows a dialog), notification
  on press; game mode by hand (balanced profile; "off" until the games
  close); overlay forced show/hide until the next game start/stop. No
  portal → "unavailable" + `ug shortcut <id>` to bind by hand.
  `Agent1.Shortcuts`, `ug shortcut`, window card (Overlay page), TUI
  Overlay → Global shortcuts. agent-e2e 41 checks. **Not tried on the real
  KDE desktop yet** (portal version 1 is there).
  Keys the user picks (2026-10-03): recorder in the window card ("Change",
  press the keys, Esc cancels) and the TUI (`c` records, Tab adds Meta,
  `x` back to the suggested keys), `ug shortcut keys <id> <keys>`,
  `Agent1.Shortcuts.SetKeys`. Kept in `shortcuts.json` (`keys`, XDG
  format; `keys.rs` reads any spelling), sent as the preferred trigger,
  and on KDE set in kglobalaccel (`setForeignShortcutKeys`; a key another
  program holds is refused with its name, from `globalShortcutsByKey`),
  also re-applied after every bind because KDE keeps old keys. Other
  portal desktops: bind again with the new suggestion. Fixed the portal
  `version` property bug (finding in 05). `scripts/shortcuts-e2e.sh`
  (fake KDE portal + kglobalaccel, `fake-kde-shortcuts.py`) 14 checks.
- 🟨 6.6 beta packaging (2026-10-03, `f9aa5ac`): `scripts/stage.sh` (one
  file list, prefix /usr or /usr/local), `scripts/build-packages.sh` →
  `dist/`: .deb (dpkg-shlibdeps Depends, maintainer scripts, lintian
  clean but for systemctl/man-page/libyaml notices), install.sh tarball
  (/usr/local, --uninstall from a file list, refuses beside a package or
  dev install), Arch PKGBUILD + .install. Checked in Debian/Arch
  containers and a scratch root only. Not done: AppImage, `ug diagnose`,
  GitHub page, announcements (never without the user).
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

## Self-update and releases (2026-10-04, on `p5-gpu`)

- `ug-core::update`: GitHub Releases feed (`UG_UPDATE_FEED` for tests),
  version compare, install kind (package / tarball / development / none),
  minisign verification with the embedded public key.
- Agent1.Update: daily check (Settings `check_updates`, on by default),
  one notification per release; `Install` downloads tarball + signature
  into `~/.cache/unix-guardian/update` and calls Daemon1.Update.Install
  (polkit `org.unixguardian.update.install`, password every time), which
  verifies, keeps the old files for `Rollback`, runs install.sh as its own
  unit. Only tarball installs are updated; packages/dev get the command.
- Front ends: `ug update [--check|--install|--rollback]`, window Updates
  page + Settings switch, TUI page 11 + Settings row. agent-e2e covers
  check and a refused install; a real signed install has never run (no
  release exists yet).
- Releases: `scripts/release.sh VERSION`, `.github/workflows/release.yml`
  drafts a signed release on a pushed `v*` tag. Nothing published; the
  user said to hold the public repo and the release.

## Five UI asks (2026-10-04 afternoon, on `p5-gpu`)

- Internet use: `Monitor.TrafficBuckets` (equal buckets from a start; local
  midnight + 1 h = a day's hours). Window Ping → Internet control: last
  hour / today / 7 / 30 days, 30-day bars, a day hour by hour.
  `ug traffic [--day YYYY-MM-DD|today]`; TUI traffic tab totals.
- Game mode by hand with any profile: `Profiles.SetGameMode/GameMode`,
  dashboard button + profile picker, Game mode page "turn on now with X",
  `ug profile on [X]|off`, TUI `g`. The agent leaves and re-enters when
  the requested settings change (the service applies them only on enter).
- Monitoring history keeps clocks, swap, disk r/w/busy, load average,
  processes, battery (`System.Extra`), net speeds from counters; old
  history.db migrated by ALTER TABLE. Window hour/day/week: averages +
  peaks + nine charts; `ug history` columns + averages.
- Playtime: hours per day stacked by game (7/30), a day's sessions on a
  24 h line, hours of the day, figures (daily average, days played,
  average/longest session, streak), focus on one game. ug-core
  `per_day`/`per_hour`/`streak`; `ug playtime days`; TUI Days tab.
- Profiles: `keep_screen_on` (ScreenSaver hold), `do_not_disturb` (KDE
  Notifications.Inhibit; our own notifications held too), own profiles
  (create from any / delete), pause apps edited in place. Holds use
  NoAutoStart + 2 s timeout (on the test bus a hold call blocked game
  mode before that). gamemode-e2e has a fake desktop for the holds.
- Not tried on the real desktop yet: needs the reinstall. DND on KDE
  and the screen hold are the parts most worth checking by hand.

## Test lab (2026-10-04 evening, on `p5-gpu`)

Asked by the user: the app is meant for every Linux system but was only
really run on the dev laptop, so a section that tests and benchmarks
every feature on the end user's machine and gives a text report to email
to the bug address. Window page «آزمایشگاه تست» (after Bug report), TUI
page 11, `ug testlab [--yes] [--out FILE] [--recover]`.

- Crate `ug-testlab`: one engine for all three. 21 steps over the same
  D-Bus interfaces (`STEPS` in lib.rs). Turned on and then off: game
  mode (all options, own PID), a profile `ug-testlab-probe`, a tracked
  app, a power job and wake alarm 300 days ahead, a block rule for a
  made-up app, settings/overlay saved unchanged, a 3 s recording
  (deleted; skipped when capture is the portal). Benchmarks: snapshot
  latency, 2 s + 4 s CPU spin with a self-check and a reading under
  load, game mode enter/leave, library scan, daemon/agent RSS and CPU.
- Undo journal (`~/.local/share/unix-guardian/testlab-undo.json`) written
  before each change; window/TUI start, the page and `--recover` put
  back what an interrupted run left; flock so one run at a time. Game
  mode is left to the daemon's own revert-on-disconnect.
- Never: session-wide/compositor settings, logout/restart, package
  installs, password prompts (GPU tune, driver fixes): listed under
  "NOT TESTED AUTOMATICALLY".
- `scrub.rs` cleans the whole text: home → `~`, user/host as words
  (host kept when it equals the distro id, e.g. `parrot`), UUID, MAC,
  IPv4/IPv6, ≥32-char hex. Nothing is sent; Copy / Save (Downloads).
- Found: `GameMode.Active` and `Power.WakeAlarm` are unsignalled
  properties, so zbus's caching proxy returns the first read; the lab
  reads them uncached. Other long-lived proxies reading these would have
  the same problem (none found in CLI/window: they create a proxy per call).
- Verified: unit tests (11 + TUI 1), `scripts/testlab-e2e.sh` (24
  checks, fixture sysroot), TUI driven in tmux, window page in the
  browser preview. **Not run on the real machine yet** (needs the
  reinstall; the real service then really changes the governor for a
  few seconds, by design).

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

- ✅ Driver Fix M1 (2026-10-03): the troubleshooter covers Wi-Fi, Bluetooth,
  sound, touchpad/input, camera, printing/scanning, sleep/hibernate and
  firmware, not only graphics. `ug-driver/src/hardware.rs` (files only; the
  kernel log comes from the caller, or `Daemon1.Driver.FirmwareLog`, which
  returns only lines about firmware), a `category` on rules and findings,
  51 rules. Live automatic fixes (no restart, exact undo): rfkill unblock
  (sysfs write), `systemctl enable --now` bluetooth/cups/avahi, our own
  `modules-load.d` file + modprobe (i2c_hid_acpi, uvcvideo). Package and
  boot-parameter fixes are commands until M2–M4. Fixed: two driver changes
  in the same millisecond shared an id; `ug diagnose | head` panicked.
- ✅ Driver Fix M2+M3 (2026-10-03): package fixes run only under a
  snapshot. `ug-driver/src/snapshot.rs`: snapper (root config), Timeshift
  (configured), ZFS root, or a read-only Btrfs snapshot of the root
  subvolume into /var/lib/unix-guardian/snapshots (restore commands swap
  the subvolume from the top level); none → refused with "install
  Timeshift". Every package fix first copies driver configuration and the
  package list (`backups/driver-<id>/config`). `packages.rs`: argv per
  family (apt-get with DEBIAN_FRONTEND, never `pacman -Sy` alone), undo
  removes exactly what the install added (dpkg/pacman database before vs
  after, written ahead). Auto: firmware, SOF, BlueZ (+enable), CUPS
  (+enable), ipp-usb/airscan, HPLIP, libcamera, Vulkan, broadcom-wl,
  headers, NVIDIA on Debian/Arch. Ubuntu's NVIDIA (ubuntu-drivers),
  Fedora/openSUSE NVIDIA (repos first), MOK enrollment stay commands.
  `ug driver snapshot`; the review shows "Safety net".
- ✅ Driver Fix M4 (2026-10-03): boot-parameter editor
  (`ug-driver/src/bootparams.rs`): GRUB (`GRUB_CMDLINE_LINUX_DEFAULT` +
  update-grub / grub(2)-mkconfig, rerun on undo), systemd-boot entries and
  `/etc/kernel/cmdline`, rEFInd `refind_linux.conf`, kernelstub (-a/-d).
  Same name replaces, add+remove gives the file back byte for byte. Fixes
  remove-nomodeset, amd-ppfeaturemask, mem-sleep-deep, psmouse-intertouch
  are automatic; `ug driver param add|remove <p>` (plain characters only)
  and a "Kernel parameters" card in the window; the TUI shows the command
  (no text input there yet).
- ✅ Driver Fix M5 (2026-10-03): the check after a restart. A change that
  needs a restart records its boot id (`verify = restart`); in the next
  boot it becomes `check`. The window shows a "check the change" card
  (keep / undo), the TUI marks it ("y" keeps), the agent notifies once at
  login, `ug driver keep`. A change that can break the desktop (graphics,
  boot parameters) after which no logind session of type wayland/x11 (a
  greeter counts) exists 10 minutes after boot is undone by itself
  (`UG_DRIVER_CHECK_GRACE_S` shortens it in tests). driver-e2e 27 checks.
- ✅ Hibernation swap setup (2026-10-03, asked by the user): fix
  `hibernate-swap` next to the "not enough swap" finding. `ug-driver/src/swap.rs`
  plans a swap file of the memory size (60% if space is short, 5 GB must
  stay free): on Btrfs in an existing `/swap` subvolume or a new nested
  subvolume `/swap` (so root snapshots keep working), made with `btrfs
  filesystem mkswapfile`; ext4/XFS with fallocate + mkswap. The executor
  makes it, adds fstab (pri=-10), swapon, reads the offset (`btrfs
  inspect-internal map-swapfile -r` / filefrag) and UUID (findmnt), adds
  `resume=UUID=… resume_offset=…` through the boot-parameter editor,
  rebuilds the initramfs. Refused (with the step-by-step guide as the
  commands) under lockdown, with mkinitcpio lacking resume/systemd hooks,
  without space. Swap "enough" now looks at the largest non-zram area.
- 🟨 P7 Recorder, first part (2026-10-03, asked as "start the recorder"):
  agent `recorder.rs` drives gpu-screen-recorder (PATH or Flatpak
  `com.dec05eba.gpu_screen_recorder`; "Set up" = `flatpak install --user`
  from Flathub, no root; remove undoes it). Record to ~/Videos/Unix
  Guardian (SIGINT finishes the file), replay buffer 15–120 s (SIGUSR1
  saves; the path comes on stdout), quality small/balanced/high (gsr -q
  medium/high/very_high), 30/60/120 fps, capture portal / monitor / screen
  from `--list-capture-options`, codec, game sound + mic as two tracks,
  MB/minute and buffer memory estimates. `Agent1.Recorder`, `ug record`,
  window page, TUI page 8, shortcuts toggle-recording (Meta+Shift+R) and
  save-replay (Meta+Shift+S). recorder-e2e (stand-in recorder) 15 checks.
  Set up (2026-10-04): `flatpak install --user -y` runs on a pty of its
  own so Flatpak prints its progress line; the agent parses it
  (`RecorderState.setup_pct/setup_step`, fixture
  `crates/ug-agent/testdata/flatpak-install-tty.txt` captured from the real
  thing); window: G gauge (`GaugeLoader variant="progress"`) + bar; TUI: bar
  line; `ug record setup` draws it. A broken download is reported plainly.
  Why the 2026-10-03 Set up "failed": it had not; `--noninteractive` into a
  pipe prints nothing for minutes while the ~300 MB runtime downloads over a
  flaky link, and the machine was shut down mid-download (05-findings).
  **The real Flatpak is now installed for the user** (installed by hand on
  2026-10-04 while finding this), but the agent has **not yet been run
  with the real gpu-screen-recorder**: its `--list-capture-options` lines and the replay-saved stdout
  line are from its documentation, check them first. Not done: region
  capture, quality presets measured against real files (±10%), the clip
  library with thumbnails, ffmpeg compression, Telegram (7.4), audio
  device choice from pw-dump, highlight on loud sound.
