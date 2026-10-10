# Unix Guardian

Free and open-source gaming toolkit for Linux: game mode, GPU tuning, fan curves,
system control, live monitoring, ping stability, overlay, recording, per-app network
control, playtime, and a graphics-driver troubleshooter.

Status: **pre-alpha.** Monitoring, game detection, playtime and reversible game mode work;
the UI is not started. Plan, status and project knowledge: [`claude-project/`](claude-project/00-README.md).

## Architecture

Three processes, one protocol:

| Process      | Runs as                  | Job                                                        |
|--------------|--------------------------|------------------------------------------------------------|
| `ug-daemon`  | root, systemd system     | sysfs/NVML/nftables/tc/logind — everything needing privilege |
| `ug-agent`   | user, systemd --user     | game detection, playtime, MangoHud, recorder, Telegram       |
| `ug-app`     | user, optional           | Tauri UI — closed during games, zero cost when not open      |

The UI never touches sysfs. Everything goes through D-Bus so the CLI, scripts and the
Telegram bot share one API.

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

```bash
# Debian / Ubuntu / Parrot
gnome-extensions enable ubuntu-appindicators@ubuntu.com
# Fedora, Arch, openSUSE, extensions.gnome.org
gnome-extensions enable appindicatorsupport@rgcjonas.gmail.com
```

**در ویلند، اگر افزونه فعال نشد، یک بار از حساب خارج و دوباره وارد شوید.**
آیکون پس از اجرای افزونه خودبه‌خود ظاهر می‌شود و نیازی به راه‌اندازی دوبارهٔ Unix Guardian نیست.

## Building

```bash
cargo build --workspace
cargo test --workspace
```

Run the daemon on the session bus for development (no root, no install, never
touches the real system's services), and the agent against it:

```bash
cargo run -p ug-daemon -- --session
cargo run -p ug-cli -- --session status
cargo run -p ug-agent -- --daemon-session
```

End-to-end checks on a private bus:

```bash
dbus-run-session -- ./scripts/smoke.sh
dbus-run-session -- ./scripts/agent-e2e.sh
dbus-run-session -- ./scripts/gamemode-e2e.sh
```

## Reporting bugs

Email **unixguardian@atomicmail.io** with what you did, what happened, and the
output of `ug status` and `journalctl -u unix-guardiand -b`. The app's Bug
report page, the TUI's help (`?`) and every CLI error show this address too.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
