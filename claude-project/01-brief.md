# 01 — Project brief

## What Unix Guardian is

A free, open-source (GPL-3.0-or-later) **gaming utility for Linux**: automatic game
mode, GPU tuning and fan curves, system power control, live monitoring (temps,
power, FPS with 1%/0.1% lows), ping stabilisation and per-app network control,
an in-game overlay, recording with instant replay, playtime tracking, and a
graphics-driver troubleshooter that fixes the NVIDIA-on-Linux pain.

It is the **Linux counterpart of an existing Windows app, "Phoenix Guardian"
(فونیکس گاردین)**. The user's own words on how the two relate: *"all its
features, only on Linux instead of Windows — that is the most complete
description"*. It is a **separate codebase**, written from scratch for Linux;
nothing is shared with the Windows app except the feature set. (Whether the user
is also the Windows app's author has not come up — do not assume either way; it
matters for branding, see 09-open-questions.md #12.)

- Name: **Unix Guardian** (chosen by the user; I pointed out it may suggest
  BSD/macOS support; they kept it). Binaries: `ug-daemon`, `ug-agent`, `ug`.
- Platform: **Linux only**, all major distributions (see 02-decisions.md tiers).
- Price: **everything free**. The Windows app has a paid tier called
  **شاه‌پَر (Shahpar)**; the user said to ignore that word — on Linux there are
  no tiers.
- Team: **one person** (the user), working on a laptop running Parrot OS.
- UI language: **Persian (RTL) by default, English second.** The user writes to
  Claude in Persian.

## Who it is for

Linux gamers, including beginners who hit driver problems, and especially
Persian-speaking users (hence the Persian-first UI). The Windows app advertises
Telegram delivery "without a VPN" (بدون فیلترشکن), so the Linux version plans
proxy support for Telegram from the start.

## Hard constraint from the user

The app must **not use much RAM or cause lag during gaming or daily use**. This
is why the UI (Tauri) is optional and closes completely during games, and why the
daemon and agent have measured resource budgets (see 05-findings.md for the
real numbers).

## The Windows app's feature list (user's own intro, verbatim)

The user pasted this from the Windows app's landing page. ✓ = free there,
✦ = Shahpar tier there. On Linux everything is free.

```text
حالت گیمینگ — بازیت که شروع شد، خودم می‌فهمم
 ✓ ۶۱ بازی رو از قبل می‌شناسم، بقیه رو هم خودم تشخیص می‌دم
 ✓ هر بازی پروفایل خودش رو داره
 ✓ برنامه‌های مزاحم رو فقط موقع بازی ببند یا همیشه
تیون کارت گرافیک — آندرولت و اورکلاک، با اندازه‌گیری واقعی
 ✦ تست پایداری قبل از اعمال
 ✦ سه حالت آماده: خنک‌تر، کم‌مصرف‌تر، قوی‌تر
 ✦ هرلحظه با یک کلیک برمی‌گرده به حالت اولش
کرو فن — فن‌ها رو خودم تنظیم می‌کنم
 ✦ کرو از روی رفتار واقعی خنک‌شدن سیستم خودت ساخته می‌شه
 ✦ بی‌صدا موقع کار، خنک موقع بازی
کنترل سیستم — خاموش و روشن شدن سیستم، دست خودت
 ✓ خاموشی و ری‌استارت با زمان دلخواه
 ✦ خاموشی شرطی، مصرف برق، خواب و بیداری
 ✦ ری‌استارت مستقیم به بایوس یا حالت امن
مانیتورینگ — دما، FPS و ترافیک، همه زنده
 ✓ FPS بازی و دما و مصرف قطعات
 ✓ هشدارها فقط به تلگرام فرستاده می‌شن
 ✦ مصرف اینترنت به تفکیک هر برنامه
پایداری پینگ — کنترل آپلود اپ‌های پس‌زمینه
 ✓ شناسایی خودکار تسک‌های پس‌زمینه
 ✓ امکان بلاک مستقیم تسک موردنظر
 ✓ امکان محدودسازی آپلود تسک‌های پس‌زمینه روی عدد دلخواه
اورلی — هرچی لازم داری، وسط بازی جلوی چشمت
 ✓ هرجای صفحه که راحتی، با یک شرت‌کات خاموش و روشن
 ✓ دو چیدمان: کارتی یا فشرده
ضبط بازی — لحظه‌های خوب از دست نره
 ✓ ضبط با کارت گرافیک، بدون افت محسوس FPS
 ✦ هایلایت هوشمند، همون بافر رم و شرت‌کات ذخیره
 ✦ فرستادن کلیپ به تلگرام، مستقیم از خود برنامه
کنترل اینترنت — اینترنتت مال خودت باشه
 ✓ سرویس‌های ویندوز مثل ویندوز آپدیت
 ✦ نرم‌افزار قطعات، دیسکورد، مرورگر، دانلودر
 ✦ برای هر پروفایل جدا
تایم‌پلی — چه‌قدر بازی کردی
 ✓ ساعت بازی هر روز و هر بازی نگه داشته می‌شه
```

The user also wants to **offer extras beyond the Windows app**, naming "an
all-Rust core for better optimisation" and "network settings" as examples (see
the note on this wording in 02-decisions.md). Linux-only extras planned in the
roadmap: sched_ext schedulers, GameScope integration, Proton/Wine tuning, CAKE
bufferbloat control, laptop platform profiles.

## Feature names, Persian ↔ English

| Persian (UI) | English (code/docs) | Roadmap phase |
|---|---|---|
| داشبورد | Dashboard | P1 (UI) |
| حالت گیمینگ | Game mode | P2 |
| تیون کارت گرافیک | GPU tuning | P5 |
| کرو فن | Fan curve | P5 |
| کنترل سیستم | System control (power) | P3 |
| مانیتورینگ | Monitoring | P1, P6 (FPS) |
| پایداری پینگ | Ping stability | P4 |
| کنترل اینترنت | Internet (per-app network) control | P4 |
| اورلی (Overlay) | Overlay | P6 |
| ریکوردر / ضبط بازی | Recorder | P7 |
| هایلایت هوشمند | Smart highlight (replay buffer) | P7 |
| تایم‌پلی | Playtime | P2 |
| گزارش باگ | Bug report | P6.6 |
| به‌روزرسانی | Updates | P10.6 |
| تنظیمات | Settings | P1.7 |
| عیب‌یاب درایور | Driver troubleshooter | P8 |
| پروفایل رقابتی / متعادل / ساکت | competitive / balanced / quiet profile | P2 |
| فقط هنگام بازی / همیشه | only while gaming / always | P2 |

## The Windows app's UI (10 screenshots, described)

The user shared 10 screenshots of the Windows app. The user later said the Linux
UI **does not need to look like them** ("whatever you think is best") — they are
product references, not a design spec. All screens: dark theme, purple accent,
Persian RTL, rounded cards, a right-hand sidebar with: داشبورد، حالت گیمینگ،
کنترل سیستم، مانیتورینگ، پایداری پینگ، Overlay، تایم‌پلی، ریکوردر، گزارش باگ،
به‌روزرسانی، تنظیمات.

1. **Playtime (تایم‌پلی).** Range filter (today / this week / all). Three tiles:
   total this week "34 h 15 min", number of games "5", most played "League of
   Legends". A list sorted by time with percentage bars (LoL 16:55 · 39%,
   Valorant 8:22 · 34%, Elden Ring 5:40 · 17%, Counter-Strike 2 3:15 · 9%,
   Cyberpunk 2077 3 min · 0%), each with a delete button.
2. **Game mode — internet variant.** Master toggle "on"; chips "game selection:
   automatic", "active profile: competitive". Two cards: "internet optimisation —
   block Windows apps and services" (selected; segmented "only while gaming /
   always") and "hardware" ("while gaming"). Tabs "Windows services" / "app
   management". A grid of toggles: block Microsoft auto apps, driver auto-update,
   cloud sync, Microsoft Store, Windows Update (advanced), telemetry, Windows
   notifications, widgets & news. A "revert everything" button.
3. **Recorder.** Tabs "record" / "smart highlight". "Start recording" with
   hotkey Alt+R. What to capture: full screen / window / region; display
   "main monitor · 5120×1440". Quality & size: high / balanced / small, a slider,
   "size per minute: 35 MB", a note that static screens compress smaller.
   Resolution "Native", frame rate "60 fps", advanced settings (codec, audio,
   save path). A card "send straight to Telegram — no VPN needed, full speed".
   "My recordings" gallery with filters and "compress video".
4. **Overlay.** A live overlay preview (139 FPS, 110 / 7.2 ms, GPU temp 63° load
   94%, CPU temp 54° load 33%, frame-time graph). A warning card "the overlay
   is not visible over exclusive fullscreen — use borderless" (a Windows
   limitation that does not exist with MangoHud on Linux). Activation: off /
   while gaming / always. Hotkeys Alt+O (show/hide) and Alt+P (move/resize mode).
   Position & size: preset / custom, a 4-corner grid, small / medium / large.
   Opacity: whole / background, slider at 78%.
5. **Ping chart.** "Ping jitter while downloading, 45 ms": an orange jagged line
   (25–120 ms) before a dashed divider and a flat green line (~45 ms) after it,
   labelled "before: high jitter" and "after: almost flat".
6. **Monitoring.** "Live" badge. Tiles: graphics card 62 °C 41% (RTX 5070 Ti),
   processor 54 °C 23% (Core Ultra 7 265K), memory 11.8/32 GB 37%, network
   ↓4.2 Mb/s ↑0.8 Mb/s. An "FPS monitoring" toggle and "recording now: Cyberpunk
   2077". Tabs: FPS monitoring / temperature & power / internet traffic. Filters
   (today / this week / all, per game), "clear all reports", "4 sessions". Per-
   session cards, e.g. Counter-Strike 2: AVG 287, 1% low 201, 0.1% low 168, "AVG
   3.5 · P99 5.9 ms", badge "smooth"; Cyberpunk 2077: AVG 142, 1% low 98, 0.1%
   low 72, "AVG 7.0 · P99 12.3 ms", badge "3 lag".
7. **System control.** Tabs: power command / conditional shutdown / power usage /
   sleep & wake / wake alarm. Four cards: sleep, hibernate, shutdown (selected,
   red), restart. "Right now / scheduled". A big red "shut down the system"
   button. "Queued commands": "restart in 2 hours · scheduled · cancel". A note:
   "the same commands are available from the Telegram bot".
8. **Fan curve chart.** Presets quiet / balanced / cool. X: GPU temperature
   30–90 °C; Y: fan speed %. Points (30,0) (40,0) (50,28) (60,46) (70,66)
   (80,86) (90,100).
9. **GPU auto-test result.** A row of card models (RTX 5070 Ti Gaming OC, RTX
   3050 ROG selected, RTX 5070 Prime, RTX 3070 Ti Gaming X Trio, GTX 1650 4GB
   TUF). "Automatic GPU test 100%, result recorded, 7 of 7". Results: voltage
   −194 mV, power −27 W, temperature −6 °C, "result of a real run on RTX 3050
   ROG". (On Linux NVIDIA has no voltage control — see 02-decisions.md.)
10. **Game mode — hardware variant.** Card "hardware optimisation — RAM, CPU,
    graphics and Windows" selected. Tabs: CPU & RAM / Windows & system /
    graphics / advanced graphics. Toggles: CCD lock (X3D CPUs), CPU priority for
    the game, prevent CPU clock drops, RAM optimisation (on).
