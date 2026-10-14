#!/usr/bin/env bash
# Runs the release tarball's install.sh into scratch roots (never the real
# system, no root needed) and checks what a user sees:
#
#   ./scripts/tarball-install-test.sh                 a tarball folder built here
#                                                     from fake binaries
#   ./scripts/tarball-install-test.sh dist/X.tar.gz   the real tarball
#
# - the tarball holds install.sh, LICENSE and files/ and nothing else is needed;
# - install, then --uninstall, leaves nothing behind;
# - over an install from the source folder (scripts/dev-install.sh, which
#   installer.sh runs) it takes that install's place instead of sending the
#   user to an installer.sh their folder does not have.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

if [ -n "${1:-}" ]; then
  tar -C "$work" -xzf "$1"
  dir=$(find "$work" -mindepth 1 -maxdepth 1 -type d | head -n1)
else
  mkdir -p "$work/bin"
  for b in ug-daemon ug-agent ug ug-stress ug-app; do
    printf '#!/bin/sh\necho %s\n' "$b" > "$work/bin/$b"; chmod 755 "$work/bin/$b"
  done
  dir="$work/unix-guardian-test-x86_64"
  UG_BIN_DIR="$work/bin" UG_APP_BIN="$work/bin/ug-app" "$here/scripts/stage.sh" "$dir/files" /usr/local 2>/dev/null
  install -m755 "$here/packaging/install.sh" "$dir/install.sh"
  cp "$here/LICENSE" "$dir/"
fi

# What the folder holds.
[ -x "$dir/install.sh" ] || fail "no executable install.sh in the tarball"
[ -f "$dir/LICENSE" ] || fail "no LICENSE in the tarball"
[ -x "$dir/files/usr/local/bin/ug-daemon" ] || fail "no ug-daemon under files/"
[ -f "$dir/files/usr/local/lib/systemd/system/unix-guardiand.service" ] || fail "no daemon unit under files/"
bash -n "$dir/install.sh" || fail "install.sh does not parse"
# Every file install.sh names must be in the folder it ships in.
if grep -n 'installer\.sh' "$dir/install.sh" | grep -v '^[0-9]*:[[:space:]]*#'; then
  fail "install.sh tells the user to run installer.sh, which the tarball does not have"
fi
programs() { (cd "$1" && find . -type f -o -type l) | grep -v '^./usr/local/share/unix-guardian/installed-files$' | sort; }

# 1. A clean install, then --uninstall.
r="$work/root1"; mkdir -p "$r"
"$dir/install.sh" --root "$r" >/dev/null
[ -x "$r/usr/local/bin/ug-daemon" ] || fail "install did not put ug-daemon in place"
[ -f "$r/usr/local/share/unix-guardian/installed-files" ] || fail "no installed-files list"
diff <(programs "$dir/files") <(programs "$r") >/dev/null || fail "installed files differ from files/"
"$dir/install.sh" --root "$r" --uninstall >/dev/null
[ -z "$(find "$r" -type f)" ] || fail "--uninstall left: $(find "$r" -type f)"

# 2. Over an install from the source folder: every path dev-install.sh
#    writes (its files=() list and the tray icons) exists beforehand.
r="$work/root2"; mkdir -p "$r"
mapfile -t dev < <(sed -n '/^files=(/,/^)/p' "$here/scripts/dev-install.sh" | grep -o '/[^ )]*')
for state in normal warning critical gamemode offline; do
  dev+=(/usr/local/share/icons/hicolor/scalable/status/unix-guardian-$state.svg
        /usr/local/share/icons/hicolor/16x16/status/unix-guardian-$state.svg)
done
[ "${#dev[@]}" -gt 10 ] || fail "could not read dev-install.sh's file list"
for f in "${dev[@]}"; do install -D /dev/null "$r$f"; echo source-install > "$r$f"; done
out=$("$dir/install.sh" --root "$r" 2>&1) || fail "install over a source-folder install failed: $out"
for f in "${dev[@]}"; do
  if [ -e "$r$f" ] && ! [ -e "$dir/files$f" ]; then fail "the source-folder install's $f is still there"; fi
  if [ -e "$r$f" ] && grep -qx source-install "$r$f"; then fail "$f was not replaced"; fi
done
diff <(programs "$dir/files") <(programs "$r") >/dev/null || fail "after replacing a source install, files differ from files/"
"$dir/install.sh" --root "$r" --uninstall >/dev/null
[ -z "$(find "$r" -type f)" ] || fail "--uninstall after a takeover left: $(find "$r" -type f)"

echo "tarball install test: ok"
