#!/usr/bin/env bash
# One-command install of Unix Guardian from this source folder.
#
#   bash ./installer.sh                       install or update
#   bash ./installer.sh --uninstall           remove it (settings and history are kept)
#   bash ./installer.sh --uninstall --purge   remove it and your settings and history
#
# Run it as your normal user: it builds as you and asks for sudo only to
# install packages and the system service. It detects the package manager
# (pacman, apt, dnf, zypper), installs what building and running need (Rust,
# Node.js, the WebKitGTK files), builds the daemon, agent, CLI and window,
# installs them under /usr/local (scripts/dev-install.sh) and starts them.
#
# Options:
#   --yes         answer yes to every question (the package manager's too)
#   --no-extras   do not install the optional tools (MangoHud, nftables, ...)
#   --skip-deps   install no packages; only build and install
#   --help        this text
#
# A log of the last run is kept in ~/.cache/unix-guardian-installer/install.log.
set -euo pipefail
umask 022

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cache="${XDG_CACHE_HOME:-$HOME/.cache}/unix-guardian-installer"
log="$cache/install.log"

yes=0 extras=1 deps=1 mode=install purge=0
for arg in "$@"; do
  case "$arg" in
    --yes|-y) yes=1 ;;
    --no-extras) extras=0 ;;
    --skip-deps) deps=0 ;;
    --uninstall) mode=uninstall ;;
    --purge) purge=1 ;;
    --help|-h) sed -n '2,/^set -euo/p' "$0" | sed -e '$d' -e 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $arg (see --help)" >&2; exit 2 ;;
  esac
done
[ "$purge" = 0 ] || [ "$mode" = uninstall ] || { echo "--purge goes with --uninstall" >&2; exit 2; }

bold=$'\e[1m' red=$'\e[31m' green=$'\e[32m' yellow=$'\e[33m' off=$'\e[0m'
[ -t 1 ] || bold="" red="" green="" yellow="" off=""
step() { printf '\n%s==> %s%s\n' "$bold" "$*" "$off"; }
note() { printf '%s  ! %s%s\n' "$yellow" "$*" "$off"; }
ok()   { printf '%s  ✓ %s%s\n' "$green" "$*" "$off"; }
die()  { printf '\n%sError: %s%s\n' "$red" "$*" "$off" >&2; exit 1; }
ask() {
  # ask "question" -> 0 for yes. Defaults to yes; --yes or no terminal answers yes.
  [ "$yes" = 1 ] && return 0
  [ -t 0 ] || return 0
  local a; read -r -p "  $1 [Y/n] " a || return 0
  case "$a" in [nN]*) return 1 ;; *) return 0 ;; esac
}

mkdir -p "$cache"
exec > >(tee "$log") 2>&1

# ---------------------------------------------------------------- checks

[ "$(id -u)" -ne 0 ] || die "run this as your normal user, not with sudo: bash ./installer.sh
It asks for your password itself, only for the steps that need it."
[ "$(uname -m)" = x86_64 ] || die "Unix Guardian runs on x86_64 only (this is $(uname -m))."
[ -d /run/systemd/system ] || die "Unix Guardian needs a system running systemd."
command -v sudo >/dev/null || die "sudo is not installed; install it (as root) and add yourself to the sudo/wheel group."

os_id="" os_like="" os_name="Linux"
if [ -r /etc/os-release ]; then
  # shellcheck disable=SC1091
  . /etc/os-release
  os_id="${ID:-}" os_like="${ID_LIKE:-}" os_name="${PRETTY_NAME:-${NAME:-Linux}}"
fi

immutable() {
  cat >&2 <<EOF

${red}${os_name} is an image-based (immutable) system.${off}
Its /usr and its package database are read-only, and Unix Guardian is a
system service (a root daemon with a polkit policy under /usr/share), so it
cannot be installed there with this script yet. A Flatpak cannot run the
root daemon either.

Unix Guardian supports regular distributions for now: Arch (and CachyOS,
Manjaro, EndeavourOS), Debian, Ubuntu (and Mint, Pop!_OS, Parrot), Fedora
Workstation and openSUSE Tumbleweed/Leap.
EOF
  exit 1
}

if [ "$mode" = install ]; then
  [ -e /run/ostree-booted ] && immutable                     # Bazzite, Bluefin, Silverblue, Kinoite
  case "$os_id" in steamos|nixos|*microos*|aeon|kalpa) immutable ;; esac
  findmnt -no OPTIONS /usr 2>/dev/null | tr ',' '\n' | grep -qx ro && immutable
fi

[ -w "$here" ] || die "this folder is not writable by you ($here); clone it somewhere in your home."

pm=""
for p in pacman apt-get dnf zypper; do
  if command -v "$p" >/dev/null; then pm="$p"; break; fi
done

sudo_ready=0
need_sudo() {
  [ "$sudo_ready" = 1 ] && return
  echo "  (sudo asks for your password: $1)"
  sudo -v || die "sudo failed."
  sudo_ready=1
}

# ---------------------------------------------------------------- uninstall

if [ "$mode" = uninstall ]; then
  step "Removing Unix Guardian"
  if [ -e /etc/systemd/system/unix-guardiand.service ] || [ -e /usr/local/bin/ug-daemon ]; then
    pkill -u "$(id -u)" -x ug-app 2>/dev/null || true
    systemctl --user stop ug-agent.service 2>/dev/null || true
    pkill -u "$(id -u)" -x ug-agent 2>/dev/null || true
    need_sudo "to remove the system service"
    sudo "$here/scripts/dev-install.sh" --uninstall
    systemctl --user daemon-reload 2>/dev/null || true
    ok "removed"
  else
    note "no install from this script was found (/usr/local/bin/ug-daemon is missing)"
    [ -e /usr/bin/ug-daemon ] && note "a package is installed: sudo pacman -R / apt remove / dnf remove unix-guardian"
  fi
  if [ "$purge" = 1 ]; then
    cfg="${XDG_CONFIG_HOME:-$HOME/.config}/unix-guardian"
    data="${XDG_DATA_HOME:-$HOME/.local/share}/unix-guardian"
    echo
    echo "  This deletes for good: $cfg, $data, /var/lib/unix-guardian"
    echo "  (your profiles, playtime history, recordings list, settings)."
    if ask "Delete them?"; then
      rm -rf "$cfg" "$data"
      need_sudo "to delete /var/lib/unix-guardian"
      sudo rm -rf /var/lib/unix-guardian
      ok "settings and history deleted (other users' own folders are not touched)"
    fi
  else
    echo "  Your settings and history are kept; --uninstall --purge deletes them."
  fi
  exit 0
fi

# ---------------------------------------------------------------- conflicts

if [ -e /usr/bin/ug-daemon ]; then
  die "Unix Guardian is installed from a package already (/usr/bin/ug-daemon).
Remove it first (sudo pacman -R / apt remove / dnf remove unix-guardian), or update the package instead."
fi
if [ -e /usr/local/share/unix-guardian/installed-files ]; then
  die "Unix Guardian is installed from the release tarball. Remove it first with that folder's: sudo ./install.sh --uninstall"
fi

# A copy put in ~/.local/bin by hand (only the window, say) comes first on
# PATH, so it would run instead of this install and find no service.
stale=()
for b in ug ug-app ug-agent ug-daemon ug-stress; do
  # ugrep also installs a program called ug: only ours mention Unix Guardian.
  f="$HOME/.local/bin/$b"
  [ -f "$f" ] && grep -qa "Unix Guardian" "$f" && stale+=("$f")
done
if [ "${#stale[@]}" -gt 0 ]; then
  note "older copies found: ${stale[*]}"
  note "they would run instead of the version installed now."
  if ask "Remove them?"; then rm -f "${stale[@]}"; ok "removed"; fi
fi

step "Installing Unix Guardian on $os_name"
[ -e /usr/local/bin/ug-daemon ] && echo "  An earlier install was found; it is updated in place, settings kept."

# ---------------------------------------------------------------- packages

version_ge() { [ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -1)" = "$2" ]; }
rust_ok() {
  command -v cargo >/dev/null && command -v rustc >/dev/null || return 1
  version_ge "$(rustc --version | awk '{print $2}')" 1.85
}

# The window's build (Vite) wants Node ^20.19 or >=22.12. Debian 12 and
# Ubuntu 24.04 ship 18, so an official Node 22 is unpacked into the cache
# then, used for the build only and not installed anywhere.
node_ok() {
  command -v node >/dev/null && command -v npm >/dev/null || return 1
  local v; v="$(node --version | tr -d v)"
  case "${v%%.*}" in
    20) version_ge "$v" 20.19 ;;
    21) return 1 ;;
    *) version_ge "$v" 22.12 ;;
  esac
}


# Build needs: a C toolchain, pkg-config, WebKitGTK 4.1 + GTK 3 + librsvg
# headers (the window), D-Bus headers, curl and xz (Rust/Node downloads).
# Run needs: WebKitGTK, the AppIndicator library (tray), polkit.
# Extras: optional features, each also works without them.
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
have_cargo=0; command -v cargo >/dev/null && have_cargo=1
case "$pm" in
  pacman)
    req=(base-devel git curl xz pkgconf webkit2gtk-4.1 gtk3 librsvg libayatana-appindicator polkit)
    node_ok || req+=(nodejs npm)
    [ "$have_cargo" = 1 ] || req+=(rust)
    ext=(mangohud nftables iproute2 flatpak vulkan-icd-loader) ;;
  apt-get)
    req=(build-essential pkg-config git curl xz-utils ca-certificates libwebkit2gtk-4.1-dev
         libgtk-3-dev librsvg2-dev libdbus-1-dev libayatana-appindicator3-1)
    node_ok || req+=(nodejs npm)
    # Debian's and Ubuntu's Rust is usually older than 1.85: rustup below.
    ext=(mangohud nftables iproute2 flatpak libvulkan1) ;;
  dnf)
    req=(gcc gcc-c++ make pkgconf-pkg-config git curl xz webkit2gtk4.1-devel gtk3-devel
         librsvg2-devel dbus-devel libappindicator-gtk3 polkit)
    node_ok || req+=(nodejs npm)
    [ "$have_cargo" = 1 ] || req+=(cargo rust)
    ext=(mangohud nftables iproute flatpak vulkan-loader) ;;
  zypper)
    req=(gcc gcc-c++ make pkgconf git curl xz webkit2gtk3-devel gtk3-devel librsvg-devel
         dbus-1-devel libappindicator3-1 polkit)
    node_ok || req+=(nodejs-default npm-default)
    [ "$have_cargo" = 1 ] || req+=(cargo rust)
    ext=(mangohud nftables iproute2 flatpak libvulkan1) ;;
  *) req=(); ext=() ;;
esac

available() {
  case "$pm" in
    pacman)  pacman -Si "$1" >/dev/null 2>&1 || pacman -Qi "$1" >/dev/null 2>&1 ;;
    apt-get) [ -n "$(apt-cache policy "$1" 2>/dev/null | sed -n 's/^ *Candidate: //p' | grep -v none)" ] ;;
    dnf)     dnf -q info "$1" >/dev/null 2>&1 ;;
    zypper)  zypper -q --non-interactive search -x "$1" >/dev/null 2>&1 ;;
  esac
}

pm_install() {
  case "$pm" in
    pacman)
      local f=(); [ "$yes" = 1 ] && f=(--noconfirm)
      sudo pacman -S --needed "${f[@]}" "$@" ;;
    apt-get)
      local f=(); [ "$yes" = 1 ] && f=(-y)
      sudo DEBIAN_FRONTEND=readline apt-get install "${f[@]}" "$@" ;;
    dnf)
      local f=(); [ "$yes" = 1 ] && f=(-y)
      sudo dnf install "${f[@]}" "$@" ;;
    zypper)
      local f=(); [ "$yes" = 1 ] && f=(--non-interactive)
      sudo zypper "${f[@]}" install "$@" ;;
  esac
}

if [ "$deps" = 0 ]; then
  step "Skipping packages (--skip-deps)"
elif [ -z "$pm" ]; then
  step "Packages"
  note "no known package manager (pacman, apt, dnf, zypper) was found."
  note "Install by hand: a C compiler, pkg-config, WebKitGTK 4.1, GTK 3, librsvg and D-Bus"
  note "development files, the AppIndicator library, Node.js 22 and curl; then run again with --skip-deps."
  exit 1
else
  step "Installing build and runtime packages with $pm"
  echo "  ${req[*]}"
  need_sudo "to install packages"
  if [ "$pm" = apt-get ]; then sudo apt-get update; fi
  if [ "$pm" = pacman ] && ! pm_install "${req[@]}"; then
    die "pacman could not install them. If it said a file was not found, the package
lists are out of date: run 'sudo pacman -Syu' and then this installer again."
  fi
  [ "$pm" = pacman ] || pm_install "${req[@]}" || die "$pm could not install the packages above."
  ok "build and runtime packages"

  if [ "$extras" = 1 ]; then
    want=()
    for p in "${ext[@]}"; do available "$p" && want+=("$p"); done
    if [ "${#want[@]}" -gt 0 ]; then
      echo
      echo "  Optional tools: ${want[*]}"
      echo "  (in-game overlay and FPS, internet control, the recorder, GPU stability test)"
      if ask "Install them too?"; then
        pm_install "${want[@]}" || note "some optional tools were not installed; everything else still works"
      fi
    fi
  fi
fi

# ---------------------------------------------------------------- Rust

step "Rust"
if rust_ok; then
  ok "$(rustc --version)"
else
  if command -v rustup >/dev/null; then
    rustup toolchain install stable --profile minimal
  else
    echo "  Rust 1.85 or newer is needed and was not found$(command -v rustc >/dev/null && echo " ($(rustc --version))")."
    ask "Install it for your user with rustup (into ~/.cargo, from rustup.rs)?" \
      || die "Rust is needed to build Unix Guardian."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain stable
    . "$HOME/.cargo/env"
  fi
  rust_ok || die "Rust is still older than 1.85 ($(rustc --version 2>/dev/null))."
  ok "$(rustc --version)"
fi

# ---------------------------------------------------------------- Node.js

step "Node.js"
if node_ok; then
  ok "node $(node --version)"
else
  echo "  Node.js 20.19+ or 22.12+ is needed$(command -v node >/dev/null && echo "; this one is $(node --version)")."
  ask "Download the official Node.js 22 from nodejs.org into $cache (for the build only)?" \
    || die "Node.js is needed to build the window."
  base=https://nodejs.org/dist/latest-v22.x
  sums="$(curl -fsSL "$base/SHASUMS256.txt")" || die "could not reach nodejs.org"
  line="$(printf '%s\n' "$sums" | grep -E ' node-v22\.[0-9.]+-linux-x64\.tar\.xz$')" \
    || die "no Node 22 linux-x64 tarball listed on nodejs.org"
  file="${line##* }"
  if [ ! -x "$cache/${file%.tar.xz}/bin/node" ]; then
    curl -fL --progress-bar -o "$cache/$file" "$base/$file"
    (cd "$cache" && printf '%s\n' "$line" | sha256sum -c --quiet -) || die "the Node download is damaged"
    tar -xJf "$cache/$file" -C "$cache"
    rm -f "$cache/$file"
  fi
  export PATH="$cache/${file%.tar.xz}/bin:$PATH"
  node_ok || die "Node.js is still not usable."
  ok "node $(node --version) (downloaded)"
fi

# ---------------------------------------------------------------- build

step "Building the daemon, agent and command line (first time: several minutes)"
cd "$here"
cargo build --release --locked --workspace
ok "target/release"

step "Building the window"
cd "$here/app"
# npm ci deletes and refetches node_modules, so only when the lock changed.
if [ ! -f node_modules/.package-lock.json ] || [ package-lock.json -nt node_modules/.package-lock.json ]; then
  npm ci --no-audit --no-fund
fi
npx tauri build --no-bundle
cd "$here"
ok "app/src-tauri/target/release/ug-app"

# ---------------------------------------------------------------- install

step "Installing (system service, user agent, window, menu entry)"
need_sudo "to install the system service"
sudo "$here/scripts/dev-install.sh"

step "Starting"
systemctl --user daemon-reload 2>/dev/null || true
systemctl --user restart ug-agent.service 2>/dev/null \
  || note "could not start the agent now; it starts by itself at your next login"

up=0
for _ in $(seq 1 20); do
  if /usr/local/bin/ug status >/dev/null 2>&1; then up=1; break; fi
  sleep 0.5
done
if [ "$up" = 1 ]; then
  ok "the daemon answers"
else
  note "the daemon does not answer yet: journalctl -u unix-guardiand -b"
fi

cat <<EOF

${green}${bold}Unix Guardian is installed.${off}
  Open it from the app menu (Unix Guardian), or run:  ug-app   or   ug tui
  Check this machine:  ug testlab
  Update later:        git pull && bash ./installer.sh
  Remove:              bash ./installer.sh --uninstall
EOF
if [[ ":${XDG_CURRENT_DESKTOP:-}:" == *:GNOME:* ]]; then
  echo "  GNOME: the tray icon needs the AppIndicator extension (README, \"Tray icon on GNOME\")."
fi
# An NVIDIA card (vendor 0x10de, display class 0x03) without NVIDIA's own
# driver loaded: games run on nouveau. The driver check says how to fix it.
for d in /sys/bus/pci/devices/*; do
  if [ "$(cat "$d/vendor" 2>/dev/null)" = 0x10de ] && [[ "$(cat "$d/class" 2>/dev/null)" == 0x03* ]] \
     && [ ! -d /sys/module/nvidia ]; then
    note "This NVIDIA card is not using NVIDIA's driver. See how to install it: ug diagnose"
    [ "$pm" = dnf ] && note "(on Fedora it comes from RPM Fusion; ug diagnose lists the commands)"
    break
  fi
done
echo "  Log of this run: $log"
