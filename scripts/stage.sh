#!/usr/bin/env bash
# Lays out an installed Unix Guardian under a scratch root, the one file list
# every package uses (the .deb, the Arch PKGBUILD, the install.sh tarball).
#
#   ./scripts/stage.sh DESTDIR [PREFIX]
#
# PREFIX is /usr (distribution packages, the default) or /usr/local (the
# install.sh tarball, so it never collides with a package). Binaries come from
# target/release, and the window from app/src-tauri/target/release when it has
# been built. Paths inside units and D-Bus files are rewritten to PREFIX.
#
# Files whose place is fixed by the program that reads them stay there
# whatever the prefix: polkit only reads /usr/share/polkit-1/actions, and the
# system bus policy goes to /usr/share/dbus-1/system.d (packages) or
# /etc/dbus-1/system.d (local installs).
set -euo pipefail
umask 022

dest="${1:?usage: stage.sh DESTDIR [PREFIX]}"
prefix="${2:-/usr}"
here="$(cd "$(dirname "$0")/.." && pwd)"
pkg="$here/packaging"
bin="${UG_BIN_DIR:-$here/target/release}"
app="${UG_APP_BIN:-$here/app/src-tauri/target/release/ug-app}"

for b in ug-daemon ug-agent ug; do
  [ -x "$bin/$b" ] || { echo "missing $bin/$b: cargo build --release --workspace" >&2; exit 1; }
done

# /usr/bin/ug-… in the shipped files becomes PREFIX/bin/ug-…
fix() { sed -e "s|/usr/bin/ug|$prefix/bin/ug|g" -e "s|^Exec=ug|Exec=$prefix/bin/ug|"; }
put() { install -Dm644 /dev/stdin "$dest$1"; }

install -Dm755 "$bin/ug-daemon" "$dest$prefix/bin/ug-daemon"
install -Dm755 "$bin/ug-agent" "$dest$prefix/bin/ug-agent"
install -Dm755 "$bin/ug" "$dest$prefix/bin/ug"
[ -x "$bin/ug-stress" ] && install -Dm755 "$bin/ug-stress" "$dest$prefix/bin/ug-stress"
if [ -x "$app" ]; then
  install -Dm755 "$app" "$dest$prefix/bin/ug-app"
else
  echo "note: the window (ug-app) is not built and is left out: cd app && npx tauri build --no-bundle" >&2
fi

fix < "$pkg/systemd/unix-guardiand.service" | put "$prefix/lib/systemd/system/unix-guardiand.service"
fix < "$pkg/systemd/ug-agent.service" | put "$prefix/lib/systemd/user/ug-agent.service"
fix < "$pkg/dbus/org.unixguardian.Daemon1.service" | put "$prefix/share/dbus-1/system-services/org.unixguardian.Daemon1.service"
fix < "$pkg/dbus/org.unixguardian.Agent1.service" | put "$prefix/share/dbus-1/services/org.unixguardian.Agent1.service"
if [ "$prefix" = /usr ]; then policy=/usr/share/dbus-1/system.d; else policy=/etc/dbus-1/system.d; fi
put "$policy/org.unixguardian.Daemon1.conf" < "$pkg/dbus/org.unixguardian.Daemon1.conf"
put /usr/share/polkit-1/actions/org.unixguardian.policy < "$pkg/polkit/org.unixguardian.policy"
fix < "$pkg/autostart/ug-agent.desktop" | put /etc/xdg/autostart/ug-agent.desktop
fix < "$pkg/applications/unix-guardian.desktop" | put "$prefix/share/applications/unix-guardian.desktop"

brand="$here/assets/branding"
put "$prefix/share/icons/hicolor/scalable/apps/unix-guardian.svg" < "$brand/ug-app-icon.svg"
for state in normal warning critical gamemode offline; do
  put "$prefix/share/icons/hicolor/scalable/status/unix-guardian-$state.svg" < "$brand/tray/ug-tray-$state.svg"
  put "$prefix/share/icons/hicolor/16x16/status/unix-guardian-$state.svg" < "$brand/tray/ug-tray-$state-16.svg"
done
put "$prefix/share/licenses/unix-guardian/LICENSE" < "$here/LICENSE"
