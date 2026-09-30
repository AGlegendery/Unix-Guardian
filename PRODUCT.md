# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

(A Tauri 2 desktop window on Linux: WebKitGTK renders the UI, a Rust process
talks D-Bus to the daemon and agent. No browser, no server.)

## Users

Linux desktop users who want control over their own machine: people who play
games on Linux, and people who do not play at all but want to see and manage
their hardware (temperatures, power, what runs in the background, shutting
down on a schedule). Confirmed by the user, 2026-09-30: the ecosystem is for
gamers and ordinary Linux users alike, not gamers only.

Two situations bring them to the window:

- **Between sessions:** before or after playing or working, checking
  temperatures and load, choosing a game's profile, looking at playtime. During
  a game the window is closed.
- **When something is wrong:** lag, heat, a graphics driver that does not work.
  Many are Linux beginners; NVIDIA driver trouble is the classic case.

Primary language Persian (right to left); English second. Many users are in
Iran, where Telegram and some services need a proxy.

## Product Purpose

Unix Guardian is a free, open-source (GPL-3.0-or-later) Linux utility that
brings the feature set of the paid Windows app Phoenix Guardian to Linux:
automatic game mode, live monitoring, system power control, ping stability and
per-app network control, GPU tuning and fan curves, an in-game overlay
(MangoHud), recording with instant replay, playtime, and a graphics-driver
troubleshooter. Everything is free; there are no tiers.

Success: a user opens the window, understands the state of their machine and of
game mode at a glance, changes what they came to change, and closes it, without
the app ever costing them performance.

## Positioning

The only Linux tool that does all of this in one place, in Persian first, with
a native Rust core that measurably stays out of the way (daemon ~6 MB idle, 0%
CPU when nobody watches). It changes the system only temporarily and always
puts the user's own previous values back, including after a crash, and it
never pretends: a control that cannot work on this hardware is shown disabled
with the reason.

## Operating Context

- The window is optional and closes completely (no background WebView) when not
  in use, especially during games. The daemon and agent do the real work.
- Every UI action has a CLI equivalent (`ug …`); the UI never talks to sysfs
  directly, only to D-Bus.
- Tested hardware so far: Intel i5-12450HX laptop (P/E cores) with an NVIDIA
  RTX 3050 Laptop GPU that is usually asleep (hybrid graphics).
- Development uses private D-Bus buses and scratch directories; the UI must be
  usable against `ug-daemon --session`.

## Capabilities and Constraints

Built (backend): live telemetry (CPU load/temp/power/clocks/governor, per-core
load, GPU temp/load/power/clocks/VRAM/fan or "asleep", memory, network rates),
hardware inventory including Intel P/E cores and AMD V-Cache, GPU selector,
game detection (Steam, Lutris, Heroic, Bottles, GameMode, heuristics), three
profiles (competitive, balanced, quiet) plus user profiles, per-game profile
choice, reversible game mode (CPU performance, idle-state hold, priority,
memory sysctls, pausing update timers, pausing chosen apps, preferred cores),
playtime history.

Not built yet (must be shown honestly as not available, never faked): system
power control, network/ping control, GPU tuning and fan curves, overlay,
recording, Telegram, driver troubleshooter, updates.

Hard constraints: very low RAM and no lag during games or daily use (the user's
condition for using Tauri); no network fetches from the UI (fonts and assets
bundled); NVIDIA on Linux has no voltage control, so "undervolt" is never
claimed.

## Brand Commitments

- Name: Unix Guardian. Persian UI by default.
- **Visual design follows Phoenix Guardian's (the Windows app's) screenshots**:
  dark theme, purple accent, rounded cards, right-hand sidebar with the feature
  list, tiles for live readings, segmented controls and toggle grids. Decided by
  the user on 2026-09-30 after seeing an independent "circuit board" preview
  («ترجیح می‌دم همون سیستم داخل عکس‌ها باشه، یعنی دیزاین همون باشه»). This
  supersedes the earlier "independent identity" answer. The name and logo stay
  Unix Guardian's own; no Phoenix Guardian branding is used.
- Font: Vazirmatn, bundled (decided earlier).
- No logo, colour or domain exists yet (open question).

## Evidence on Hand

- Real telemetry from the development laptop through the daemon (`ug status
  --json`), and a captured fixture at
  `crates/ug-testfs/fixtures/dev-laptop-rtx3050`.
- A real installed game in Bottles ("Fears to Fathom – Ironbark Lookout").
- No users, testimonials, benchmarks or screenshots of the Linux app yet; do
  not invent any.

## Product Principles

1. Out of the way: the app's own cost is part of the product. Nothing polls,
   animates or renders when nobody is looking.
2. Honest state: show what is true on this machine, including "asleep",
   "not supported here" and "not built yet", with the reason.
3. Everything reversible: every change says what it changed and can be put
   back.
4. For everyone who owns the machine: plain words a beginner understands, with
   depth available to those who want it.
5. Persian first: right-to-left, Persian digits and wording are the primary
   design, not a translation.

## Accessibility & Inclusion

Right-to-left layout as the primary direction with a correct left-to-right
English mode; Persian digits in Persian UI; keyboard use and visible focus;
colour never the only carrier of state (temperatures and states also have
words); respects reduced motion.
