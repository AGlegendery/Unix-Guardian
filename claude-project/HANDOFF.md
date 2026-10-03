# HANDOFF — read this first

The short version of where the work is, so a new session (Claude Code on the
user's laptop, possibly started cold after the previous one died) can resume
in minutes. **Update this file after every finished item and commit it.**
Details live in `04-status.md` (per-task truth), `02-decisions.md`,
`05-findings.md`, `07-conventions.md`, `03-architecture.md`.

Last updated: 2026-10-03.

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
4. ✱ **Settings page**, in the window **and** the TUI in the same change
   (project rule: every page exists in both).

After the queue: see "Next steps" in `04-status.md` (real-install checks that
need sudo, pushing, P7+).

## In progress: Settings page

Not started yet. Window page + TUI page in the same change.

## Done but not tried on the real desktop

- Global shortcuts (P6.3): KDE's portal answers `version 1`, but binding
  (KDE shows a confirmation dialog) needs the installed agent rebuilt and
  reinstalled. Check: `ug shortcut on`, confirm the dialog, press Meta+Shift+G,
  `ug shortcut` shows the keys; `ug shortcut off`.
- Packages (P6.6): the .deb was installed and purged in a Debian container
  without network (`--force-depends`, no systemd there), install.sh into a
  scratch root, the PKGBUILD's `package()` and `--printsrcinfo` in an Arch
  container (no fakeroot there, so no real `makepkg`). Never installed on a
  real system. AppImage (roadmap 6.6) not done; `ug diagnose` not done.

## Build, test, install

```bash
cargo build --workspace
cargo test --workspace
./scripts/test-bus.sh ./scripts/smoke.sh
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
