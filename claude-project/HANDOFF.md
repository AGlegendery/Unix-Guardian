# HANDOFF — read this first

The short version of where the work is, so a new session (Claude Code on the
user's laptop, possibly started cold after the previous one died) can resume
in minutes. **Update this file after every finished item and commit it.**
Details live in `04-status.md` (per-task truth), `02-decisions.md`,
`05-findings.md`, `07-conventions.md`, `03-architecture.md`.

Last updated: 2026-10-03 (shortcut recorder).

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

7. ⏳ **Driver Fix** (asked 2026-10-03; the user's single most important
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
   - M4 boot-parameter editor (8.7): GRUB, systemd-boot, kernelstub;
     idempotent, backup, diff-empty round trip. Fixes: nomodeset,
     ppfeaturemask, mem_sleep_default=deep, psmouse.synaptics_intertouch.
   - M5 check after restart (8.6): pending-verify per boot id, re-diagnose,
     Keep / Undo in window, TUI and `ug driver verify`; recovery kit.
   Same rules as before: GUI + TUI + CLI for everything, plan shown in
   full before running, polkit, backup, undo from a text console.

**The queue is done.** Release binaries and `dist/` were rebuilt after the
last commit; the user still has to reinstall (sudo) to try them for real.

After the queue: see "Next steps" in `04-status.md` (real-install checks that
need sudo, pushing, P7+).

## In progress

Driver Fix M4, boot-parameter editor (see item 7 above). M2+M3 done:
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
