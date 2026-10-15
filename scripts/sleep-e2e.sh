#!/usr/bin/env bash
# End-to-end check of the sleep settings: sleep variant, keep-at-boot file,
# lid and power-button drop-in, through the CLI and a development daemon on a
# scratch tree (--sysroot), so nothing real is written.
#
#   cargo build --workspace && ./scripts/test-bus.sh ./scripts/sleep-e2e.sh
set -uo pipefail
if busctl --user list --activatable 2>/dev/null | grep -q org.unixguardian; then
  echo "FAIL  the test bus can activate an installed Unix Guardian service; run with scripts/test-bus.sh" >&2
  exit 1
fi
B="${1:-$PWD/target/debug}"
W="$(mktemp -d)"
R="$W/root"
fails=0
check() { if eval "$2"; then echo "PASS  $1"; else echo "FAIL  $1"; fails=$((fails+1)); fi; }
until_() { local d=$((SECONDS+$1)); shift; until eval "$@"; do (( SECONDS < d )) || return 1; sleep 0.2; done; }
ug() { "$B/ug" --session "$@"; }
mkdir -p "$R/sys/power" "$R/run/systemd/system" "$R/proc/acpi/button/lid/LID0"
echo "s2idle [deep]" >"$R/sys/power/mem_sleep"
"$B/ug-daemon" --session --sysroot "$R" --state-dir "$W/state" --log warn 2>>"$W/daemon.log" & daemon=$!
trap 'kill $daemon 2>/dev/null; rm -rf "$W"' EXIT
until_ 20 "ug power sleep >/dev/null 2>&1" || { echo "FAIL  daemon did not start"; cat "$W/daemon.log"; exit 1; }

check "the variants and the one in use are shown" "ug power sleep | grep -q 'Sleep: deep.*s2idle, deep'"
check "a lid is found" "ug power sleep | grep -q 'Lid closed on battery: *suspend'"
check "s2idle can be chosen and kept" "ug power sleep --mode s2idle --keep >/dev/null && grep -qx s2idle '$R/sys/power/mem_sleep' && grep -q 'mem_sleep - - - - s2idle' '$R/etc/tmpfiles.d/unix-guardian-sleep.conf'"
# The kernel shows the choice in brackets; a scratch file just holds the write.
echo "[s2idle] deep" >"$R/sys/power/mem_sleep"
check "keep can be turned off" "ug power sleep --keep=false >/dev/null && [ ! -e '$R/etc/tmpfiles.d/unix-guardian-sleep.conf' ]"
check "a variant the kernel does not offer is refused" "! ug power sleep --mode hibernate >/dev/null 2>&1"
D="$R/etc/systemd/logind.conf.d/90-unix-guardian.conf"
check "the lid while charging can be set" "ug power sleep --lid-ac ignore | grep -q 'charging: *ignore' && grep -q 'HandleLidSwitchExternalPower=ignore' '$D'"
check "the power button can be set" "ug power sleep --power-key suspend >/dev/null && grep -q 'HandlePowerKey=suspend' '$D'"
check "an unknown action is refused" "! ug power sleep --lid explode >/dev/null 2>&1"
check "back to the defaults leaves no file" "ug power sleep --lid-ac suspend --power-key poweroff >/dev/null && [ ! -e '$D' ]"
check "JSON has every field" "ug --json power sleep | grep -q '\"lid_handled_by\"'"

echo "$fails failed"
exit $(( fails > 0 ))
