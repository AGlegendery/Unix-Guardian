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
    **Memory floor, measured later:** 307 MB by default → 281 MB with only
    Mesa's EGL vendor (GLVND maps NVIDIA's EGL libraries into both processes
    even when the Intel GPU renders) → **~248 MB** with WebKit compositing and
    GTK GL also off, CPU unchanged at 3 %. Mesa's LLVM (24 MB per process)
    stays: WebKitGTK aborts with no EGL display. The roadmap's 150 MB open
    budget is not reachable with WebKitGTK; closed is 0 (the process exits).
21. **An installed agent sees the e2e scripts' fake games.** It scans all the
    user's processes, so `SteamAppId=730 … sleep` started by a test turned on
    real game mode and wrote real playtime; and a plain `dbus-run-session`
    bus D-Bus-activated the installed agent (real XDG dirs) when the test
    agent did not answer. Fixed with the `UG_TEST_RUN` token (processes of one
    run are invisible to every other detector) and `scripts/test-bus.sh`.

22. **nftables and cgroups (nft 1.1.3, kernel 7.0).** `cgroupsv2` sets with
    `counter` count per element and work for unprivileged root in
    `unshare -rn`. A removed cgroup's element prints as its numeric id and
    cannot be deleted by path or id, so sets are flushed and refilled.
    Paths with systemd's `\x2d` escapes are taken literally.
23. **A Steam game runs in Steam's own cgroup** (`app-steam@….service`),
    so per-app rules cannot tell them apart without moving the game.
24. **Test namespaces:** a peer netns is made by running the peer itself
    under `unshare -n` and moving the veth end in with `ip link set … netns
    PID`; `nsenter -t PID -n` from a nested user namespace fails (EINVAL).
    The browser pane throttles timers while hidden, so a page can look
    empty in a screenshot while it works.

25. **MangoHud 0.7.x (Debian/Parrot package 0.7.2, `--version` v0.7.1)
    writes the frame log only when logging stops or the game exits
    cleanly** (checked with `mangohud vkcube --c 900`: file appears at
    exit; killed by a signal: no file). Live FPS needs 0.8+ (rows flushed
    as written). The session pipeline was checked end to end with that
    real log: avg 144, 1 % 125, 0.1 % 111 for vkcube on the laptop. 0.7's
    columns differ from master (no `cpu_power`); only fps, frametime and
    the last column (elapsed ns) are read.
26. **v2rayN "TUN on" on the laptop did not create a TUN device** (2026-10-01):
    `EnableTun: true` in guiNConfig, but only xray runs with socks/http
    inbounds (sing-box, which v2rayN uses for TUN, was not running), no
    tun interface, no policy rules; apps use HTTP(S)_PROXY. Unix Guardian
    correctly reported proxy mode.

27. **Steam keeps per-game playtime in `localconfig.vdf`** (2026-10-03):
    `userdata/<account>/config/localconfig.vdf`, under
    `UserLocalConfigStore/Software/Valve/Steam/apps/<appid>`: `Playtime`
    (total minutes, all computers) and `LastPlayed` (Unix seconds), written
    only for played apps. The key path was read on the laptop's Flatpak Steam
    (its apps had only `cloud`/`BadgeData`, nothing played yet); the two keys
    come from projects that read them (Playnite issue 1016,
    Steam-Deck-HA-MQTT-Playtime's `get_localconfig_playtime`).
28. **Desktop entries separate apps from the desktop's own processes**
    (2026-10-03): of 70 processes of the user holding a Wayland/X11
    connection on KDE, plasmashell, kded6, ksmserver, the portals, Xwayland,
    kwalletd6 etc. have no visible desktop entry; Firefox, Dolphin, Konsole,
    Telegram, Spectacle, Claude, XDM did (by `Exec` basename), and Chrome by
    its own install directory (`/opt/google/chrome`, the entry runs a
    wrapper there). Helpers in the same directory (crashpad handler, Firefox
    `crashhelper`) lose to the program matched by its own name or with the
    most processes.

## Incident 2026-10-04: the card switch broke KDE

**Never write GPU or overlay variables where the whole session reads
them** (`~/.config/environment.d`, `/etc/environment`,
`~/.config/plasma-workspace/env`, `~/.profile`, `~/.pam_environment`, …).
Per program only: `ug run`, a `.desktop` copy with `PrefersNonDefaultGPU`,
Steam launch options. Enforced by `ug_core::session_env` and the guard test
`crates/ug-core/tests/no_session_wide_writes.rs`.

What happened (from the logs and files; written up by the repair session):
- 2026-10-03 12:12 `edd4115` added the card switch. It wrote
  `~/.config/environment.d/90-unix-guardian-gpu.conf` with
  `__NV_PRIME_RENDER_OFFLOAD=1`, `__GLX_VENDOR_LIBRARY_NAME=nvidia`,
  `__VK_LAYER_NV_optimus=NVIDIA_only`, `UG_DEFAULT_GPU=0000:01:00.0`.
  04-status said "not yet tried across a real re-login". At 20:49 the user
  chose NVIDIA. (The recorder Set up failing that evening was unrelated.)
- 2026-10-04 07:05, first login after it: Plasma Wayland black (windows
  showed only their borders, glitches on every movement); Plasma X11
  garbled textures; XFCE fine (it does not go through the systemd user
  environment). ~14 reboots and generic "fixes" followed (restarting all
  units, wiping caches, rewriting kwinrc to XRender with compositing off,
  a plasma-rescue script that moved all Plasma config to
  `~/kde_backup_20261004_085231` and reinstalled mesa/kwin/sddm, edits to
  `/etc/environment` and modprobe, installing XFCE). None helped, because
  the cause was not found. The user's panels, widgets, themes and KWin
  effects were lost.
- Repair: the file was moved (not deleted) to
  `~/.config/environment.d.disabled/`, `systemctl --user daemon-reexec`,
  reboot; Plasma config restored from the backup (state before restoring:
  `~/kde_before_restore_20261004_100905`); kwinrc deliberately not
  restored (the backup's copy was the broken XRender one), so the old KWin
  effects are gone. The `/etc/environment` (`KWIN_DRM_USE_MODIFIERS=0`,
  `__GLX_VENDOR_LIBRARY_NAME=mesa`) and `nvidia-drm modeset=1 fbdev=1`
  edits were left in place (harmless). Do not touch the backups or the
  restored config.

Why it broke:
1. The module's comment was wrong: "NVIDIA's offload variables touch GLX
   and Vulkan only". `__NV_PRIME_RENDER_OFFLOAD=1` also steers **EGL**
   (NVIDIA's docs: EGL programs need only that variable). KWin on Wayland
   and Qt Quick (plasmashell, apps) all draw through EGL.
2. systemd's user manager reads `environment.d`, and
   `plasma-kwin_wayland.service`, `plasma-kwin_x11.service` and plasmashell
   are user services: the compositor itself rendered on the RTX 3050 while
   the panel is wired to the Intel UHD (no MUX). On X11
   `__GLX_VENDOR_LIBRARY_NAME=nvidia` made KWin use NVIDIA's GLX on an
   Intel screen. `__VK_LAYER_NV_optimus=NVIDIA_only` hid the Intel GPU from
   every Vulkan program.
3. `Linger=yes`: the user manager outlives a logout and keeps the
   variables; even with the file gone they stayed until `daemon-reexec` or
   a reboot. (So the agent's migration also runs
   `systemctl --user unset-environment` for exactly the moved variables.)
4. `__GLX_VENDOR_LIBRARY_NAME=mesa` in `/etc/environment` only fixed GLX;
   EGL and Vulkan still went to NVIDIA.
Hardware and drivers were fine: Mesa 26.1.2, NVIDIA 550.163.01 (kernel
module and userspace match, DKMS built for both kernels), kernel 7.0.13.

The same pattern existed in the overlay's "Load into every Vulkan game"
(`MANGOHUD=1` in `environment.d/90-unix-guardian-mangohud.conf`: every
Vulkan program, GTK 4 apps included, would load MangoHud) and in a Driver
Fix command (`LIBVA_DRIVER_NAME=nvidia` into `/etc/environment`). All three
are gone; the agent moves the two old files to
`~/.config/unix-guardian/retired/` at start and notifies.

**Lesson:** anything that changes the session or the compositor is not done
until the user has logged out and in for real (Claude never runs logout or
reboot; it asks). A "takes effect at the next login" feature was shipped
without that test.

## Recorder Set up on 2026-10-03 (from the journal, 2026-10-04)

The user saw Set up "fail" several times. The journal shows one `flatpak
install` (pid 720798): remote added 23:10:11, GL, NVIDIA GL, Locale, VAAPI
runtimes installed by 23:14:48, then `libostree HTTP error … Failure when
receiving data from the peer` on static deltas of org.freedesktop.Platform
24.08 (~300 MB) at 23:15, and the session was shut down at 23:17 while it
was still downloading. Running the same command on 2026-10-04 finished
(exit 0). Two lessons: Flatpak with `--noninteractive` or into a pipe
prints no progress at all, so the UI looked stuck (fixed: pty + real
progress); and `org.freedesktop.Platform.openh264` comes from
`ciscobinary.openh264.org`, which answers **403** from this network
(geo-blocked); Flatpak only warns, the recorder does not need it.

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
| **"Not a game" was forgotten at the next login** | the detector's ignore list lived in memory only (its test said "now and forever" within one detector) | choices saved in `playtime.json` and loaded at start; the agent test fails without the load |
| **Global shortcuts said "this desktop does not offer them" on KDE, which has the portal** | the portal's property is `version` (lower case); zbus turned the proxy's `version()` into a request for `Version` | `#[zbus(property, name = "version")]`; found by `scripts/shortcuts-e2e.sh` against a fake portal (fails without the fix) |
| **The updated window got "Signature mismatch: got `(sbstsssas…)`, expected `(sbstsssysas…)`" after a reinstall** (2026-10-04) | the agent started at login by the XDG autostart entry kept the bus name with its old program (exe "(deleted)"); dev-install restarted only the unit, whose new agent saw the name taken and stepped aside | a new agent stops an agent whose program file was replaced and takes over (agent-e2e fails without it); dev-install kills outdated agents; CLI and window turn a signature mismatch into "restart the agent" |

## Flatpak recorder kept running after Stop (2026-10-05)

`flatpak run` becomes `bwrap` (same pid as our child); it starts an inner
`bwrap` (the sandbox's first process) which runs the program. A signal to
our child kills only the outer bwrap: the inner one and gpu-screen-recorder
are reparented to the session's systemd and keep recording, while the agent
sees its child gone and reports "stopped" (checked with `flatpak run
--command=sleep`). Seen by the user after the software test's 3-second
recording (portal dialog). Fix: signals (SIGINT stop, SIGUSR1 replay) go to
the `gpu-screen-reco` process found under our child in /proc; `flatpak run
--die-with-parent`; recorders writing into our folder that the agent does
not track are stopped at agent start, before a new recording, and by Stop
when idle (the test's recover uses that).

## Test lab on Hyprland, XFCE and GNOME (2026-10-10)

- **Ping "0/0 answered"** (GNOME, Hyprland): not the VPN. The agent pings
  only while someone asks (`Ping.Recent` touches it); the lab asked once,
  in the ping step, so a freshly idle agent had sent nothing. XFCE showed
  50/50 only because the window had the ping chart open before. The lab
  now asks at the service step and waits up to 8 s for 5 echoes.
- **X11 recording failed** (XFCE, Intel desktop): gpu-screen-recorder
  records a monitor on X11 with Intel/AMD (and on any Wayland) through its
  KMS root helper, and the Flatpak finds that helper only under
  `/var/lib/flatpak` (hard-coded; its README says install `--system` to
  record a monitor). Our Set up installs `--user`, so `eDP-1`/`screen`
  died at once and Stop said only "not running". Window capture
  (`focused`, XComposite) needs nothing. `ug_core::capture` decides:
  automatic on X11 is `screen` where the helper is reachable or NvFBC is
  used (NVIDIA desktop), else `focused`; monitors are not offered where
  they cannot work. Stop now returns the run's own error.
- **"battery none"** in power use: `battery_w` is the discharge draw, only
  measurable on battery. **"via a other interface"**: VPN apps name their
  TUN devices anything (`xray_tun`); the kind is now read from sysfs.
