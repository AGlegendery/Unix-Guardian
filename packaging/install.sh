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
    echo "A development install is present; remove it first: sudo ./scripts/dev-install.sh --uninstall" >&2
    exit 1
  fi
  command -v systemctl >/dev/null || { echo "Unix Guardian needs systemd." >&2; exit 1; }
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
