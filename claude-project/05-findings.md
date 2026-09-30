# 05 — Findings from real hardware

Facts established by measurement on the user's laptop (see 06-dev-environment.md
for the machine), and bugs that only appeared when the code actually ran. Each
one is encoded in code and tests now; keep them in mind when extending nearby.

## Measured resource use (release build)

| What | Value |
|---|---|
| `ug-daemon` / `ug` binary size | 2.4 MB / 1.8 MB (`opt-level="s"`, LTO, stripped) |
| Daemon start-up | **59 ms** (was ~1.6 s while it initialised NVML at discovery) |
| Daemon idle, no subscribers | **6.2 MB RSS, 0.000 % CPU** — the loop truly parks |
| Daemon sampling at 1 Hz | 6.3 MB, **0.2 % CPU** |
| Daemon while NVML is loaded | 21.7 MB → back to 6.4 MB ~31 s after last use |
| Daemon's own code without NVML | 4.7 MB (NVML = 0.9 MB code + 13.8 MB heap) |
| `Snapshot()` latency | ~9 ms with the GPU awake |
| Detection, 518 processes | first scan 74 ms, then **1.1 ms per scan** every 2 s |

## Hardware and OS behaviour

1. **Querying a sleeping laptop dGPU wakes it — and keeps it awake.** Any NVML
   call (and any `amdgpu` sysfs read) on a runtime-suspended GPU takes ~1.5 s and
   leaves it powered while polling continues: a 1 Hz monitor would drain the
   battery. The laptop's dGPU had spent ~28 h suspended in total. **Rule: read
   `/sys/bus/pci/devices/<pci>/power/runtime_status` first; if `suspended` or
   `suspending`, do not query — report `asleep`.** Verified: 5 s of 1 Hz
   monitoring added 0 ms of dGPU active time.
2. **NVML costs ~15 MB while loaded, and `nvmlInit` itself wakes a sleeping GPU**
   (1.6 s; 15 ms if already awake). Shutting it down returns all the memory.
   Hence: load lazily, only for an awake GPU; release 30 s after last use.
   Reproduce: `cargo run --release -p ug-gpu --example nvml_lifecycle`.
3. **`/proc/driver/nvidia/gpus/*/information` also wakes the GPU** (and contains
   the GPU's UUID — never collect it). The NVIDIA driver version is readable
   without waking from `/sys/module/nvidia/version`.
4. **NVIDIA registers no hwmon chip**; all NVIDIA telemetry must come from NVML.
   NVML writes PCI bus IDs with an 8-digit domain (`00000000:01:00.0`), sysfs
   with 4 — compare the `bus:dev.fn` tail.
5. **NVIDIA laptop fans belong to the embedded controller**: `fan_speed(0)`
   fails. Report "fan control: no" with that reason, not an error.
6. **RAPL `energy_uj` is mode 0400 (root only)** on current kernels (PLATYPUS
   mitigation). CPU package power appears only when the daemon runs as root.
7. **`intel_pstate` refuses EPP writes (EBUSY) while the governor is
   `performance`.** Restoring EPP before the governor silently fails; the tuner
   restores a policy governor-first as one undo entry.
8. **Autogroup scheduling is on** (`/proc/sys/kernel/sched_autogroup_enabled=1`,
   the desktop default): nice values only compete *inside* an autogroup. To
   prioritise a game, raise its autogroup (`/proc/<pid>/autogroup`), too.
9. **`/dev/cpu_dma_latency`**: the latency request lasts while the fd is open and
   the kernel drops it on close — crash-safe by construction, no undo needed.
10. **Intel hybrid core types** are in `/sys/devices/cpu_core/cpus` (P) and
    `/sys/devices/cpu_atom/cpus` (E). The laptop: P = 0–7 (4 cores × SMT,
    4.4 GHz), E = 8–11 (3.1 GHz).
11. `sysctl` is not on the user's `PATH` on Parrot (`/usr/sbin`) — read/write
    `/proc/sys/...` directly. THP is already `always` on this machine.
12. **Chromium/Electron/CEF rewrite their process title**: `/proc/<pid>/cmdline`
    becomes ONE space-joined string (a single NUL), so `--type=gpu-process` is
    not a separate argument and splitting `argv[0]` on `/` yields garbage like
    `Claude --gpu-preferences=…`. Seen with Chrome, the Claude desktop app,
    GapGPT and an AppImage. **Name native processes by `/proc/<pid>/exe`; look
    for `--type=` inside a rewritten title too.**
13. **Wine/Proton processes:** `argv[0]` is the Windows path (`C:\…\Game.exe`,
    may contain spaces), `comm` is truncated to 15 bytes, and `/proc/<pid>/exe`
    points at the Wine preloader — `argv[0]` is the only place the game's name
    survives.
14. **Steam:** `SteamAppId` is set on everything Steam launches and inherited by
    the whole tree, including `reaper`, `fossilize_replay` (shader
    pre-compilation — not a game) and runtime tools; tools have their own app IDs
    (Proton, Steam Linux Runtime…). `~/.steam/steam` is a symlink to
    `~/.local/share/Steam` (canonicalise or every game appears twice).
    `StateFlags` bit 4 = fully installed.
15. **Bottles** stores the same program twice when re-added, bottle `Name:` is
    not unique (directories `RedDead` and `RedDead__149`), and long paths wrap
    across YAML lines. Identify bottles by directory; de-duplicate programs.
16. **Anti-cheat that blocks Linux** (in games.json with a reason): Riot Vanguard
    (League of Legends, VALORANT — both in the user's Windows screenshots),
    Fortnite (EAC not enabled), Apex (EA disabled Linux in 2024), PUBG, Rainbow
    Six Siege, Destiny 2 (BattlEye), Call of Duty (Ricochet), GTA Online.
17. **GPU marketing names**: take the bracketed part of the `pci.ids` entry and
    prefix the vendor (`GA107BM / GN20-P0-R-K2 [GeForce RTX 3050 6GB Laptop GPU]`
    → "NVIDIA GeForce RTX 3050 6GB Laptop GPU", identical to NVML). A hand-written
    table said "RTX 2050" for this card — wrong, and removed.

18. **polkit treats the agent under `systemd --user` as the active session.**
    Processes in `user@1000.service` belong to no login session, but polkit
    resolves them to the user's display session, so `allow_active=yes`
    (`org.unixguardian.gamemode`) applies: game mode turned on in ~1 s with no
    prompt on the real install.
19. **Measured on the real install:** agent 5.4 MB RSS under systemd, daemon
    6.2 MB; the `nice -10` game also got its autogroup to `nice -10`.

20. **The UI window's cost (WebKitGTK 2.52, release build, dashboard at 1 Hz):**
    open, all processes together about **244 MB PSS** (ug-app ~90, WebKit web
    process ~140, network process ~25) and **2–5 % of one core** (2 % with the
    DMA-BUF renderer, 5 % without). One endlessly pulsing CSS dot made it
    **~100–130 %**: WebKitGTK repaints the whole window at 60 fps. Keeping a
    tray icon in the window process after closing the window still held
    **124 MB** and left WebKit's network process running, so the window process
    now exits on close (tray belongs in the agent).
21. **An installed agent sees the e2e scripts' fake games.** It scans all the
    user's processes, so `SteamAppId=730 … sleep` started by a test turned on
    real game mode and wrote real playtime; and a plain `dbus-run-session`
    bus D-Bus-activated the installed agent (real XDG dirs) when the test
    agent did not answer. Fixed with the `UG_TEST_RUN` token (processes of one
    run are invisible to every other detector) and `scripts/test-bus.sh`.

## Bugs that only running the code revealed (all fixed, all with tests)

| Bug | Cause | Fix |
|---|---|---|
| Daemon kept sampling after `ug monitor` was killed by `timeout` | counted subscribers, CLI only unsubscribed on Ctrl-C | subscriptions keyed by unique bus name, released on `NameOwnerChanged`; CLI also handles SIGTERM |
| First sample of `ug monitor` missing | CLI subscribed before adding its signal match rule | listen first, then subscribe |
| `ug monitor --json \| head` panicked | `println!` on a closed pipe | treat `BrokenPipe` as a clean end |
| NVML released after up to 60 s instead of 30 | release loop slept a full grace period each time | sleep only the remaining time |
| Tuner reported "CPU set to performance" when the write had failed | undo recorded, result of the write ignored | write-ahead helper withdraws the undo entry on failure and reports "skipped" with the reason |
| **A development daemon stopped 7 real system timers and raised polkit password prompts on the user's desktop** | `--sysroot` redirected file writes, but `systemctl` still acted on the real machine | development runs (`--session`/`--sysroot`) use a disabled service manager; `systemctl` also gets `--no-ask-password`. Verified afterwards: all 7 timers active again; 0 prompts / 0 unit changes in later runs |
| `busctl get-property … Active` timed out | the property locked the tuner mutex from async code while `Enter` held it | `Active` is an `AtomicBool` |
| Game mode never left after the game exited | agent dropped its connection on any error, got a new unique name, and the daemon refused the "stranger" | ownership by **user**; agent reconciles wanted vs confirmed state every tick |
| **The Claude desktop app was detected as a game** | Chromium process-title rewriting (finding 12) | `--type=` check on split args; native names from `/proc/<pid>/exe` |
| Agent tests flaky with "Text file busy" | writing an executable while other test threads fork | symlink to `/bin/sleep` instead of copying |
| `capture-fixture.sh` exited silently | `set -e` + `cond && cmd` as a function's last command | explicit `if`, `return 0` |
| **A second daemon or agent silently took over the running one** — and a second daemon ran crash recovery on the live undo list, reverting a game's settings mid-game | zbus's `Builder::name` requests names with `AllowReplacement \| ReplaceExisting` by default | names claimed with `DoNotQueue` only; the daemon claims its name *before* crash recovery and exits non-zero if taken; a second agent exits 0 (so systemd does not restart it). e2e checks in both scripts |
| After the first real install the daemon would be gone after a reboot | `dev-install.sh` started the unit but never enabled it, and did not install the system-bus activation file | both added (checked in a container) |
| `ug playtime --csv \| head` would panic | `println!` on a closed pipe (same as the monitor bug) | explicit writer, `BrokenPipe` = clean end |
| Fixture leaked the root filesystem UUID | `/proc/cmdline` copied verbatim | UUIDs zeroed; gateway zeroed; no serials/MACs/UUIDs collected |
