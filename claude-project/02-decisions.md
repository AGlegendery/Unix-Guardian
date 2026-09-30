# 02 — Decisions

Everything here is settled unless the user reopens it. Part A is what the user
chose, Part B the technical decisions built on that, Part C where the
implementation deliberately departs from the approved roadmap and why.

## A. The user's answers (quoted, then what they mean)

| Question | User's answer (Persian, verbatim) | Meaning |
|---|---|---|
| UI stack | «اگر سنگین نمیشه و از رم زیادی استفاده نمیکنه که مزاحم گیمینگ بشه و لگ بندازه روی کاربری روزانه از Tauri استفاده کن» | Tauri — **on condition** it stays light. Hence: the UI is a separate, optional process that is destroyed (not hidden) when closed. |
| Platforms | «فقط لینوکس ورژن ویندوز وجود داره» | **Linux only.** A Windows version already exists separately. |
| Team | «فعلا فقط خودم» | Solo developer. Estimates are for one person. |
| GPU scope | «باید همه‌ی Architecture ها پشتیبانی شه ولی نه همه بیشترشون NVIDIA سری ۱۰۰۰ به بالا و AMD سری ۵۰۰۰ به بالا و … یک گزینه‌ی انتخاب بین GPU ها داخلش باشه و GPU Driver Troubleshoot برای تمام لینوکس یوزرها — نصب درایورها و سینک و عیب‌یابی کارت گرافیک‌ها Nvidia سخته و میخوام این قضیه حل شه» | NVIDIA **GTX 1000 (Pascal) and newer**, AMD **RX 5000 (RDNA1) and newer**; a **GPU selector**; a **driver troubleshooter** that makes NVIDIA install/sync/debug easy for every distro. |
| Overlay | «حتما نیازی نیست شبیه تصویر بشه UI هرچیزی خودت صلاح میدونی MongoHud به عنوان موتور» | **MangoHud is the overlay engine.** The UI need **not** copy the Windows screenshots — design freedom. |
| Driver troubleshooter depth | «تشخیص + نصب با تأیید و snapshot» | Diagnose **and** install, but only **after explicit confirmation and with a snapshot** to roll back to. |
| Licence | GPL-3.0-or-later | Allows forking/borrowing from GPL tools (MangoHud, gpu-screen-recorder). |
| Relation to the Windows app | «قابلیت‌هاش همه فقط بجای ویندوز روی لینوکسه این کامل‌ترین توصیفه» | Same features, independent Linux codebase. |
| AMD testing | The user has **no AMD machine**; a friend does, available later. «تمام بخش‌هایی که به کارت AMD ربطی نداره رو بساز» | Build everything; defer AMD *hardware* verification. See 08-hardware-plan.md. |
| Commits | «آره کامیت کن و برو سراغ فاز ۲» | Committing authorised; phase 2 work goes on branch `p2-game-mode`. |

### A note on «هسته‌ی تمام‌راست»

In the first message the user wanted to offer «هسته‌ی تمام راست برای بهینه‌سازی
بهتر». I first read it as a *fully real-time (PREEMPT_RT) kernel* and advised
against that — RT kernels trade throughput for bounded latency, hurt gaming, and
NVIDIA's driver does not support them; sched_ext schedulers, `split_lock_detect`,
ntsync and gaming kernels are the useful levers (roadmap P9). The user then asked
«یعنی میگی Rust برای نوشتن هسته آپشن مناسبی نیست؟», which shows they most likely
meant **an all-Rust core** (راست = Rust). Both are now true: **the core is 100%
Rust**, and **PREEMPT_RT is not used**.

## B. Technical decisions

| Topic | Decision | Why |
|---|---|---|
| Core language | Rust (workspace, edition 2021, `rust-version = 1.85`), tokio | The daemon runs as root; memory safety is a security property here. LACT and CoolerControl made the same choice. |
| Processes | `ug-daemon` (root, system bus) · `ug-agent` (user session, session bus) · `ug-app` (Tauri, optional) · `ug` CLI | Privileged work (sysfs writes, NVML, nftables, logind) vs. session work (the user's processes, launcher libraries, notifications, MangoHud, recording) vs. a UI that can be closed. |
| IPC | D-Bus via `zbus 5` (tokio), authorisation via **polkit** per method | Linux standard; lets CLI, scripts, the Telegram bot and even Flatpak apps use one API. |
| UI | Tauri 2 + Svelte 5 + TypeScript + Tailwind 4; charts with uPlot; font Vazirmatn (bundled) | Small bundles, fast live charts. React and Chart.js rejected. **Not started** — needs `libwebkit2gtk-4.1-dev`. |
| Overlay | MangoHud as engine; Unix Guardian writes its config, theme and hotkeys | Works on Wayland (including GNOME) and exclusive fullscreen; a custom Vulkan layer is not justified for one developer. |
| Recording | `gpu-screen-recorder` (NVENC/VAAPI/AMF, replay buffer) | Already has the "smart highlight" replay buffer; Wayland-safe. |
| Scheduler extras | sched_ext via `scx_loader` D-Bus (kernel ≥ 6.12) | Scheduler switch without reboot while a game runs. |
| Storage | SQLite (`rusqlite`, bundled) in the agent; TOML config | `/etc/unix-guardian/` (daemon config), `/var/lib/unix-guardian/` (daemon state), `~/.config/unix-guardian/` and `~/.local/share/unix-guardian/` (user). |
| UI language | Persian (RTL) default + English; Paraglide for i18n | Weblate in P10. |
| GPU support | NVIDIA Pascal+ (proprietary on Pascal, open module on Turing+) · AMD RDNA1+ (voltage on RDNA1/2, offset only on RDNA3/4) · Intel monitoring only | Intel exposes no tuning interface on Linux. |
| NVIDIA "undervolt" | **Not offered as undervolting.** NVIDIA's Linux driver has no voltage/frequency curve. Offered instead: lower power limit + positive clock offset, which gives the same cooler/quieter result — and the UI says exactly that. | Honesty: the Windows screenshot's "−194 mV" is impossible on Linux NVIDIA. |
| Distro tiers | **Tier 1:** Arch/CachyOS/Manjaro/EndeavourOS, Debian/Parrot/Ubuntu/Pop/Mint, Fedora/Nobara · **Tier 2:** openSUSE, Bazzite/Silverblue/Kinoite (immutable: no package changes) · **Tier 3:** NixOS, Void, Alpine, Gentoo, unknown (diagnose + print commands only) | Debian family is Tier 1 because the user's machine is Parrot. An unlisted derivative of a Tier 1 distro is Tier 2. Implemented in `ug-core/src/distro.rs`. |
| Packaging | Native packages (AUR, deb, rpm) + `install.sh`; Flatpak for the UI only | A root daemon cannot live in a sandbox. |
| Licence | GPL-3.0-or-later | See A. All dependencies checked GPL-compatible (`deny.toml`). |

## C. Where the implementation departs from the roadmap (deliberately)

The roadmap (10-roadmap.fa.md) was approved before any code existed. These
changes came from building and measuring; keep them unless there is a reason.

1. **"CCD lock" became "preferred cores".** One setting, `cpu_preferred_cores`,
   pins the game to the **V-Cache CCD on AMD X3D** or the **P-cores on Intel
   hybrid** (the user's own i5-12450HX has P-cores 0–7, E-cores 8–11). Done in
   the **agent** with `sched_setaffinity` on every thread — a user may do that to
   their own processes, so no root and no cgroup surgery (roadmap 2.5 planned a
   cgroup cpuset in the daemon). Nothing to restore: affinity dies with the game.
2. **Resource budget for the daemon.** Roadmap: ≤ 15 MB RSS. Measured: **6.2 MB
   idle**, but NVIDIA's NVML library alone adds ~15 MB while loaded. NVML is
   therefore loaded lazily and released 30 s after last use. Proposed budget:
   **≤ 8 MB idle, ≤ 25 MB while an NVIDIA GPU is being read.** (Awaiting the
   user's OK — see 09-open-questions.md.)
3. **A sleeping laptop dGPU is never woken for telemetry.** Not in the roadmap;
   found by measurement (05-findings.md). Telemetry checks PCI runtime PM first.
4. **Profiles are flat booleans**, not sections with `when = "in_game" |
   "always"`: `cpu_performance`, `cpu_no_idle_states`, `cpu_preferred_cores`,
   `cpu_priority`, `memory_tuning`, `pause_services`, `pause_apps`. The
   **"always" (persistent) mode is not implemented** — every change is temporary
   and reverted. Adding "always" safely is an open question.
5. **Transparent huge pages are left alone** (roadmap 2.6 said `always` in game):
   `always` can add latency spikes; not worth the risk without measurement.
6. **Service pausing is limited to timers and interruptible services.**
   Package-manager services (apt, dnf, PackageKit, unattended-upgrades) are
   **never** stopped — killing a package operation mid-run can break the system.
   Their *timers* are held instead. User-session indexers (tracker, baloo) are
   not handled yet.
7. **Priority also raises the autogroup.** With `kernel.sched_autogroup_enabled=1`
   (on for the user and most desktops), per-thread nice only orders threads
   inside the game's own group; the game's **autogroup nice** is what gives it an
   edge. It is restored afterwards through any surviving member of the group.
   `SCHED_BATCH` for other apps (roadmap) is not implemented.
8. **Game mode polkit action allows active local sessions without a password**
   (`org.unixguardian.gamemode`, `allow_active=yes`) — it runs automatically at
   game start, so a prompt would appear mid-launch. Same trust as Feral GameMode;
   everything is temporary and reverted.
9. **games.json has 64 titles, not 80**, and every Steam App ID is verified
   against the Steam store API (`scripts/verify-games-db.py`). Executable names
   are included only where known with confidence. Schema: `id, name,
   steam_appid, exe[], cmdline[], profile, linux ("blocked"), linux_note`.
10. **GPU names come from the system `pci.ids`**, not a hand-written table (one
    was tried; the first entry checked against real hardware was wrong).
11. **The agent reconciles game mode instead of firing events**: every tick it
    compares "wanted" with "confirmed by the daemon" and fixes the difference, so
    lost calls, daemon restarts and reconnects heal themselves.
12. **Development runs (`--session` or `--sysroot`) never touch the real
    system's services and skip polkit** — found the hard way (05-findings.md).
13. **UI i18n is a typed dictionary, not Paraglide** (`app/src/lib/messages/`):
    Paraglide's compiler fetches its message-format plugin from a CDN at build
    time, and distribution builds run without network. English is the type,
    so a missing Persian string fails the type check.
14. **No tray icon in the window process**: closing the window ends `ug-app`.
    Measured, the tray-only state still used 124 MB. A tray (StatusNotifierItem)
    belongs in the agent, which can start the window on demand (not done yet).
15. **The UI follows Phoenix Guardian's visual design** (dark, purple,
    rounded cards, right sidebar) at the user's request on 2026-09-30, after an
    independent "circuit board" preview; see PRODUCT.md.
17. **nftables through the `nft` program, not `rustables`.** `nft -f -` is
    atomic and handles `cgroupsv2` sets, maps and per-element counters,
    which the netlink crates did not cover when this was built. Everything
    lives in one table, `inet unix_guardian`.
18. **Apps are told apart by the cgroup systemd already gives them**, not by
    moving processes into `ug.slice/ug-net-<class>.scope` (roadmap 4.1).
    Counting, blocking and marking match `socket cgroupv2 level L`; rules
    are saved by app name so a restarted app is caught again.
19. **Upload and download caps are HTB classes with fq_codel leaves**
    (download through `ug-ifb0`); CAKE and bandwidth detection are later.
16. Smaller: default network interface is read from `/proc/net/route` (not
    rtnetlink); logs go to stderr in a journald-friendly format (no
    `tracing-journald`); notifications call `org.freedesktop.Notifications`
    directly (no `notify-rust`); GPU names prefer NVML's once it has loaded.
