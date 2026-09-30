# `ug tui` — screen design (for approval before any code)

Status: **draft for the user's approval** (roadmap T.0). Nothing is built yet.

## Principles

1. **Same map as the window.** The same pages, in the same order, with the same
   names. Someone who knows one interface finds everything in the other.
2. **Three fixed zones, always in the same place:** the menu on one side, the
   page in the middle, and a **key bar at the bottom that always shows what you
   can press right now**. Nothing to memorise.
3. **Mouse and keyboard, both complete.** Click a menu item, a tab, a button, a
   switch; scroll with the wheel; hover shows a hint in the key bar. Everything
   also works with keys only.
4. **Dangerous actions ask twice**, exactly as in the window (shutdown: press,
   then confirm within 5 s).
5. **Light:** redraws only when a number changes or you press something, and
   nothing animates. Target below 15 MB RAM.
6. **Honest:** an unavailable feature shows the reason (and a "Set up" action
   where one exists), unbuilt pages say so.

## Layout at 100×30 and larger

```text
┌ Unix Guardian ─────────────────────────────────────────────────────── ● service ● agent ┐
│ ▌Dashboard     │ Dashboard                                                     Live ●      │
│  Game mode     │ ╭ Graphics card ── in use ╮╭ Processor ────────╮╭ Memory ──────╮╭ Network ─╮│
│  System control│ │ 850 MHz                 ││ 51°  cool   32 %  ││ 22.3/27.1 GB ││ ↓ 1.2 MB/s││
│  Monitoring    │ │ Intel UHD Graphics      ││ ████████░░░░░░░░  ││ █████████░░  ││ ↑ 48 kB/s ││
│  Ping stability│ │ NVIDIA RTX 3050: asleep ││ i5-12450HX        ││ 82 %         ││ wlp8s0    ││
│  Overlay       │ ╰─────────────────────────╯╰───────────────────╯╰──────────────╯╰──────────╯│
│  Playtime      │ ╭ Game mode ──────────── Off ╮╭ Temperature, last 2 minutes ──────────────╮│
│  Recorder      │ │ Starts by itself when a    ││ 70°┤                                       ││
│  Drivers       │ │ game starts                ││ 60°┤      ⣀⡠⠤⠒⠉⠉⠑⠒⠤⣀                      ││
│  Bug report    │ │ Profile: Balanced          ││ 50°┤⠤⠤⠒⠉          ⠉⠒⠤⠤⠤⠤⠤⠤⠤⠤⠤ ━ Processor    ││
│  Updates       │ │ ☑ CPU at full speed        ││ 40°┤                                       ││
│  Settings      │ │ ☐ No clock drops …         │╰───────────────────────────────────────────╯│
│                │ ╰────────────────────────────╯╭ Cores ─ P ▇▇▅▃█▂▆▄  E ▃▂▅▁ ───────────────╮│
│ [فارسی]        │                                ╰───────────────────────────────────────────╯│
├────────────────┴──────────────────────────────────────────────────────────────────────────┤
│ ↑↓ menu  Enter open  Tab next section  1-9 jump  ? help  g window  q quit                 │
└───────────────────────────────────────────────────────────────────────────────────────────┘
```

- The **active menu item** is highlighted (purple, like the window) with a bar.
- **Charts** use Braille dots for fine detail; bars use block characters.
- Colours follow the window's palette (dark background, purple accent); on a
  16-colour terminal they fall back to the nearest ANSI colours.

## Layout at 80×24 (small terminal, SSH)

The menu collapses to a top row of short tabs, cards stack, and the page
scrolls (mouse wheel or PgUp/PgDn):

```text
┌ Unix Guardian ────────────────────────────────── ● service ● agent ┐
│ Dash │ Game │ System │ Monitor │ … (← → or click)                  │
├─────────────────────────────────────────────────────────────────────┤
│ Graphics card (in use)    850 MHz   Intel UHD Graphics              │
│   NVIDIA RTX 3050: asleep                                           │
│ Processor   51° cool   32 %  ████████░░░░░░░░   62 W                │
│ Memory      22.3 / 27.1 GB   82 %  █████████░░                       │
│ Network     ↓ 1.2 MB/s   ↑ 48 kB/s   wlp8s0                         │
│ Game mode   Off — starts by itself when a game starts               │
│ ...                                                         ▼ more  │
├─────────────────────────────────────────────────────────────────────┤
│ ←→ page  ↑↓ scroll  Enter select  ? help  q quit                    │
└─────────────────────────────────────────────────────────────────────┘
```

Below 60×20 it shows one line asking for a bigger window instead of a broken
layout.

## The other pages

**Game mode:** a profile selector row (`[Competitive] Balanced Quiet`), then a
list of switches, each with its one-line explanation under it; Space or a
click toggles. A second section lists your games with their profile; Enter
opens a small picker.

```text
│ Game mode                                    Off   Game selection: automatic │
│ Profile  ‹ Competitive ›  Balanced  Quiet                                    │
│ ─ Processor and memory ─ System and services ─ Graphics ─                    │
│ [■] CPU at full speed                                                        │
│     Sets the CPU governor and energy preference to performance.              │
│ [■] No clock drops between frames                                            │
│ [ ] Game on the fastest cores            (this CPU: performance cores 0–7)   │
```

**System control:** the same tabs as the window. Power command shows the four
actions as large buttons; the chosen one is highlighted, `Enter` arms, `Enter`
again within 5 s confirms (red), `Esc` cancels. Scheduling asks "in how long / at
what time / repeat" in a small form. The queue lists jobs with `x` (or a click)
to cancel.

**Monitoring:** tabs for temperature and power (three Braille charts stacked,
range switch `h`/`d`/`w`) and traffic (live speeds plus seven daily bars).

**Unbuilt pages:** a single card saying it comes in a later version.

## Keys (always shown at the bottom, context-sensitive)

| Key | Everywhere |
|---|---|
| ↑ ↓ / j k | move |
| ← → / Tab | next tab or section |
| Enter / Space | open, toggle, press |
| 1 … 9 | jump to a menu page |
| ? | full help overlay |
| g | open the graphical window instead (closes the TUI) |
| q / Esc | back, then quit |

Mouse: click anything clickable; wheel scrolls the area under the pointer;
right-click is not used (nothing hidden behind it).

## Persian in the terminal (needs your decision)

Terminals handle right-to-left text unevenly:

- **Konsole (yours), GNOME Terminal ≥ 3.34 and mlterm** can show Persian
  correctly when their BiDi option is on.
- **Many others (Alacritty, kitty, xterm, most SSH clients)** show Persian
  letters unjoined and in reversed order.

Options:

1. **English in the TUI by default; Persian as an option** that is offered
   when the terminal is known to support it (recommended: always readable).
2. **Persian by default**, with the layout mirrored (menu on the right) and
   Unix Guardian shaping and reordering the text itself for terminals without
   BiDi. More work, and some terminals may still misrender.

## Switching between the two interfaces

- In the window: a small "Terminal interface" button at the bottom of the
  sidebar opens your terminal with `ug tui` and closes the window completely.
- In the TUI: `g` opens the window and closes the TUI.
