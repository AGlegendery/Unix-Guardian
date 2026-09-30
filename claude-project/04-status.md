# 04 — Status (as of 2026-09-30)

## Where the code is

- Repository: `/home/aglegend/SSDBkup/IMPORTANT/ProgramingTable/unix-guardian`
  (on the user's laptop). **No git remote yet — nothing has been pushed**, so
  CI has never run.
- Branches: `master` = P0/P1 foundation. `p2-game-mode` = phase 2 work,
  **not merged yet** (checked out).

| Commit | Branch | What |
|---|---|---|
| `d0ac949` | master | Foundation: daemon, sensors, GPU read path, CLI (P0 + P1) |
| `e4aa5e9` | p2-game-mode | Game library scanning and running-game detection (P2.1–2.3) |
| `278cb62` | p2-game-mode | Per-user agent: detection loop, profiles, playtime, pinning, pausing |
| `2fbfa46` | p2-game-mode | Game mode end to end: reversible tuning in the daemon, reconciled by the agent |
| latest | p2-game-mode | This knowledge pack (`claude-project/`, `CLAUDE.md`), `scripts/agent-e2e.sh`, `scripts/gamemode-e2e.sh` (also run by CI) |

- **172 tests pass** (`cargo test --workspace`): ug-hw 41, ug-games 39, ug-gpu
  35, ug-agent 15, ug-core 15, ug-daemon 12, ug-tune 12, ug-cli 2, doc-test 1.
  `scripts/smoke.sh` (daemon + CLI over a private D-Bus bus) passes.
- `dbus-run-session -- ./scripts/agent-e2e.sh` (10 checks) and `./scripts/gamemode-e2e.sh` (11 checks, incl. "no polkit prompt / no real unit touched") pass on the laptop.
- ~10,600 lines of Rust in 10 crates.
- **Not run locally:** `cargo fmt --check`, `cargo clippy -D warnings` (not
  installed on the dev machine — Debian's Rust 1.94 has no rustup), `cargo deny`.
  Expect CI to flag some style issues on the first push.

## How things were verified

| Verified on the real laptop | Verified only against fixtures/scratch | Never exercised |
|---|---|---|
| sensors, CPU topology, RAPL absence, network (live) | AMD GPU read path (synthetic RDNA1–3 layouts) | anything **as root on the real `/sys`** |
| NVML telemetry, GPU names, capabilities | Steam / Lutris / Heroic library parsing (format samples) | the **system bus**, real **polkit prompts** |
| dGPU runtime-PM skip (0 ms added active time) | game-mode tuning writes (scratch copy of the laptop's `/sys`) | a **real game** (no Steam/Lutris/Heroic installed; a fake `SteamAppId` process was used) |
| NVML lazy load / 30 s release (6.2 → 21.7 → 6.4 MB) | crash recovery of the tuner (unit tests) | desktop **notifications** (tests used `--quiet`/private bus) |
| subscription release on SIGTERM/SIGKILL | | agent under `systemd --user` / autostart |
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
| 1.2 ug-agent | 🟨 | Binary done. No `systemd --user` unit or XDG autostart yet; `dev-install.sh` does not install it. |
| 1.3 ug-hw | ✅ | Plus Intel hybrid P/E cores. |
| 1.4 ug-gpu (read) | ✅ NVIDIA/Intel · 🟨 AMD | Plus runtime-PM skip and lazy NVML. AMD not run on hardware. |
| 1.5 monitor service | 🟨 | Daemon side done (subscribe/park/`Sample`). Agent-side ring buffer and per-minute SQLite history not done. |
| 1.6 network counter | ✅ | Via `/proc/net/route`. |
| 1.7 UI shell/monitoring/settings | ⛔ | Tauri. |
| 1.8 ug-cli | 🟨 | `status/inventory/monitor/gpu/game/playtime`. Missing `ug profile …`; `ug hw dump` ≈ `ug inventory --json`. |
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
| 2.9 playtime | ✅ | CSV export not done; not compared with Steam's counter. |
| 2.10 game-mode UI page | ⛔ | Tauri. |
| 2.11 notifications | 🟨 | Written with a "Not a game" button for heuristic detections; not seen on a real desktop yet. |

### P3–P10
⬜ Not started: P3 system control, P4 network & ping, P5 GPU tuning & fans, P6
overlay & FPS, P7 recording & Telegram, P8 driver troubleshooter, P9 Linux
extras, P10 packaging & 1.0.

## Next steps, in order

1. **First real install (needs the user, needs sudo).** Extend
   `scripts/dev-install.sh` to also install `ug-agent` plus a `systemd --user`
   unit, then on the laptop: `cargo build --release && sudo ./scripts/dev-install.sh`.
   Check: `ug status` without `--session` (system bus), RAPL watts now visible,
   `busctl call … SetLogFilter s debug` raises a polkit prompt, a fake game
   (`SteamAppId=730 bash -c 'exec -a cs2 sleep 120'`) turns game mode on for
   real (`cat /sys/devices/system/cpu/cpufreq/policy0/scaling_governor`,
   `cat /proc/sys/vm/swappiness`, `systemctl list-timers`) and exiting restores
   it; then `sudo kill -9 $(pgrep -x ug-daemon)` mid-game → daemon restarts →
   everything restored from `/var/lib/unix-guardian/gamemode.json`.
   ⚠️ This really changes the CPU governor, sysctls and timers (temporarily).
   Tell the user before asking them to run it.
2. Review and **merge `p2-game-mode` into `master`**.
3. Create the GitHub repository (the `repository` URL in `Cargo.toml` is a
   placeholder), push, fix whatever fmt/clippy/deny report.
4. Install WebKitGTK dev packages → **Tauri shell (0.6)** → monitoring page
   (1.7) → game-mode page (2.10). Keep the "closed window = no WebView process"
   rule and measure RSS.
5. P2 leftovers: agent user unit, `ug profile …`, manual "mark as game",
   close/limit apps, session indexers, playtime CSV, a real notification check,
   a test with an actual Steam game.
6. Then **P3 (system control)**.
