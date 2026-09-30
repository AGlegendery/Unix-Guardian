# 07 — Conventions

How this codebase has been written so far. Follow them unless there is a
concrete reason not to, and say so when deviating.

## Principles

1. **Measure on real hardware before believing it.** Every finding in
   05-findings.md came from running the code, and several contradicted what
   looked obviously right. A feature is not "done" until it has run on a real
   machine, or the status says plainly that it has not.
2. **Never put facts from memory into data.** Steam App IDs are checked against
   the Steam store API; GPU names come from `pci.ids`; sysfs layouts come from
   captured fixtures. A hand-written table was wrong on its first check.
3. **Honest UI and reports.** Never claim a change that did not happen, never
   offer a control that cannot work (NVIDIA has no voltage control on Linux —
   say so). When a feature is unavailable, show it disabled **with the reason**
   (`GpuCapabilities.notes`, `GameModeReport.skipped`).
4. **Everything temporary is reverted — including after a crash.** Record the
   undo *before* the change, persist it atomically, restore the user's own
   previous value (not a default), keep failed restores for a retry.
5. **Least privilege.** Only the daemon is root. Anything a user may do to their
   own processes (affinity, SIGSTOP/SIGCONT, reading their files) happens in the
   agent. Every privileged D-Bus method checks a polkit action; the daemon also
   checks that PIDs belong to the caller.
6. **Do not disturb the machine.** No periodic work when nobody is watching
   (the sampling loop parks); never wake a sleeping GPU for telemetry; release
   big libraries (NVML) when idle. The resource budget is a requirement from the
   user, not a nicety.
7. **Development must never touch the real system.** `--session` and
   `--sysroot` runs disable the service manager and skip polkit; tests use
   fixtures, scratch dirs and private buses.
8. **Every feature has a CLI equivalent.**

## Code

- Rust, edition 2021, `rust-version = 1.85`; stable toolchain; `rustfmt.toml`
  sets `max_width = 100`. Clippy must pass with `-D warnings` (CI).
- **Doc comments explain *why*** (the constraint, the hardware quirk, the bug it
  prevents), not *what* the next line does. Module docs state the design and
  its limits. Keep the existing density — this is a reference codebase.
- Hardware and `/proc` readers take a **`SysRoot`** (`SysRoot::real()` or a
  fixture); never hard-code `/sys` or `/proc`.
- **Missing readings are not errors.** Readers return `Option`; the D-Bus
  boundary uses sentinels (NaN / 0 / "") with `Option` accessors in `ug-proto`.
- No `unwrap()` on anything that depends on the machine; `unwrap()` is fine in
  tests. Mutex poisoning is recovered (`unwrap_or_else(|e| e.into_inner())`).
- Never hold a lock that a long operation may hold from an `async` fn — use
  `spawn_blocking` for blocking work and atomics for status properties.
- D-Bus interfaces only grow; a breaking change is a new interface name.
- New polkit actions arrive in the same change as the first method that checks
  them (a test enforces that code and policy file match).
- Dependencies: prefer the standard library and small, well-known crates; check
  licences stay GPL-3-compatible (`deny.toml`).

## Tests

- Test names are sentences describing behaviour:
  `a_suspended_gpu_is_reported_asleep_without_being_queried`.
- **For every bug: write the failing test first, see it fail, then fix.** When
  adding a test for an existing fix, temporarily undo the fix to confirm the test
  catches it.
- Hardware logic is tested against `ug-testfs` trees and captured fixtures
  (`ug_testfs::fixture("dev-laptop-rtx3050")`), so tests run on any machine and in
  CI without root.
- Tests that start real processes must not race (see the ETXTBSY finding) and
  must clean up children.
- End-to-end checks run inside `dbus-run-session` with scratch XDG directories
  and, for game mode, a scratch sysroot (`scripts/*-e2e.sh`).

## Git

- Work on a feature branch per phase (`p2-game-mode`, next `p3-…`); `main` stays stable.
  Commit only with the user's go-ahead (given for this work stream so far).
- Commit messages: a short imperative subject naming the roadmap tasks
  (`… (P2.5, 2.6, 2.8)`), then a body explaining what and *why*, what was
  verified and how, and what is known not to work yet. Previous commits end with
  a `Co-Authored-By:` line for the Claude model that wrote them.
- Never commit generated or private data: the knowledge-pack `source/` folder is
  git-ignored; fixtures are scrubbed (see `capture-fixture.sh`).

## Working with the user

- The user writes in **Persian**; answer in Persian. Code, comments, commit
  messages and these docs are in **English**.
- The user is the only person running commands on the real machine. Give **one
  command per code block**, say what it changes, and warn before anything that
  needs `sudo` or touches the real system (CPU governor, sysctls, services,
  bootloader, drivers).
- Anything irreversible or outward-facing (pushing, publishing, installing
  drivers, editing boot entries) needs an explicit yes first.
