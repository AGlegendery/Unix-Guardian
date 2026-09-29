#!/usr/bin/env bash
# Installs a development build as a real system service, to exercise the parts
# `--session` cannot: the system bus, polkit prompts, and running as root.
#
#   cargo build --release && sudo ./scripts/dev-install.sh
#   sudo ./scripts/dev-install.sh --uninstall
#
# Everything goes under /usr/local and /etc so it cannot collide with a
# packaged install; --uninstall removes exactly what this installs.
set -euo pipefail

[ "$(id -u)" -eq 0 ] || { echo "run with sudo" >&2; exit 1; }
here="$(cd "$(dirname "$0")/.." && pwd)"
pkg="$here/packaging"

files=(
  /usr/local/bin/ug-daemon
  /usr/local/bin/ug
  /etc/systemd/system/unix-guardiand.service
  /etc/dbus-1/system.d/org.unixguardian.Daemon1.conf
  /usr/share/polkit-1/actions/org.unixguardian.policy
)

if [ "${1:-}" = "--uninstall" ]; then
  systemctl disable --now unix-guardiand.service 2>/dev/null || true
  rm -f "${files[@]}"
  systemctl daemon-reload
  echo "removed. State in /var/lib/unix-guardian is kept; delete it by hand if wanted."
  exit 0
fi

bin="$here/target/release"
[ -x "$bin/ug-daemon" ] || { echo "build first: cargo build --release" >&2; exit 1; }

install -Dm755 "$bin/ug-daemon" /usr/local/bin/ug-daemon
install -Dm755 "$bin/ug"        /usr/local/bin/ug
sed 's|/usr/bin/ug-daemon|/usr/local/bin/ug-daemon|' "$pkg/systemd/unix-guardiand.service" \
  > /etc/systemd/system/unix-guardiand.service
install -Dm644 "$pkg/dbus/org.unixguardian.Daemon1.conf" /etc/dbus-1/system.d/org.unixguardian.Daemon1.conf
install -Dm644 "$pkg/polkit/org.unixguardian.policy"     /usr/share/polkit-1/actions/org.unixguardian.policy

systemctl daemon-reload
# dbus-broker and dbus-daemon both pick up new policy on reload.
systemctl reload dbus.service 2>/dev/null || busctl call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus ReloadConfig
systemctl restart unix-guardiand.service

echo "installed. Try (as your normal user, no sudo):"
echo "  ug status"
echo "  ug gpu caps"
echo "  busctl call org.unixguardian.Daemon1 /org/unixguardian/Daemon1 org.unixguardian.Daemon1.System SetLogFilter s debug"
echo "     ^ should raise a polkit password prompt"
echo "logs: journalctl -u unix-guardiand -f"
