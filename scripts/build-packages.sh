#!/usr/bin/env bash
# Builds the beta packages locally into dist/. Uploads and publishes nothing.
#
#   ./scripts/build-packages.sh            release build + window, then packages
#   ./scripts/build-packages.sh --no-build package what target/release has
#
# dist/ gets:
#   unix-guardian_<ver>_amd64.deb             Debian, Ubuntu, Mint, Parrot, Pop!_OS
#   unix-guardian-<ver>-x86_64.tar.gz         any systemd distribution: install.sh
#   arch/PKGBUILD + unix-guardian.install + source tarball, for makepkg on Arch
set -euo pipefail
umask 022

here="$(cd "$(dirname "$0")/.." && pwd)"
cd "$here"
ver=$(sed -n 's/^pkgver=//p' packaging/arch/PKGBUILD)          # 0.1.0beta1
tag=$(sed -n 's/^_tag=//p' packaging/arch/PKGBUILD)            # 0.1.0-beta.1
debver=$(sed -E 's/([0-9])(alpha|beta|rc)/\1~\2/' <<<"$ver")     # 0.1.0~beta1: sorts before 0.1.0
arch=$(dpkg --print-architecture 2>/dev/null || echo amd64)
out="$here/dist"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

if [ "${1:-}" != --no-build ]; then
  cargo build --release --workspace
  (cd app && npm run build >/dev/null && npx tauri build --no-bundle)
fi
rm -rf "$out"
mkdir -p "$out/arch"

# --- .deb -----------------------------------------------------------------
deb="$work/deb"
./scripts/stage.sh "$deb" /usr
# Debian keeps the licence in the copyright file, not under share/licenses.
rm -rf "$deb/usr/share/licenses"
doc="$deb/usr/share/doc/unix-guardian"
mkdir -p "$doc"
{
  echo "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/"
  echo "Upstream-Name: Unix Guardian"
  echo "Source: https://github.com/AGlegendery/Unix-Guardian"
  echo
  echo "Files: *"
  echo "Copyright: 2026 Unix Guardian contributors"
  echo "License: GPL-3.0-or-later"
  echo " On Debian systems, the full text of the GNU General Public License"
  echo " version 3 can be found in /usr/share/common-licenses/GPL-3."
} > "$doc/copyright"
printf 'unix-guardian (%s) unstable; urgency=medium\n\n  * Beta build.\n\n -- Unix Guardian <unixguardian@atomicmail.io>  %s\n' \
  "$debver" "$(date -R)" | gzip -9n > "$doc/changelog.gz"
strip --strip-unneeded "$deb"/usr/bin/* 2>/dev/null || true
# Library dependencies of the binaries, as Debian names them.
mkdir -p "$work/shlib/debian"
echo "Source: unix-guardian" > "$work/shlib/debian/control"
depends=$(cd "$work/shlib" && dpkg-shlibdeps -O "$deb"/usr/bin/* 2>/dev/null | sed -n 's/^shlibs:Depends=//p')
mkdir -p "$deb/DEBIAN"
sed -e "s/@VERSION@/$debver/" -e "s/@ARCH@/$arch/" -e "s/@DEPENDS@/$depends/" \
    -e "s/@SIZE@/$(du -sk --exclude=DEBIAN "$deb" | cut -f1)/" packaging/deb/control.in > "$deb/DEBIAN/control"
install -m755 packaging/deb/postinst packaging/deb/prerm packaging/deb/postrm "$deb/DEBIAN/"
# Files under /etc are the admin's to edit.
(cd "$deb" && find etc -type f | sed 's|^|/|') > "$deb/DEBIAN/conffiles"
# GitHub renames "~" in uploaded file names (to "."), which breaks SHA256SUMS:
# the file name says 0.1.0-beta1, the package inside keeps 0.1.0~beta1.
fakeroot dpkg-deb --build -Zxz "$deb" "$out/unix-guardian_${debver//\~/-}_${arch}.deb" >/dev/null

# --- install.sh tarball ---------------------------------------------------
tdir="$work/unix-guardian-$ver-x86_64"
./scripts/stage.sh "$tdir/files" /usr/local
install -m755 packaging/install.sh "$tdir/install.sh"
cp LICENSE "$tdir/"
tar -C "$work" --owner=0 --group=0 -czf "$out/unix-guardian-$ver-x86_64.tar.gz" "unix-guardian-$ver-x86_64"

# --- Arch -----------------------------------------------------------------
cp packaging/arch/PKGBUILD packaging/arch/unix-guardian.install "$out/arch/"
# The tarball GitHub would serve for tag v$ver, from the committed tree.
git archive --format=tar.gz --prefix="Unix-Guardian-$tag/" -o "$out/arch/unix-guardian-$ver.tar.gz" HEAD

echo "Built in dist/ (nothing was uploaded):"
(cd "$out" && find . -type f | sort | sed 's|^\./|  |')
[ -x "$deb/usr/bin/ug-app" ] || echo "note: the window was not built, so it is not in the packages"
