# HANDOFF — read this first

The short version of where the work is, so a new session (Claude Code on the
user's laptop, possibly started cold after the previous one died) can resume
in minutes. **Update this file after every finished item and commit it.**
Details live in `04-status.md` (per-task truth), `02-decisions.md`,
`05-findings.md`, `07-conventions.md`, `03-architecture.md`.

Last updated: 2026-10-18 (threshold alerts; on `main`).

## Now (2026-10-18, evening): "sleep until" blocked by an inhibitor ✅

User: timed sleep said "Operation denied due to active block inhibitor".
Blockers on the laptop: Ryoku caffeine mode (sleep:idle, user-toggled)
and ryoku-shell ("route suspend through a compositor-secure session
lock"; it also has a delay inhibitor + PrepareForSleep lock). Fixed in
`adffa6f`: Power.Blocked error, Power.Blockers, Power.NowAnyway
(SKIP_INHIBITORS, polkit power.ignore-inhibit = auth_admin_keep),
`ug_core::sleepguard` (plain words; lock first: `ryoku-shell lock` +
`ryoku-shell lock status`, else loginctl + LockedHint; unconfirmed →
ask). Window/TUI/CLI flows; power-e2e with UG_SIMULATED_INHIBITORS.
**Not tried for real** (never suspend from a session): the user tries
"sleep until" → Sleep anyway; check the lock screen is up on wake and
the alarm woke it. Needs the new polkit policy: reinstall.

## Before (2026-10-18, later): the user's visual notes ✅

User could not find the alerts (install was fine: they live in
Monitoring → 5th tab; Settings → Notifications now links there) and
the visual notes from their Phoenix analysis were not done. Done in
`1ac4a2b`: darker 3-level surfaces + Pure black (OLED) option,
category colours (cpu blue, gpu green, mem teal, net light violet,
power yellow, danger red) via `data-cat` / `--cat`, coloured card
hover in both motion styles, `ui/ModeSwitch.svelte` (big switch + On /
Off / Waiting for a game badge) on game mode, replay buffer, overlay,
background slowdown, one-shot rainbow border (2 sweeps, then still),
brighter off switches, TUI tile colours + game-mode badge.
**Waiting on the user:** reinstall, look at it, say what to change.

Later: migrate to zbus 6 (zvariant / zbus_names / zvariant_derive merged
into it; their unmaintained notices RUSTSEC-2026-0370..0372 are ignored
in deny.toml until then). Not started; only on the user's word.

## Before (2026-10-18): threshold alerts ✅

Phoenix Guardian parity list, item 1 (see 04-status "Threshold alerts").
Agent `alerts.rs` + Agent1.Alerts, `ug alert`, window Monitoring →
Alerts + Dashboard card, TUI Monitoring tab 4, test-lab step,
`scripts/alerts-e2e.sh` in CI. Release binaries and window rebuilt.
**Waiting on the user:** `sudo ./scripts/dev-install.sh`, then "Send a
test alert" and a real alert (e.g. `ug alert set cpu 50` with
`ug alert set sensitivity 3`, then put it back with `ug alert set cpu 90`).
Next parity items: ask the coordinator.

## Now (2026-10-17): visual fixes + Settings regrouped ✅

- Scrollbars hide when idle: `lib/viewport.ts` `watchScrolling` sets
  `data-scrolling` on a box for 900 ms after it scrolls; app.css shows the
  thumb only then (`::-webkit-scrollbar` for WebKitGTK, checked offscreen).
- Lively hover glow was on wrapper sections (`.row`, `.tiles`): one square
  box around two rounded cards. Now only `section.card`/`article.card`.
- Sidebar icons no longer slide 2 px on hover (Lively); their Hugeicons
  motion is unchanged.
- Sidebar stuck scrolled after the window grows: `clampScroll` action
  rebuilds the box's scroll layer on resize (offscreen WebKit never showed
  the bug: layout reads were right, only the real window painted stale).
- No page zoom: Ctrl+wheel/keys/gesture events blocked in JS, viewport
  `user-scalable=no`, and the GTK window swallows touchpad pinch and
  Ctrl+scroll in GDK's event handler (`no_page_zoom`, main.rs; `gtk` dep).
  Pinch not verifiable offscreen: ask the user.
- 2026-10-18: the first version connected to `captured-event`, which this
  GTK 3 does not have: glib panics, the window aborted at start (user:
  "doesn't open"). Lesson: after any change in app/src-tauri, start the
  release `ug-app` for ~10 s before asking the user to install.
- 2026-10-18, second: the rebuild of that fix was a plain `cargo build
  --release`, which leaves custom-protocol off: installed, the window
  showed "Could not connect to localhost". main.rs now refuses to compile
  a release build without it. Only `npx tauri build --no-bundle`. Check
  the page renders with the vite server stopped (grim the window).
- Settings: Appearance | Window and tray (+ new icons-only sidebar switch,
  `lib/sidebar.svelte.ts`), Notifications and updates | Finding games,
  Internet use, shortcuts, About. TUI Settings grouped the same, with a
  "Reset the internet-use counts" row (Enter twice).
- Working tree repaired from git (modes, `:` fixture paths, symlinks); the
  renamed copies were moved out, not deleted.
- Game vs heavy app detection: proposal sent to the user, nothing built.
**Waiting on the user:** `sudo ./scripts/dev-install.sh`, check pinch zoom
and the sidebar after resizing; pick an option for game detection.

## Before (2026-10-16): icons really move, in Classic too ✅

- User: icons didn't animate (nothing after reinstall; later only a small
  zoom). Causes: HugeIcon played only in Lively, and Classic is the default;
  the installed `ug-app` (2026-10-15 14:51) predates `ac44814`. The port
  itself was right: regenerating from fresh registry files gave a
  byte-identical hugeicons.json, and the frames in WebKitGTK move per path.
- Now icons play on hover and click in both motion styles (only reduced
  motion stops them); Classic still keeps pages/cards calm (Settings note
  says so). Default stays Classic.
- Weak custom motions made clearer (sliders slide, cpu/gpu/ram/moon/snow/
  layers/swap/chevrons/terminal/speed/ban bigger).
- `scripts/icon-motion-check.py` (needs `npm run dev`, page
  `app/dev/icons.html`): hovers every icon in WebKitGTK offscreen, fails an
  icon that barely changes; `--strip out.png` shows the frames. It failed on
  every icon before the fix (Classic).
**Waiting on the user:** `sudo ./scripts/dev-install.sh` (release window
built 2026-10-16 18:39), then hover the sidebar icons.

## Also (2026-10-16): hibernate swap warning restored ✅

- The user wants Driver Fix to warn when swap is too small for hibernation
  (laptop: 16 GB swap, 27 GB RAM). `c1137ca` had dropped it as a false
  positive (image_size rule); back to swap >= 60 % of RAM (`hardware.rs`
  `hibernate.swap_enough`, `sleep.toml` text, test with the laptop numbers).
  image_size is a target, not a promise. Do not lower it again unasked.
- The project folder moved to `/home/aglegend/IMPORTANT/ProgramingTable/
  unix-guardian`; the copy flipped ~819 files to mode 755, renamed the
  fixture paths with `:` to `_` and dropped the fixture symlinks under
  `crates/ug-testfs/fixtures/dev-laptop-rtx3050/sys`. Repaired 2026-10-17.

## Before (2026-10-15, later): animated Hugeicons + full-charge wording ✅

- The user's 85 `@hugeicons-animated` icons (a shadcn registry of React +
  motion components) are ported, not React-in-Svelte:
  `scripts/port-hugeicons.cjs <dir of registry json>` runs each TSX against
  stand-ins and writes `app/src/lib/icons/hugeicons.json` (SVG markup +
  resolved variants + the MIT notice). `HugeIcon.svelte` + `player.ts`
  replay them (keyframes, times, easing, springs, pathLength). Plays in both
  motion styles since 2026-10-16, never under reduced motion; endless repeats play twice and stop.
  Used in the sidebar (dashboard, playtime, bug report, updates, settings),
  battery/history headers, notifications row, copy (plays on copied),
  updates, delete/add, chevrons, alarm. Credit in Settings → About;
  `app/src/lib/icons/LICENSE-hugeicons.md`.
- User rule (2026-10-15): every icon in the window is an animated Hugeicon;
  none from another family. Icons the set lacks (24: nav game/system/
  monitoring/overlay/recorder/fix/lab/update, terminal, cpu, ram, gpu,
  network, moon, snow, save, export, sliders, shield, ban, gauge, swap,
  sidebar chevrons) are in `scripts/hugeicons-custom.cjs`: Hugeicons' own
  free shapes (`npm pack @hugeicons/core-free-icons`) + motion in the set's
  style. Regenerate: `node scripts/port-hugeicons.cjs <registry dir>
  <core-free-icons package dir>`. @lucide/svelte is gone.
- Battery "full charge once": the daemon restores the limit as soon as the
  battery reads 100 % or Full (checked every 5 min); wording fixed in en/fa/TUI.
**Waiting on the user:** rebuild + reinstall, try Lively hover on icons.

## Before (2026-10-15): Hyprland, sleep, fans, battery, Driver Fix, motion ✅

Seven asks from the user, all on `main` (`2b17bb6` … `dc28602`), release
binaries and the window rebuilt; CI was red on fmt only, fixed in `dc28602`.
- Window on tiling compositors: min size 480×400, sidebar folds < 820 px,
  dark backing (05-findings "Window resizing on Hyprland").
- Fans: ideapad fan_mode writes are ignored by this laptop's EC; probed
  once (switch and back), verdict in the daemon's `firmware.json`, shown as
  "not supported by firmware"; a pwm that will not leave auto is mode
  `firmware`.
- Sleep and wake tab: Power.SleepSettings/SetSleepMode/SetButtonAction
  (polkit `power.settings`), mem_sleep + tmpfiles keep, logind drop-in
  `90-unix-guardian.conf` + reload, inhibitors reported (ryoku-clamshell
  holds the lid here). `ug power sleep`, TUI System → Sleep,
  `scripts/sleep-e2e.sh` (CI). Not run against the real logind yet.
- Battery: agent records on_ac + screen_on; Monitor.BatteryReport; three
  health methods (gauge 88.9 %, measured 82 %, cycles 74–87 % on the real
  history). `ug battery report`, TUI under protection, window chart.
- Driver Fix: flatpak-gl automatic, Limine via /etc/kernel/cmdline +
  limine-mkinitcpio, restart button, disabled button with the service's
  reason, hibernate-swap false positive fixed, driver.fix auth_admin_keep.
- Settings → Animations: Classic / Lively (app.css `[data-motion=lively]`).
- GPU tuning with nothing allowed shows "not supported".
**Waiting on the user:** reinstall (`sudo ./scripts/dev-install.sh`), then
try the Sleep tab (lid/power key change needs the new polkit action),
fan tab, battery tab, Driver Fix on the Flatpak finding, Lively motion,
and resizing in Hyprland.

## Before (2026-10-14): installer + Arch

The laptop runs **Arch Linux + Hyprland** now (was Parrot/KDE). `installer.sh`
(repo root) and its README sections are done in `c943006`. At first no
GitHub credentials (user asked to run `gh auth login`). webkit2gtk-4.1 was
not installed; the user was asked to run `bash ./installer.sh` themselves
(sudo). Installer ran fine on Arch (daemon + agent up); main pushed (gh auth
setup-git done). Test lab on Arch/Hyprland: 48 pass, 0 fail. Fixed a false
`nvidia-version-mismatch` (the open module's version line, "Open Kernel Module
for x86_64 615.71.09", read as "for"). README has an Arch row. The user must
re-run `bash ./installer.sh` to get the fix installed.
Fedora (user's 2026-10-12 report, before installer.sh: only ug-app copied to
~/.local/bin, so service/agent unreachable): window, CLI, TUI and test lab now
say "not installed, run bash ./installer.sh" when the unit files are missing
(`ug_core::missing`); dev-install.sh relabels for SELinux; the installer
removes stale ~/.local/bin copies and notes an NVIDIA card without its driver;
the dnf NVIDIA fix lists RPM Fusion + akmods MOK steps. Not run on Fedora yet
(no docker access here): next time the user boots Fedora, `bash ./installer.sh`
then `ug testlab`.
Release (2026-10-14, user asked for a beta for all distros): CI is green for
the first time (cargo fmt over everything, clippy clean, CI containers got
PyGObject + diffutils). The release tarball's install.sh now installs
WebKitGTK/AppIndicator and relabels SELinux. Tag push was refused by auto
mode, so the user runs: `git tag -a v0.1.0-beta.1 -m ... && git push origin
v0.1.0-beta.1`; release.yml makes a draft; publish with `gh release edit
v0.1.0-beta.1 --draft=false` after checking it. Repo stays private.
2026-10-14: tag re-pushed on `8d6bb4f` (deb name without "~": GitHub turned
it into "." and broke SHA256SUMS). Draft checked: sums OK, all three
signatures verify with PUBLIC_KEY, tarball installs into a scratch root.
User publishes with `gh release edit untagged-04fde8244b1ae9495c46 --draft=false`.
v0.1.0-beta.1 is published. The user's tarball install.sh stopped on the
laptop's source-folder install and told them to run `installer.sh`, which the
tarball doesn't have (it meant the repo's). Fixed: packaging/install.sh now
replaces a source-folder install itself; `scripts/tarball-install-test.sh`
(CI job `tarball`, and on the real tarball in release.yml) checks contents,
install/uninstall and the takeover in scratch roots. beta.1's tarball still
has the old install.sh; a beta.2 (scripts/release.sh 0.1.0-beta.2, user pushes
the tag) would ship the fix.
2026-10-14 ~11:00, STOPPED (user closed the app): user asked to "complete the
update section". Found: check works (repo is public now); download → verify
→ install.sh → rollback has never run end to end. Next: scripts/update-e2e.sh
(daemon --session --sysroot, agent, file:// feed with a tarball from
stage.sh signed by a test key via `sign keygen --no-password` and
update.toml public_key in the sysroot). Open question to the user, no answer
yet: let installer.sh (development) installs update from the signed release
tarball too (recommended; install.sh now takes them over) or keep showing
the git pull command. No code written for this yet.
2026-10-14 11:44: user ran release.sh 0.1.0-beta.2 and pushed (tag on
`fa68d03`). Draft checked: release.yml green incl. the tarball test, sums OK,
3 signatures OK with PUBLIC_KEY, binaries say 0.1.0-beta.2, install.sh has
no installer.sh reference. User publishes with
`gh release edit v0.1.0-beta.2 --repo AGlegendery/Unix-Guardian --draft=false`.

## Where we are

- Repo: `/home/aglegend/SSDBkup/IMPORTANT/ProgramingTable/unix-guardian`
  (Parrot Linux, KDE Plasma on Wayland, i5-12450HX + RTX 3050 laptop + Intel iGPU).
- **Current branch: `p5-gpu`.** It carries P5 (GPU tuning), the tray,
  branding, the card switch, Play Time, tracked apps and Steam launch options
  (P6.1). Not merged into `main` yet; the user gave a standing go-ahead for
  commits and merges. Nothing is pushed (no GitHub credentials on the laptop).
- Every other branch (`main`, `p2-game-mode` … `p6-overlay`, `pt-tui`,
  `ui-shell`, `monitoring`) is fully contained in `p5-gpu`; `p5-gpu` is 38+
  commits ahead of `main`. Keep the old branches until the user pushes.

## Work queue (from the user, in order)

1. ✅ Steam launch options (P6.1) — `299399d` … `f154e75`.
2. ✅ Global shortcuts through the XDG GlobalShortcuts portal (P6.3) — `9ddb1c1`.
3. ✅ Beta packaging, local only (P6.6) — `f9aa5ac`. `./scripts/build-packages.sh`
   → `dist/` (.deb, install.sh tarball, arch/PKGBUILD + source). Not
   published, and must not be without the user's word.
4. ✅ Settings page, window + TUI (P1.7) — `17ea6ec`. Agent1.Settings
   (`settings.json`: notifications, scan interval, close window on game),
   `ug settings`; window page with language, tray, shortcuts, About; TUI page 12.

5. ✅ Driver troubleshooter, first cut (P8) — `7d18b5a`, `fdc602a`, `5b5d55b`.
   The user calls it one of the most important features. Crate `ug-driver`
   (inventory from files, 21 TOML rules in `data/driver-rules/`, fixes,
   undoable executor), `Daemon1.Driver` (polkit `org.unixguardian.driver.fix`,
   password every time; root passes), `ug diagnose`, `ug driver
   plan|fix|history|rollback`, window page, TUI page 9, `scripts/driver-e2e.sh`.

6. ✅ Shortcut recorder (asked after the queue): pick the global shortcuts
   keys in the window (Settings → Global shortcuts, "Change") and the TUI
   (Overlay → Global shortcuts, `c`), and `ug shortcut keys <id> <keys>`.
   The agent sets them in KDE's kglobalaccel directly. Also fixed: the agent
   never found the portal (zbus asked for `Version`, the property is
   `version`), so `ug shortcut on` always said "not offered".
   `scripts/shortcuts-e2e.sh` runs it against a fake KDE desktop.
   Also `dev-install.sh` now closes the old window and restarts the user's
   agent (`3138f00`), because the user saw no troubleshooter page after
   reinstalling: the old window and agent were still running.

7. ✅ **Driver Fix** (M1–M5 done; see 04-status) (asked 2026-10-03; the user's single most important
   feature, and **not only for gamers**). Plan, highest impact first, one
   commit per milestone:
   - **M1 ✅** (`1bebb27`) diagnosis beyond the GPU: a `category` on rules
     and findings (graphics, wifi, bluetooth, audio, input, camera,
     printing, sleep, firmware); new facts in `ug-driver` (`hardware.rs`:
     PCI/USB devices without a driver, rfkill, `/proc/asound/cards`, SOF
     firmware, i2c-HID touchpads (ACPI PNP0C50), UVC/IPU6 cameras, USB
     printers (class 07, IPP-over-USB 07/01/04), CUPS/bluetooth/avahi
     units, `/sys/power/mem_sleep`, swap vs RAM for hibernate, missing
     firmware from the kernel log (`Env.kernel_log`, given by the caller)
     plus file checks (iwlwifi loaded, no ucode), Debian non-free-firmware
     component); rule files per category in `data/driver-rules/`; the
     window and TUI group findings by category.
     Automatic fixes in M1 only where undo is exact: rfkill unblock
     (sysfs write, undo writes back), enable units `--now` (bluetooth,
     cups), our own `modules-load.d`/`modprobe.d` files.
   - **M2 ✅** snapshot layer (8.5): snapper / timeshift / btrfs / LVM detection,
     light config backup always, `ug driver snapshots`.
   - **M3 ✅** distro adapter + package installs (8.4), only with a snapshot
     (golden rule), argv only, live log; undo removes what was installed.
     Firmware, sof-firmware, bluez, cups, ipp-usb, sane-airscan, NVIDIA.
   - **M4 ✅** boot-parameter editor (8.7): GRUB, systemd-boot, kernelstub;
     idempotent, backup, diff-empty round trip. Fixes: nomodeset,
     ppfeaturemask, mem_sleep_default=deep, psmouse.synaptics_intertouch.
   - **M5 ✅** check after restart (8.6): pending-verify per boot id, re-diagnose,
     Keep / Undo in window, TUI and `ug driver verify`; recovery kit.
   Same rules as before: GUI + TUI + CLI for everything, plan shown in
   full before running, polkit, backup, undo from a text console.

**The queue is done.** Release binaries and `dist/` were rebuilt after the
last commit; the user still has to reinstall (sudo) to try them for real.

After the queue: see "Next steps" in `04-status.md` (real-install checks that
need sudo, pushing, P7+).

## Current work (2026-10-04 afternoon): five UI asks from the user

1. ✅ Internet use in Ping → Internet control (hour/today/7/30 days, a day
   hour by hour): `Monitor.TrafficBuckets`, `ug traffic --day`.
2. ✅ Dashboard game mode button + any profile on by hand
   (`Profiles.SetGameMode`, `ug profile on|off`, TUI `g`).
3. ✅ Monitoring: hour/day/week averages with charts for everything the
   Live view shows (history table needs the extra columns).
4. ✅ Playtime: charts by game per day, day sessions, hours of day, figures.
5. ✅ Profiles: screen on, do not disturb, own profiles, pause apps, on at once (was: part done
   in 2; still: new options, custom profiles, pause_apps editing).

## Test lab (2026-10-04 evening) ✅

«آزمایشگاه تست»: window page after Bug report, TUI page 11, `ug testlab`.
Crate `ug-testlab` (engine, undo journal, scrubber, CPU bench);
`scripts/testlab-e2e.sh`. Commits `529a895` … (see git log). Release
binaries and the window were rebuilt afterwards. Still to do: run it on
the real machine after the reinstall and read the report together.

## Bug sweep (2026-10-04 evening) ✅

Asked after the Test lab. Fixed, each with a test that failed first:
game threads kept nice -10 after game mode ended mid-game (ug-tune
ThreadNice undo); paused apps stayed frozen if the agent died
(paused.json) or when game mode was turned off by hand; power jobs,
conditional power actions and net rules stayed in force in memory when
saving them failed; window Dashboard/GPU-tuner left a sample listener,
a subscription or a GPU-waking poll behind on fast page changes.
net-e2e's three tc checks fail on this machine before and after (no
`tc` on PATH outside sbin); not caused by the sweep.

## Profile picker (2026-10-05) ✅

Many profiles broke the picker; now it scrolls sideways at its old size with
fades, arrows and a thin bar, window and TUI (see 04-status "Profile
picker"). Release binaries rebuilt; the user has to reinstall.

## Batch after the picker (2026-10-05) ✅

Traffic day/week/month + top apps + reset, Test lab fixes from the first
real report, the Persian rename, G loader everywhere and the splash. See
04-status "Batch after the picker". Release rebuilt; user must reinstall.

## Incident 2026-10-04 (top priority, read 05-findings "Incident")

The card switch (`edd4115`) wrote NVIDIA offload variables into
`~/.config/environment.d`; at the next login KWin/plasmashell rendered on
the RTX 3050 and KDE broke (black Wayland, garbled X11). Another session
repaired the machine (file moved to `~/.config/environment.d.disabled/`,
Plasma config restored from `~/kde_backup_20261004_085231`, pre-restore
copy in `~/kde_before_restore_20261004_100905`; never touch those or the
restored config). Fixed in code, one commit per task:
1. ✅ `db263c4` switch is per program only (`~/.config/unix-guardian/default-gpu`,
   `ug run`, `ug gpu app <name>` → `.desktop` copy with PrefersNonDefaultGPU).
2. ✅ `3001a61` agent moves an old environment.d file to
   `~/.config/unix-guardian/retired/`, unsets its variables, notifies.
3. ✅ notes (this, 04-status, 05-findings, 02-decisions, CLAUDE.md).
4. ✅ `4ebaef1` overlay's session-wide `MANGOHUD=1` option removed (same pattern).
5. ✅ `1970c79` guard test over the whole workspace + unit tests.
6. ✅ `b2ba4b8` Set up had not failed: no progress was shown while the
   ~300 MB runtime downloaded over a flaky link, and the machine was shut
   down mid-download (05-findings). Now a pty shows Flatpak's real
   progress; `bc81d0e` G gauge + bar in the window, bar in the TUI. The
   Flatpak is installed for the user now.
Release binaries and the window were rebuilt after `bc81d0e`; they have no
session-wide writer (checked with grep). The installed ones still do
(`/usr/local/bin/ug`, `ug-app` the GPU file, `ug-agent` the MangoHud one):
until the reinstall, do not touch the dashboard card switch. `dist/` is
stale (built before the fix); rebuild it before anything is shared.
**Still needed from the user:** reinstall (release build) and a real
logout/login on Plasma Wayland and X11, then `ug gpu default` and the
dashboard switch. Not "done" before that.

After reinstalling, the window showed "Signature mismatch": an autostarted
agent from before the update kept the bus name. Fixed (new agent takes over
from an outdated one; clear message in CLI and window). Seen on the real
gpu-screen-recorder for the first time: `--list-capture-options` gives
`eDP-1`, `region`, nine `/dev/video0|…` camera modes and `portal`;
cameras and duplicates are now left out (test with the real lines). Also
fixed: switching pages kept the shared scroll area's position (a new page
opened half way down, stuck in WebKitGTK); each page now opens at its top.

## Release status (2026-10-04, read first)

- All branches were pushed to github.com/AGlegendery/Unix-Guardian (still
  PRIVATE). Secrets MINISIGN_KEY / MINISIGN_PASSWORD are set; the key lives
  in `~/.config/unix-guardian-release/` (never commit it). Public key is in
  `ug-core/src/update.rs` (`3c10bd0`).
- First CI run (on `0df99f2`) was red: TUI test fixture (fixed `d9ccae1`),
  cargo-deny wildcards (fixed `42a2a1d`), and `cargo fmt --check` (1907
  hunks: the code was never formatted; needs rustfmt, the user must
  `sudo apt install rustfmt rust-clippy`, then `cargo fmt --all` + clippy).
- Waiting on the user: (1) permission for `git push` (Claude Code's auto
  mode denied push and git filter-repo), (2) yes/no on scrubbing the old
  disk UUID `ba83246d-…` and MAC `b8:1e:a4:…` from history with
  git filter-repo (replacements: the fakes already in HEAD) before the repo
  goes public. A venv with filter-repo was made in the session scratchpad.
- 2026-10-04 (later session): the self-update is done in all three front
  ends (`ug update --install|--rollback`, window Updates page, TUI page
  11, daily-check switch in both Settings pages; see 04-status
  "Self-update"). The questions above plus "what changes do you want?"
  were asked again in the thread; no answer yet. Next: the user's changes.
- **2026-10-04 09:55 the user said: do NOT make the repo public and do not
  release yet; they have more changes first.** Later, when they say so: push, make public, `git push origin v0.1.0-beta.1` (tag from
  `scripts/release.sh` is not made yet; version already 0.1.0-beta.1, so
  tag with `git tag -a v0.1.0-beta.1`), check the draft release, publish.

## Test-lab reports fixed (2026-10-11) ✅

From the user's Hyprland, XFCE and GNOME test-lab reports (all on `main`,
pushed; release binaries and the window rebuilt): ping 0/0 (the lab now
starts ping early and waits), X11 recording (`ug_core::capture`: the
per-user Flatpak cannot record a monitor on Intel/AMD X11, so automatic
records the active window; see 05-findings), "battery none" wording, the
route's interface kind from sysfs, and **Set up for shortcuts on GNOME and
XFCE** (`ug-agent/src/deskkeys.rs`: gsettings custom keybindings / xfconf
`/commands/custom/<accel>`, undo removes only ours, record in
`desktop-shortcuts.json`; `ug shortcut setup [undo]`, window Settings card,
TUI Overlay → Global shortcuts last row; `scripts/shortcuts-e2e.sh` checks
XFCE with a fake xfconf-query). Not yet tried on a real GNOME/XFCE session
or a real X11 recording: the user should reinstall and re-run the lab there.
README "Tested on": XFCE row still says recorder being fixed until retested.

## In progress

The recorder (P7): first part done (see 04-status "P7 Recorder"). Next:
try it with the real gpu-screen-recorder (`ug record setup`), then the clip
library, compression, Telegram (7.4). The hibernation swap setup is done. Driver Fix M1–M5 are done and committed; release binaries built.
Next for Driver Fix: try it on the real laptop after reinstalling
(`ug diagnose`, `ug driver snapshot`), more rules (roadmap wants 35+ for
graphics alone), a TUI text field for kernel parameters, a driver-version
picker. M4 done:
`bootparams.rs` (GRUB/systemd-boot/rEFInd/kernelstub, add+remove is
diff-empty), fixes nomodeset/ppfeaturemask/mem_sleep_default/intertouch,
`ug driver param add|remove`, window card; TUI shows the command. M2+M3 done:
`snapshot.rs` (snapper > Timeshift > ZFS > plain read-only Btrfs snapshot
into /var/lib/unix-guardian/snapshots; LVM never), light config backup
always, `packages.rs` (apt/pacman/dnf/zypper argv, undo removes only what
the install added), `ug driver snapshot`. The laptop has btrfs-progs and
root on /@, so package fixes run there with a Btrfs snapshot. M1 done: 30 new rules
(`data/driver-rules/{wifi,bluetooth,audio,input,camera,printing,sleep,firmware}.toml`),
`ug-driver/src/hardware.rs`, live fixes (rfkill, units --now, modules-load.d +
modprobe), `Daemon1.Driver.FirmwareLog`, category chips in window and TUI.
On the laptop it found one new real thing: 512 MB swap vs 28 GB RAM (hibernate). Next candidates (ask or follow `04-status.md` "Next steps"): the
real-desktop checks below, merging `p5-gpu` into `main`, P7 (recording).

## Done but not tried on the real desktop

- Global shortcuts (P6.3) and the recorder: only the fake KDE desktop so
  far. Check after reinstalling: `ug shortcut on`, confirm KDE's dialog,
  press Meta+Shift+G, then Settings → Global shortcuts → Change, press new
  keys; System Settings → Shortcuts should show them under Unix Guardian
  (component `unix-guardian`). Unknown: whether KWin lets Meta+… reach the
  window while recording (the TUI has Tab for Meta; the CLI always works).
- Packages (P6.6): the .deb was installed and purged in a Debian container
  without network (`--force-depends`, no systemd there), install.sh into a
  scratch root, the PKGBUILD's `package()` and `--printsrcinfo` in an Arch
  container (no fakeroot there, so no real `makepkg`). Never installed on a
  real system. AppImage (roadmap 6.6) not done; `ug diagnose` not done.
- Settings: only tests, e2e and the browser preview. Theme and "start at
  login" from roadmap 1.7 are deliberately not offered (one theme; turning
  the agent off at login would break D-Bus activation).

- Driver troubleshooter: `ug diagnose` ran on the laptop (3 real findings:
  video memory not kept across sleep, fbdev off, Wayland on driver 550).
  No fix has been applied on the real machine. Automatic fixes are only our
  own `/etc/modprobe.d/unix-guardian-*.conf` files + units + initramfs
  rebuild via `systemd-run`; package installs are commands only until the
  snapshot layer (8.5) exists — the roadmap's golden rule. Not built yet:
  8.4 distro adapter (package installs), 8.5 snapshots, 8.7 boot-parameter
  editor, pending-verify after reboot with auto-undo, a driver-version
  picker; rules number 21 of the 35 the roadmap wants for release.

## For the user to run (needs sudo, at the device)

```bash
sudo ./scripts/dev-install.sh
```
(it now closes the old window and restarts the agent itself), then try
`ug shortcut on` (KDE asks to confirm the keys) and the recorder.

## Build, test, install

```bash
cargo build --workspace
cargo test --workspace
./scripts/test-bus.sh ./scripts/smoke.sh
./scripts/test-bus.sh ./scripts/shortcuts-e2e.sh  # fake KDE portal + kglobalaccel
./scripts/test-bus.sh ./scripts/agent-e2e.sh      # stop the installed agent first:
systemctl --user stop ug-agent                    # (it sees fake games and tunes the real machine)
cd app && npm run check                            # window type check
cd app && npx tauri build --no-bundle              # real window binary
cargo build --release --workspace                  # before asking the user to reinstall
```

The user installs with `sudo ./scripts/dev-install.sh` (copies
`target/release` and `app/src-tauri/target/release` as they are, so build
release first). **sudo and polkit steps need the user at the device: list
them in the reply instead of blocking.**

## Gotchas that bit before

- Never run anything that can reach the real service manager or write real
  `/sys` during tests: `--session`, `--sysroot`, private buses
  (`scripts/test-bus.sh`), scratch `XDG_*` dirs. Afterwards check `ug playtime`
  for test entries and `journalctl` for polkit prompts.
- Never wake a runtime-suspended dGPU for telemetry; daemon ~6 MB idle.
- The UI must never animate endlessly (WebKitGTK at 60 fps = a full core).
- `rustfmt`/`clippy` are not installed; CI (never run yet) has them. Keep
  lines ≤ 100 by hand.
- Bugs: failing test first, confirm it fails without the fix.
- Answer the user in Persian; code, comments, commits, notes in English.
- The previous builder sessions died (went offline seconds after reconnecting,
  once a usage limit). Commit small and often, and keep this file current.
