#!/usr/bin/env bash
# Installs Unix Guardian from this release folder on any systemd
# distribution, under /usr/local so it never collides with a package.
#
#   sudo ./install.sh              install (or update)
#   sudo ./install.sh --uninstall  remove exactly what was installed
#
# The list of installed files is kept in
# /usr/local/share/unix-guardian/installed-files for --uninstall.
# --root DIR installs into DIR without touching services (for testing).
set -euo pipefail

root=""
mode=install
while [ $# -gt 0 ]; do
  case "$1" in
    --uninstall) mode=uninstall ;;
    --root) root="${2:?--root needs a directory}"; shift ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
  shift
done
here="$(cd "$(dirname "$0")" && pwd)"
list="$root/usr/local/share/unix-guardian/installed-files"
live() { [ -z "$root" ] && [ -d /run/systemd/system ]; }

[ -n "$root" ] || [ "$(id -u)" -eq 0 ] || { echo "run with sudo" >&2; exit 1; }

if [ "$mode" = uninstall ]; then
  [ -f "$list" ] || { echo "Unix Guardian was not installed by install.sh (no $list)" >&2; exit 1; }
  if live; then
    # Stopping the daemon reverts what it changed before the files go.
    systemctl disable --now unix-guardiand.service 2>/dev/null || true
    systemctl --global disable ug-agent.service 2>/dev/null || true
  fi
  while IFS= read -r f; do rm -f "$root$f"; done < "$list"
  rm -f "$list"
  live && systemctl daemon-reload
  echo "Removed. State in /var/lib/unix-guardian and each user's ~/.config and ~/.local/share/unix-guardian is kept."
  echo "A running agent stops at logout, or now: systemctl --user stop ug-agent"
  exit 0
fi

# Two installs of the daemon would fight over the same bus name.
if [ -z "$root" ]; then
  if [ -e /usr/bin/ug-daemon ]; then
    echo "Unix Guardian is installed from a package already (/usr/bin/ug-daemon); update the package instead." >&2
    exit 1
  fi
  if [ -e /etc/systemd/system/unix-guardiand.service ]; then
    echo "An install from the source folder is present; remove it first there: bash ./installer.sh --uninstall" >&2
    exit 1
  fi
  command -v systemctl >/dev/null || { echo "Unix Guardian needs systemd." >&2; exit 1; }
fi

# The window and the tray need WebKitGTK 4.1 and an AppIndicator library
# from the distribution; everything else is in this folder.
if [ -z "$root" ]; then
  have() { ldconfig -p 2>/dev/null | grep -q "$1"; }
  need=()
  if command -v pacman >/dev/null; then
    have libwebkit2gtk-4.1.so.0 || need+=(webkit2gtk-4.1)
    have libayatana-appindicator3.so.1 || need+=(libayatana-appindicator)
    install_() { pacman -S --needed --noconfirm "$@"; }
  elif command -v apt-get >/dev/null; then
    have libwebkit2gtk-4.1.so.0 || need+=(libwebkit2gtk-4.1-0)
    have libayatana-appindicator3.so.1 || need+=(libayatana-appindicator3-1)
    install_() { apt-get update && apt-get install -y "$@"; }
  elif command -v dnf >/dev/null; then
    have libwebkit2gtk-4.1.so.0 || need+=(webkit2gtk4.1)
    have libayatana-appindicator3.so.1 || have libappindicator3.so.1 || need+=(libappindicator-gtk3)
    install_() { dnf install -y "$@"; }
  elif command -v zypper >/dev/null; then
    have libwebkit2gtk-4.1.so.0 || need+=(libwebkit2gtk-4_1-0)
    have libappindicator3.so.1 || need+=(libappindicator3-1)
    install_() { zypper --non-interactive install "$@"; }
  fi
  if [ "${#need[@]}" -gt 0 ]; then
    echo "Installing what the window needs: ${need[*]}"
    install_ "${need[@]}" || echo "note: could not install ${need[*]}; the window may not open (ug and ug tui still work)" >&2
  fi
fi

files="$here/files"
( cd "$files" && find . -type f -o -type l ) | sed 's|^\.||' | sort > "$here/.list"
while IFS= read -r f; do
  mode_=$(stat -c %a "$files$f")
  install -Dm"$mode_" "$files$f" "$root$f"
done < "$here/.list"
install -Dm644 "$here/.list" "$list"
rm -f "$here/.list"

if live; then
  # SELinux (Fedora): the new files take their folders' labels.
  if command -v restorecon >/dev/null && command -v selinuxenabled >/dev/null && selinuxenabled; then
    while IFS= read -r f; do restorecon -F "$f" 2>/dev/null || true; done < "$list"
  fi
  systemctl daemon-reload
  systemctl reload dbus.service 2>/dev/null || busctl call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus ReloadConfig >/dev/null || true
  systemctl enable unix-guardiand.service
  systemctl restart unix-guardiand.service
  systemctl --global enable ug-agent.service
  command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t /usr/local/share/icons/hicolor 2>/dev/null || true
  # Each logged-in user's agent restarts with the new program; a new agent
  # also takes over from an old one started some other way.
  for u in $(loginctl list-users --no-legend 2>/dev/null | awk '{print $2}'); do
    systemctl --user -M "$u@" restart ug-agent.service 2>/dev/null || true
  done
fi
echo "Installed. The agent starts at your next login, or now (as yourself, no sudo):"
echo "  systemctl --user daemon-reload && systemctl --user start ug-agent"
echo "Then: ug status, ug tui, or Unix Guardian in the applications menu."
