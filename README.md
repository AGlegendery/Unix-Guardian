<p align="center">
  <img src="assets/branding/ug-app-icon.svg" alt="Unix Guardian" width="96">
</p>

# Unix Guardian

**Free and open-source gaming and system toolkit for Linux.** Game mode, live
monitoring, GPU tuning, fan curves, power control, ping stability, per-app
internet control, overlay, screen recording, playtime and a driver
troubleshooter, in one window, one terminal UI and one command-line tool.

It is for people who play games on Linux and for people who don't play at all
but want to see and control their own hardware. Every change it makes is
temporary and put back afterwards, even after a crash.

**Status: beta (`0.1.0-beta.1`), not released yet.** Everything below is built
and runs; testing on more desktops and distributions is under way (see
[Tested on](#tested-on)).

[فارسی](#فارسی) · [Features](#features) · [Install](#install) ·
[Tray icon on GNOME](#tray-icon-on-gnome) · [Reporting bugs](#reporting-bugs)

> 📷 **Screenshot slot: Dashboard** (to be added)

## Features

Everything is available in all three front ends: the window (`ug-app`), the
terminal UI (`ug tui`) and the command line (`ug`). English and Persian.

### Dashboard
- CPU, GPU, memory and network at a glance.
- **Game mode button** with any profile, on by hand.
- **Card switch** on hybrid laptops: pick which GPU the programs Unix Guardian
  starts use. Per program only; the desktop itself is never moved.

### Game mode and profiles
- Detects games from Steam, Lutris, Heroic, Bottles and Wine, and guesses
  unknown ones by how they behave.
- Profiles per game, with tabs for **Processor and memory**, **System and
  services** and **Graphics**: CPU governor and energy preference, no deep idle
  states, the CPU's fast cores (P-cores, X3D cache), game priority, memory
  tuning, pausing background services and apps, power profile, network
  tuning, keeping the screen on, Do Not Disturb (KDE).
- Your own profiles; "only while gaming" or "always". Everything is reverted
  when the game ends, even after a crash.
- **Steam launch options** edited per game (GPU, MangoHud), with a backup and
  exact undo.

### GPU
- NVIDIA (NVML) and AMD: power limit, core and memory clock offsets within the
  driver's bounds.
- **Auto-tune** (faster or cooler) with a built-in Vulkan stress test, presets,
  a 30-second trial with a crash watchdog and an 88 °C guard.
- Shows which GPU a running program really uses. Never wakes a sleeping GPU
  just to read it.
- `ug run --gpu nvidia -- <program>`: run one program on the other card.

### Driver troubleshooter
- Checks graphics, Wi-Fi, Bluetooth, audio, input, camera, printing, sleep and
  firmware, and explains what is wrong in plain words.
- Fixes with the full plan shown first, a password every time, a snapshot
  (snapper, Timeshift, Btrfs, ZFS) before package installs, and undo.
- Package installs through apt, pacman, dnf and zypper; a boot-parameter
  editor (GRUB, systemd-boot, rEFInd, kernelstub).
- After a restart it checks again: **Keep** or **Undo**. Works from a text
  console too (`ug diagnose`).

### System control
- **Power command**: shut down, restart, sleep, hibernate, now or on a
  schedule; restart straight into BIOS/UEFI setup.
- **Conditional shutdown** (when downloads stop, the CPU goes idle, a program
  exits or the game ends), **power
  usage**, **wake alarm**, **sleep and wake** (including hibernation swap
  setup).
- **Battery protection**: stop charging at a limit and run from the adapter.
- **Fans and cooling**: power profile, the laptop's fan mode, fan curves with
  failsafes.

### Monitoring
- **Temperature and power**: live, last hour, today, this week, with averages,
  peaks and charts for CPU, GPU, memory, disks, battery and more.
- **FPS**: average, 1 % and 0.1 % lows and stutters per game session (from
  MangoHud).
- **Internet traffic and internet control**: use per day, week and month, the
  apps that used the most, live speed, and per-app **block** or **speed
  limit**, always or only while gaming.
- **Ping stability**: ping and jitter to the router, the DNS server and the
  game's server, before and after game mode, and **slow down background apps**
  (never the game, a VPN or DNS) while you play.

### Overlay
- MangoHud set up from the window, with a live preview; shown only while
  gaming or always.
- **Global shortcuts** for opening the window, the overlay and game mode, keys
  of your choice.

### Recorder
- Screen and game sound recording with gpu-screen-recorder, set up for you.

### Playtime
- Time per game: today, this week, all time, per day, hours of the day,
  streaks, sessions on a timeline, CSV export.
- Track any app too; import Steam's own hours.

### Test lab («تست نرم‌افزاری»)
- Tries every feature on your computer for a moment, puts everything back, and
  writes a report with personal details removed, ready to email to us. Nothing
  is sent by itself.

### Updates and settings
- Checks for new versions daily; installing always waits for you, is signed
  (minisign) and can be rolled back.
- Language, tray icon, notifications, global shortcuts.

### Tray icon
- Live G gauge (normal, warning, critical, game mode), a usage preview, and
  quick access to the window and the terminal UI.

> 📷 **Screenshot slots: Game mode · GPU tuner · Driver troubleshooter ·
> Monitoring · Playtime · Terminal UI** (to be added)

### Command line

`ug --help` lists everything. The main commands:

| Command | What it does |
|---|---|
| `ug status`, `ug monitor`, `ug history` | Readings now, live, and over past hours |
| `ug tui` | The terminal UI, every page |
| `ug profile`, `ug game` | Profiles, game mode on/off, installed and running games |
| `ug gpu`, `ug run` | GPU tuning, run a program on a chosen GPU |
| `ug diagnose`, `ug driver` | Driver check, fixes, history, undo |
| `ug power`, `ug battery`, `ug fans` | Power actions and schedules, battery limit, fans |
| `ug net`, `ug traffic`, `ug ping` | Per-app internet control, internet use, ping |
| `ug overlay`, `ug fps`, `ug record`, `ug shortcut` | Overlay, FPS stats, recorder, global shortcuts |
| `ug playtime` | Playtime, tracked apps, Steam import, CSV |
| `ug testlab` | The test lab report |
| `ug update`, `ug settings`, `ug tray`, `ug quit` | Updates, settings, tray icon, stop the agent |

Add `--json` to any command for scripts.

## Requirements

- A **systemd** Linux distribution, x86_64.
- Any desktop. **KDE Plasma** gets everything; on **GNOME** the tray needs an
  extension ([below](#tray-icon-on-gnome)) and Do Not Disturb is KDE only.
- Optional: MangoHud (overlay, FPS), power-profiles-daemon (performance
  profile while gaming), nftables and iproute2 (internet control), Flatpak
  (the recorder is installed through it).

## Install

**No release is published yet.** Until then, build the packages yourself
(see [Building](#building)): `./scripts/build-packages.sh` puts them in `dist/`.

| Distribution | Install |
|---|---|
| Debian, Ubuntu, Mint, Pop!_OS, Parrot | `sudo apt install ./unix-guardian_<version>_amd64.deb` |
| Arch, CachyOS, Manjaro, EndeavourOS | `cd dist/arch && makepkg -si` |
| Fedora, openSUSE, any other systemd distro | unpack `unix-guardian-<version>-x86_64.tar.gz`, then `sudo ./install.sh` |

Remove with `sudo apt remove unix-guardian`, `sudo pacman -R unix-guardian` or
`sudo ./install.sh --uninstall`. Your settings and history are kept.

After installing, open **Unix Guardian** from the app menu, or run `ug tui`.

## Tray icon on GNOME

**GNOME has no tray by default. To see the Unix Guardian tray icon, install and
enable the "AppIndicator and KStatusNotifierItem Support" extension.** KDE,
Cinnamon, XFCE and most other desktops need nothing.

| Distro                      | Install                                                     |
|-----------------------------|-------------------------------------------------------------|
| Debian / Ubuntu / Parrot    | **`sudo apt install gnome-shell-extension-appindicator`**   |
| Fedora                      | **`sudo dnf install gnome-shell-extension-appindicator`**   |
| Arch                        | **`sudo pacman -S gnome-shell-extension-appindicator`**     |
| openSUSE                    | **not in the official repos: install it from [extensions.gnome.org](https://extensions.gnome.org/extension/615/appindicator-support/) or the Extension Manager app** |

Then enable it. **The ID differs by distro**: the Debian/Ubuntu/Parrot package
uses `ubuntu-appindicators@ubuntu.com`, every other source uses the upstream ID.

```bash
# Debian / Ubuntu / Parrot
gnome-extensions enable ubuntu-appindicators@ubuntu.com
# Fedora, Arch, openSUSE, extensions.gnome.org
gnome-extensions enable appindicatorsupport@rgcjonas.gmail.com
```

**On Wayland, log out and back in if the extension does not show up as enabled.**
The icon appears by itself once the extension is running; Unix Guardian does not
need a restart.

## Tested on

| Desktop | Distribution | Result |
|---|---|---|
| KDE Plasma (Wayland) | Debian-based | ✅ Passed |
| GNOME (Wayland) | Debian-based | ✅ Passed |
| Hyprland (Wayland) | Debian-based | ✅ Passed |
| XFCE (X11) | Debian-based | ⚠️ Passed except the recorder (being fixed) |
| — | CachyOS | ⏳ Pending |

Tried it somewhere else? Run the test lab (`ug testlab` or the Test lab page)
and email us the report.

## Architecture

Three processes, one protocol:

| Process      | Runs as                  | Job                                                        |
|--------------|--------------------------|------------------------------------------------------------|
| `ug-daemon`  | root, systemd system     | sysfs/NVML/nftables/tc/logind — everything needing privilege |
| `ug-agent`   | user, systemd --user     | game detection, playtime, profiles, MangoHud, recorder, tray |
| `ug-app`     | user, optional           | Tauri UI — closed during games, zero cost when not open      |

The UI never touches sysfs. Everything goes through D-Bus so the window, the
terminal UI, the CLI and scripts share one API. Every privileged action goes
through polkit. The daemon stays at about 6 MB when idle.

## Building

Needs Rust 1.85+, Node.js 22 and the WebKitGTK development files:

```bash
# Debian / Ubuntu / Parrot
sudo apt install build-essential pkg-config libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev
# Fedora
sudo dnf install gcc pkgconf-pkg-config webkit2gtk4.1-devel gtk3-devel librsvg2-devel
# Arch / CachyOS
sudo pacman -S --needed base-devel webkit2gtk-4.1 gtk3 librsvg
```

```bash
cargo build --workspace
cargo test --workspace
cd app && npm ci && npx tauri build --no-bundle   # the window
./scripts/build-packages.sh                         # .deb, tarball and PKGBUILD in dist/
```

Install a development build as a real system service (under `/usr/local`,
removable with `--uninstall`):

```bash
cargo build --release --workspace
sudo ./scripts/dev-install.sh
```

Run the daemon on the session bus for development (no root, no install, never
touches the real system's services), and the agent against it:

```bash
cargo run -p ug-daemon -- --session
cargo run -p ug-cli -- --session status
cargo run -p ug-agent -- --daemon-session
cd app && npm run dev        # the UI in a browser, with recorded data
```

End-to-end checks on a private bus:

```bash
./scripts/test-bus.sh ./scripts/smoke.sh
./scripts/test-bus.sh ./scripts/agent-e2e.sh
./scripts/test-bus.sh ./scripts/gamemode-e2e.sh
```

## Reporting bugs

Email **unixguardian@atomicmail.io** with what you did, what happened, and the
output of `ug status` and `journalctl -u unix-guardiand -b`. The test lab report
(`ug testlab`) helps most. The app's Bug report page, the TUI's help (`?`) and
every CLI error show this address too.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).

---

## فارسی

<div dir="rtl">

**یونیکس گاردین: ابزار رایگان و متن‌باز گیمینگ و مدیریت سیستم برای لینوکس.**
حالت گیمینگ، مانیتورینگ زنده، تیون کارت گرافیک، منحنی فن، کنترل برق، پایداری
پینگ، کنترل اینترنت هر برنامه، اورلی، ضبط صفحه، تایم‌پلی و عیب‌یاب درایور، در یک
پنجره، یک رابط ترمینال و یک ابزار خط فرمان.

هم برای کسانی است که روی لینوکس بازی می‌کنند و هم برای کسانی که اصلاً بازی
نمی‌کنند ولی می‌خواهند سخت‌افزار خودشان را ببینند و کنترل کنند. هر تغییری که
می‌دهد موقتی است و بعد برمی‌گردد، حتی بعد از کرش.

**وضعیت: بتا (`0.1.0-beta.1`)، هنوز منتشر نشده.** همه‌ی بخش‌های زیر ساخته شده و
کار می‌کنند؛ تست روی دسکتاپ‌ها و توزیع‌های بیشتر در جریان است.

> 📷 **جای تصویر: داشبورد** (بعداً اضافه می‌شود)

### قابلیت‌ها

همه‌چیز در هر سه رابط هست: پنجره (`ug-app`)، رابط ترمینال (`ug tui`) و خط فرمان
(`ug`). به فارسی و انگلیسی.

**داشبورد**
- نمای سریع پردازنده، کارت گرافیک، رم و شبکه.
- **دکمه‌ی حالت گیمینگ** با هر پروفایل، به‌صورت دستی.
- **سوییچ کارت گرافیک** در لپ‌تاپ‌های دوکارته: انتخاب کارتی که برنامه‌هایی که
  یونیکس گاردین اجرا می‌کند از آن استفاده کنند. فقط برای هر برنامه؛ خود دسکتاپ هیچ‌وقت جابه‌جا نمی‌شود.

**حالت گیمینگ و پروفایل‌ها**
- تشخیص بازی از Steam، Lutris، Heroic، Bottles و Wine، و حدس زدن بازی‌های ناشناخته از رفتارشان.
- پروفایل برای هر بازی با سه تب **پردازنده و رم**، **سیستم و سرویس‌ها** و
  **گرافیک**: حالت پردازنده، جلوگیری از خواب عمیق پردازنده، هسته‌های سریع
  پردازنده (P-core، کش X3D)، اولویت بازی، تنظیم حافظه، توقف سرویس‌ها و برنامه‌های
  پس‌زمینه، پروفایل برق، تنظیم شبکه، روشن ماندن صفحه، مزاحم نشوید (KDE).
- پروفایل‌های شخصی؛ «فقط هنگام بازی» یا «همیشه». همه‌چیز بعد از بازی، حتی بعد از کرش، برمی‌گردد.
- **گزینه‌های اجرای Steam** برای هر بازی (کارت گرافیک، MangoHud)، با پشتیبان و برگشت دقیق.

**کارت گرافیک**
- NVIDIA و AMD: محدودیت توان، آفست فرکانس هسته و حافظه در محدوده‌ی مجاز درایور.
- **تیون خودکار** (سریع‌تر یا خنک‌تر) با تست پایداری Vulkan، پیش‌تنظیم‌ها، آزمایش
  ۳۰ ثانیه‌ای با نگهبان کرش و محافظ ۸۸ درجه.
- نشان می‌دهد هر برنامه واقعاً از کدام کارت استفاده می‌کند. کارت خوابیده را فقط برای خواندن بیدار نمی‌کند.
- `ug run --gpu nvidia -- <برنامه>`: اجرای یک برنامه روی کارت دیگر.

**عیب‌یاب درایور**
- بررسی گرافیک، وای‌فای، بلوتوث، صدا، ورودی، دوربین، پرینتر، خواب و فرم‌ویر، و توضیح مشکل به زبان ساده.
- رفع مشکل با نمایش کامل برنامه قبل از اجرا، رمز عبور در هر بار، اسنپ‌شات
  (snapper، Timeshift، Btrfs، ZFS) قبل از نصب بسته، و امکان برگشت.
- نصب بسته با apt، pacman، dnf و zypper؛ ویرایشگر پارامترهای بوت (GRUB، systemd-boot، rEFInd، kernelstub).
- بعد از ری‌استارت دوباره بررسی می‌کند: **نگه‌دار** یا **برگردان**. از کنسول متنی هم کار می‌کند (`ug diagnose`).

**کنترل سیستم**
- **فرمان برق**: خاموش، ری‌استارت، خواب، هایبرنیت، همین حالا یا زمان‌بندی‌شده؛ ری‌استارت مستقیم به بایوس/UEFI.
- **خاموشی شرطی** (وقتی دانلودها تمام شد، پردازنده بیکار شد، برنامه‌ای بسته شد یا بازی تمام شد)، **مصرف برق**، **بیدارباش**، **خواب و بیداری** (با راه‌اندازی swap برای هایبرنیت).
- **محافظت باتری**: توقف شارژ در یک حد و کار با آداپتور.
- **فن و خنک‌کاری**: پروفایل برق، حالت فن لپ‌تاپ، منحنی فن با محافظ‌های ایمنی.

**مانیتورینگ**
- **دما و مصرف**: زنده، یک ساعت اخیر، امروز، این هفته، با میانگین، اوج و نمودار.
- **مانیتورینگ FPS**: میانگین، ۱٪ و ۰٫۱٪ پایین و لگ‌ها برای هر جلسه‌ی بازی (از MangoHud).
- **ترافیک و کنترل اینترنت**: مصرف روزانه، هفتگی و ماهانه، پرمصرف‌ترین برنامه‌ها،
  سرعت زنده، و **مسدود کردن** یا **محدود کردن سرعت** هر برنامه، همیشه یا فقط هنگام بازی.
- **پایداری پینگ**: پینگ و نوسان تا روتر، سرور DNS و سرور بازی، قبل و بعد از حالت
  گیمینگ، و **کند کردن برنامه‌های پس‌زمینه** هنگام بازی (هیچ‌وقت خود بازی، VPN یا DNS).

**اورلی**
- راه‌اندازی MangoHud از داخل پنجره با پیش‌نمایش زنده؛ فقط هنگام بازی یا همیشه.
- **میان‌برهای سراسری** برای باز کردن پنجره، اورلی و حالت گیمینگ، با کلیدهای دلخواه.

**ریکوردر**
- ضبط صفحه و صدای بازی با gpu-screen-recorder که خودش برایتان راه‌اندازی می‌کند.

**تایم‌پلی**
- زمان هر بازی: امروز، این هفته، همه، روزبه‌روز، ساعت‌های روز، رکورد روزهای پشت‌سرهم، خروجی CSV.
- شمردن زمان هر برنامه‌ی دلخواه؛ ایمپورت ساعت‌های Steam.

**تست نرم‌افزاری**
- همه‌ی قابلیت‌ها را برای لحظه‌ای روی کامپیوتر شما امتحان می‌کند، همه‌چیز را
  برمی‌گرداند و گزارشی بدون اطلاعات شخصی می‌سازد که می‌توانید برای ما ایمیل کنید. چیزی خودکار فرستاده نمی‌شود.

**به‌روزرسانی و تنظیمات**
- بررسی روزانه‌ی نسخه‌ی جدید؛ نصب همیشه منتظر شماست، امضاشده است و قابل برگشت.
- زبان، آیکون سینی سیستم، اعلان‌ها، میان‌برهای سراسری.

**آیکون سینی سیستم**
- نشانگر G زنده (عادی، هشدار، بحرانی، حالت گیمینگ)، پیش‌نمایش مصرف و دسترسی سریع به پنجره و رابط ترمینال.

> 📷 **جای تصویر: حالت گیمینگ · تیونر کارت گرافیک · عیب‌یاب درایور · مانیتورینگ · تایم‌پلی · رابط ترمینال** (بعداً اضافه می‌شود)

**خط فرمان:** `ug --help` همه‌ی فرمان‌ها را نشان می‌دهد (جدول بخش انگلیسی). با `--json` خروجی برای اسکریپت‌ها می‌گیرید.

### پیش‌نیازها

- یک توزیع لینوکس با **systemd**، معماری x86_64.
- هر دسکتاپی. **KDE Plasma** همه‌چیز را دارد؛ در **گنوم** سینی سیستم یک افزونه
  لازم دارد (پایین‌تر) و «مزاحم نشوید» فقط در KDE است.
- اختیاری: MangoHud (اورلی، FPS)، power-profiles-daemon (پروفایل کارایی هنگام
  بازی)، nftables و iproute2 (کنترل اینترنت)، Flatpak (ریکوردر با آن نصب می‌شود).

### نصب

**هنوز نسخه‌ای منتشر نشده.** تا آن موقع بسته‌ها را خودتان بسازید:
`./scripts/build-packages.sh` آن‌ها را در `dist/` می‌گذارد.

- **دبیان، اوبونتو، مینت، پاپ‌اواس، پرت:** `sudo apt install ./unix-guardian_<version>_amd64.deb`
- **آرچ، کشی‌اواس، مانجارو، اندیوراواس:** `cd dist/arch && makepkg -si`
- **فدورا، اوپن‌سوزه و هر توزیع systemd دیگر:** فایل `unix-guardian-<version>-x86_64.tar.gz` را باز کنید و `sudo ./install.sh` بزنید.

حذف: `sudo apt remove unix-guardian`، `sudo pacman -R unix-guardian` یا
`sudo ./install.sh --uninstall`. تنظیمات و تاریخچه‌ی شما می‌ماند.

بعد از نصب، **Unix Guardian** را از منوی برنامه‌ها باز کنید یا `ug tui` بزنید.

### آیکون سینی سیستم در گنوم

**گنوم به‌طور پیش‌فرض سینی سیستم (tray) ندارد. برای دیدن آیکون Unix Guardian،
افزونهٔ «AppIndicator and KStatusNotifierItem Support» را نصب و فعال کنید.**
کی‌دی‌ای، سینامون، XFCE و بیشتر میزکارهای دیگر به چیزی نیاز ندارند.

- **دبیان / اوبونتو / پرت:** `sudo apt install gnome-shell-extension-appindicator`
- **فدورا:** `sudo dnf install gnome-shell-extension-appindicator`
- **آرچ:** `sudo pacman -S gnome-shell-extension-appindicator`
- **اوپن‌سوزه:** در مخزن رسمی نیست؛ از [extensions.gnome.org](https://extensions.gnome.org/extension/615/appindicator-support/) یا برنامهٔ Extension Manager نصب کنید.

سپس فعالش کنید. **شناسهٔ افزونه در توزیع‌ها فرق دارد:** بستهٔ دبیان/اوبونتو/پرت
`ubuntu-appindicators@ubuntu.com` است و بقیه شناسهٔ اصلی را دارند.

</div>

```bash
# Debian / Ubuntu / Parrot
gnome-extensions enable ubuntu-appindicators@ubuntu.com
# Fedora, Arch, openSUSE, extensions.gnome.org
gnome-extensions enable appindicatorsupport@rgcjonas.gmail.com
```

<div dir="rtl">

**در ویلند، اگر افزونه فعال نشد، یک بار از حساب خارج و دوباره وارد شوید.**
آیکون پس از اجرای افزونه خودبه‌خود ظاهر می‌شود و نیازی به راه‌اندازی دوبارهٔ Unix Guardian نیست.

### تست‌شده روی

| دسکتاپ | توزیع | نتیجه |
|---|---|---|
| KDE Plasma (ویلند) | مبتنی بر دبیان | ✅ موفق |
| گنوم (ویلند) | مبتنی بر دبیان | ✅ موفق |
| Hyprland (ویلند) | مبتنی بر دبیان | ✅ موفق |
| XFCE (X11) | مبتنی بر دبیان | ⚠️ موفق به‌جز ریکوردر (در حال رفع) |
| — | CachyOS | ⏳ در انتظار |

روی سیستم دیگری امتحانش کردید؟ تست نرم‌افزاری (`ug testlab` یا همین صفحه در برنامه) را اجرا کنید و گزارشش را برای ما ایمیل کنید.

### ساخت از سورس

دستورها در بخش انگلیسی [Building](#building) آمده‌اند: Rust نسخه‌ی ۱٫۸۵ به بالا،
Node.js 22 و فایل‌های توسعه‌ی WebKitGTK لازم است.

### گزارش باگ

به **unixguardian@atomicmail.io** ایمیل بزنید: چه کردید، چه شد، و خروجی `ug status`
و `journalctl -u unix-guardiand -b`. گزارش تست نرم‌افزاری (`ug testlab`) بیشترین کمک را می‌کند.

### مجوز

GPL-3.0-or-later. فایل [LICENSE](LICENSE) را ببینید.

</div>
