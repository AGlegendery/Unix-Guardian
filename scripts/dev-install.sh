#!/usr/bin/env bash
# Installs a development build as a real system service, to exercise the parts
# `--session` cannot: the system bus, polkit prompts, and running as root.
#
#   cargo build --release && sudo ./scripts/dev-install.sh
#   sudo ./scripts/dev-install.sh --uninstall
#
# Installs the daemon (system service) and the agent (a user service enabled
# for every user, plus its D-Bus activation file and XDG autostart fallback).
#
# Everything goes under /usr/local and /etc so it cannot collide with a
# packaged install; --uninstall removes exactly what this installs.
set -euo pipefail

[ "$(id -u)" -eq 0 ] || { echo "run with sudo" >&2; exit 1; }
here="$(cd "$(dirname "$0")/.." && pwd)"
pkg="$here/packaging"

files=(
  /usr/local/bin/ug-daemon
  /usr/local/bin/ug-agent
  /usr/local/bin/ug
  /usr/local/bin/ug-app
  /etc/systemd/user/ug-agent.service
  /usr/local/share/dbus-1/services/org.unixguardian.Agent1.service
  /etc/xdg/autostart/ug-agent.desktop
  /etc/systemd/system/unix-guardiand.service
  /usr/local/share/dbus-1/system-services/org.unixguardian.Daemon1.service
  /etc/dbus-1/system.d/org.unixguardian.Daemon1.conf
  /usr/share/polkit-1/actions/org.unixguardian.policy
  /usr/local/share/applications/unix-guardian.desktop
  /usr/local/share/icons/hicolor/scalable/apps/unix-guardian.svg
)
# The tray's G gauge, one icon per state, loaded by name so it follows the
# panel's colours.
for state in normal warning critical gamemode offline; do
  files+=(/usr/local/share/icons/hicolor/scalable/status/unix-guardian-$state.svg
          /usr/local/share/icons/hicolor/16x16/status/unix-guardian-$state.svg)
done

if [ "${1:-}" = "--uninstall" ]; then
  systemctl disable --now unix-guardiand.service 2>/dev/null || true
  systemctl --global disable ug-agent.service 2>/dev/null || true
  rm -f "${files[@]}"
  systemctl daemon-reload
  echo "removed. State in /var/lib/unix-guardian and ~/.local/share/unix-guardian is kept."
  echo "A running agent keeps running until you log out, or: systemctl --user stop ug-agent"
  exit 0
fi

bin="$here/target/release"
[ -x "$bin/ug-daemon" ] || { echo "build first: cargo build --release" >&2; exit 1; }

install -Dm755 "$bin/ug-daemon" /usr/local/bin/ug-daemon
install -Dm755 "$bin/ug-agent"  /usr/local/bin/ug-agent
install -Dm755 "$bin/ug"        /usr/local/bin/ug
# The GPU stability test, when built (cargo build --release -p ug-stress).
if [ -x "$bin/ug-stress" ]; then
  install -Dm755 "$bin/ug-stress" /usr/local/bin/ug-stress
fi
# The window, when it has been built (cd app && npx tauri build --no-bundle).
# `ug tui` opens it with g, and it opens `ug tui` from its sidebar.
if [ -x "$here/app/src-tauri/target/release/ug-app" ]; then
  install -Dm755 "$here/app/src-tauri/target/release/ug-app" /usr/local/bin/ug-app
fi
sed 's|^Exec=ug-app|Exec=/usr/local/bin/ug-app|' "$pkg/applications/unix-guardian.desktop" \
  | install -Dm644 /dev/stdin /usr/local/share/applications/unix-guardian.desktop
brand="$here/assets/branding"
install -Dm644 "$brand/ug-app-icon.svg" /usr/local/share/icons/hicolor/scalable/apps/unix-guardian.svg
for state in normal warning critical gamemode offline; do
  install -Dm644 "$brand/tray/ug-tray-$state.svg" /usr/local/share/icons/hicolor/scalable/status/unix-guardian-$state.svg
  install -Dm644 "$brand/tray/ug-tray-$state-16.svg" /usr/local/share/icons/hicolor/16x16/status/unix-guardian-$state.svg
done
touch /usr/local/share/icons/hicolor 2>/dev/null || true
sed 's|/usr/bin/ug-daemon|/usr/local/bin/ug-daemon|' "$pkg/systemd/unix-guardiand.service" \
  > /etc/systemd/system/unix-guardiand.service
install -Dm644 "$pkg/dbus/org.unixguardian.Daemon1.conf" /etc/dbus-1/system.d/org.unixguardian.Daemon1.conf
install -Dm644 "$pkg/polkit/org.unixguardian.policy"     /usr/share/polkit-1/actions/org.unixguardian.policy
install -Dm644 "$pkg/dbus/org.unixguardian.Daemon1.service" \
  /usr/local/share/dbus-1/system-services/org.unixguardian.Daemon1.service

# The agent: a user unit enabled globally (a symlink under /etc/systemd/user,
# not a change to anyone's home), activation on the session bus, and the
# autostart fallback for sessions without a systemd user manager.
sed 's|/usr/bin/ug-agent|/usr/local/bin/ug-agent|' "$pkg/systemd/ug-agent.service" \
  > /etc/systemd/user/ug-agent.service
sed 's|/usr/bin/ug-agent|/usr/local/bin/ug-agent|' "$pkg/dbus/org.unixguardian.Agent1.service" \
  | install -Dm644 /dev/stdin /usr/local/share/dbus-1/services/org.unixguardian.Agent1.service
sed 's|^Exec=ug-agent|Exec=/usr/local/bin/ug-agent|' "$pkg/autostart/ug-agent.desktop" \
  | install -Dm644 /dev/stdin /etc/xdg/autostart/ug-agent.desktop
systemctl --global enable ug-agent.service

systemctl daemon-reload
# dbus-broker and dbus-daemon both pick up new policy on reload.
systemctl reload dbus.service 2>/dev/null || busctl call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus ReloadConfig
# Enabled, not only started: without it the daemon is gone after a reboot and
# the agent logs "service not reachable" at every game. The activation file
# covers the gap until it is up.
systemctl enable unix-guardiand.service
systemctl restart unix-guardiand.service

# A window or agent that was already running keeps the old program in
# memory (its file now shows as "(deleted)"), so new pages and features do
# not appear. Close the old windows and restart the user's agent.
if [ -n "${SUDO_USER:-}" ] && [ "$SUDO_USER" != root ]; then
  stale=$(for p in $(pgrep -u "$SUDO_USER" -x ug-app; pgrep -u "$SUDO_USER" -x ug-agent); do
            readlink "/proc/$p/exe" | grep -q '(deleted)' && echo "$p"; done)
  if pgrep -u "$SUDO_USER" -x ug-app >/dev/null; then
    pkill -u "$SUDO_USER" -x ug-app && echo "closed the old Unix Guardian window; open it again to see the new version"
  fi
  if systemctl --user -M "$SUDO_USER@" daemon-reload 2>/dev/null \
     && systemctl --user -M "$SUDO_USER@" restart ug-agent.service 2>/dev/null; then
    echo "restarted $SUDO_USER's agent with the new version"
  elif [ -n "$stale" ]; then
    echo "an old agent is still running; restart it (as yourself): systemctl --user restart ug-agent"
  fi
fi

echo "installed. If the agent is not running yet (it starts by itself from the next login):"
echo "  systemctl --user daemon-reload && systemctl --user start ug-agent"
echo "Then try (as your normal user, no sudo):"
echo "  ug status"
echo "  ug gpu caps"
echo "  busctl call org.unixguardian.Daemon1 /org/unixguardian/Daemon1 org.unixguardian.Daemon1.System SetLogFilter s debug"
echo "     ^ should raise a polkit password prompt"
echo "logs: journalctl -u unix-guardiand -f"
