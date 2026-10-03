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
2. ✱ **Global shortcuts through the XDG GlobalShortcuts portal (P6.3).**
   See "In progress" below.
3. ○ **Beta packaging (P6.6)** — build packages **locally only**
   (PKGBUILD / .deb / install.sh). **Never publish or upload anything.**
4. ○ **Settings page**, in the window **and** the TUI in the same change
   (project rule: every page exists in both).

After the queue: see "Next steps" in `04-status.md` (real-install checks that
need sudo, pushing, P7+).

## In progress: global shortcuts (P6.3)

Done (uncommitted at first, check `git log` for "shortcut"):
- `crates/ug-agent/src/shortcuts.rs`: portal client (CreateSession,
  BindShortcuts, Activated / ShortcutsChanged signals, Registry.Register with
  app id `unix-guardian`), state in `~/.config/unix-guardian/shortcuts.json`
  (off until the user turns it on, because KDE shows a dialog), fallback text
  with `ug shortcut <id>` commands where there is no portal. Three actions:
  `open-window` (LOGO+SHIFT+u), `toggle-overlay` (LOGO+SHIFT+o),
  `toggle-gamemode` (LOGO+SHIFT+g).
- `core.rs`: game mode by hand (`toggle_gamemode`, `by_hand`, uses the
  `balanced` profile; "off by hand" lasts until no game runs). Test written.
- `overlay.rs`: `Overlay::toggle()` (forced show/hide until the next game
  start/stop). Test written.
- `ug-proto`: `Agent1.Shortcuts` proxy (State / SetEnabled / Run),
  `ShortcutsState`, `ShortcutInfo`.

Remaining when this file was written:
- agent: serve `org.unixguardian.Agent1.Shortcuts` in `iface.rs`, create
  `Shortcuts` in `main.rs`, act on pressed ids in the main loop (open window
  the way the tray does, overlay toggle, game mode toggle + reconcile now),
  `shutdown()` on exit, fill `gamemode_by_hand`.
- CLI: `ug shortcut [status|on|off|<id>]`.
- window + TUI: a Shortcuts card (probably on the Overlay page, later the
  Settings page) with on/off, status, keys, problem text.
- agent-e2e checks (no portal on a private bus → status `unavailable`,
  `ug shortcut toggle-gamemode` works).
- Try on the real KDE session (installed agent must be rebuilt first).

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
