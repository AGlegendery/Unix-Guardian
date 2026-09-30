# Unix Guardian — notes for Claude

Free, open-source (GPL-3.0-or-later) Linux gaming utility in Rust: a root
daemon (`ug-daemon`, system bus), a per-user agent (`ug-agent`, session bus),
a CLI (`ug`) and, later, a Tauri UI. Solo developer; the user writes in Persian
(answer in Persian; code, comments and commits in English).

**Read `claude-project/` first** — especially `04-status.md` (what exists and
what's next), `02-decisions.md`, `05-findings.md` and `07-conventions.md`.

## Commands

```bash
cargo build --workspace
cargo test --workspace
dbus-run-session -- ./scripts/smoke.sh
dbus-run-session -- ./scripts/agent-e2e.sh
dbus-run-session -- ./scripts/gamemode-e2e.sh
```

Development daemon (no root, never touches real services):
`cargo run -p ug-daemon -- --session` with `ug --session …` and
`ug-agent --daemon-session`.

## Rules that bit before

- Never run a daemon that can reach the real service manager or real `/sys`
  writes as part of testing; use `--session`/`--sysroot`, private buses and
  scratch `XDG_*` dirs. Check `journalctl` for polkit prompts afterwards.
- Never wake a runtime-suspended GPU for telemetry; keep the daemon at ~6 MB idle.
- Bugs get a failing test first; confirm it fails without the fix.
- `rustfmt`/`clippy` are not installed on the dev machine; CI runs them.
- Commit only with the user's go-ahead, on the phase branch (`p2-game-mode`).
