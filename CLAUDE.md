# Unix Guardian — notes for Claude

Free, open-source (GPL-3.0-or-later) Linux gaming utility in Rust: a root
daemon (`ug-daemon`, system bus), a per-user agent (`ug-agent`, session bus),
a CLI (`ug`) and, later, a Tauri UI. Solo developer; the user writes in Persian
(answer in Persian; code, comments and commits in English).

**Read `claude-project/HANDOFF.md` first** (current branch, work queue,
exactly where the last session stopped; update it after every finished item).
Then read `claude-project/` — especially `04-status.md` (what exists and
what's next), `02-decisions.md`, `05-findings.md` and `07-conventions.md`.

## Commands

```bash
cargo build --workspace
cargo test --workspace
./scripts/test-bus.sh ./scripts/smoke.sh
./scripts/test-bus.sh ./scripts/agent-e2e.sh
./scripts/test-bus.sh ./scripts/gamemode-e2e.sh
./scripts/test-bus.sh ./scripts/power-e2e.sh
./scripts/test-bus.sh ./scripts/shortcuts-e2e.sh
cd app && npm run check && npm run dev     # UI in a browser, recorded data
cd app && npx tauri build --no-bundle      # the real window: app/src-tauri/target/release/ug-app
```

Development daemon (no root, never touches real services):
`cargo run -p ug-daemon -- --session` with `ug --session …` and
`ug-agent --daemon-session`.

## Rules that bit before

- Never run a daemon that can reach the real service manager or real `/sys`
  writes as part of testing; use `--session`/`--sysroot`, private buses and
  scratch `XDG_*` dirs. Check `journalctl` for polkit prompts afterwards.
- Never wake a runtime-suspended GPU for telemetry; keep the daemon at ~6 MB idle.
- Unix Guardian is installed on the dev laptop (daemon + user agent). Run the
  e2e scripts through `scripts/test-bus.sh` (plain `dbus-run-session` can
  D-Bus-activate the installed agent with the real XDG dirs), and until the
  installed agent is rebuilt with `UG_TEST_RUN` support, stop it around test
  runs (`systemctl --user stop ug-agent`): it sees the fake games and tunes the
  real machine. Afterwards check `ug playtime` for test entries.
- The UI must never animate endlessly: WebKitGTK repaints at 60 fps and one
  pulsing dot measured as a full CPU core.
- `dev-install.sh` copies `target/release` and `app/src-tauri/target/release`
  as they are: run `cargo build --release --workspace` (and the Tauri
  build) before asking the user to reinstall, or they install stale binaries.
- Bugs get a failing test first; confirm it fails without the fix.
- `rustfmt`/`clippy` are not installed on the dev machine; CI runs them.
- Commit on the current phase branch (next: `p3-…`), merged into `main` when a
  phase lands. The user has given a standing go-ahead for commits and merges.
