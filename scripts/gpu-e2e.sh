#!/usr/bin/env bash
# End-to-end check of GPU tuning against a development daemon's simulated
# GPU: trials, keeping, undoing, and the crash watchdog. Nothing touches a
# real card.
#
#   cargo build --workspace && ./scripts/test-bus.sh ./scripts/gpu-e2e.sh
set -uo pipefail
if busctl --user list --activatable 2>/dev/null | grep -q org.unixguardian; then
  echo "FAIL  the test bus can activate an installed Unix Guardian service; run with scripts/test-bus.sh" >&2
  exit 1
fi
B="${1:-$PWD/target/debug}"
W="$(mktemp -d)"
fails=0
check() { if eval "$2"; then echo "PASS  $1"; else echo "FAIL  $1"; fails=$((fails+1)); fi; }
until_() { local d=$((SECONDS+$1)); shift; until eval "$@"; do (( SECONDS < d )) || return 1; sleep 0.3; done; }
ug() { "$B/ug" --session "$@"; }
start() { "$B/ug-daemon" --session --state-dir "$W/state" --log warn 2>>"$W/daemon.log" & daemon=$!; until_ 20 "ug gpu tune >/dev/null 2>&1"; }
start || { echo "FAIL  daemon did not start (no discrete GPU to simulate?)"; cat "$W/daemon.log"; exit 1; }
trap 'kill $daemon 2>/dev/null; rm -rf "$W"' EXIT

check "limits are shown" "ug gpu tune | grep -q 'allowed 35–95 W'"
check "a value out of range is refused" "! ug gpu tune --power 200 </dev/null >/dev/null 2>&1"
out="$(echo n | ug gpu tune --power 45 --core 100 2>&1)"
check "answering no undoes a trial at once" "grep -q Undone <<<\"\$out\" && ug gpu tune | grep -q 'power limit   60 W'"
out="$(echo y | ug gpu tune --power 45 --core 100 2>&1)"
check "answering yes keeps it" "grep -q Kept <<<\"\$out\" && ug gpu tune | grep -q 'kept by Unix Guardian: 45 W, core +100'"
kill "$daemon"; wait "$daemon" 2>/dev/null; start
check "a kept setting survives a restart" "until_ 5 'ug gpu tune | grep -q \"power limit   45 W\"'"
# A trial in progress when the service dies is not applied again.
( sleep 60 | ug gpu tune --power 90 --core 250 >/dev/null 2>&1 & ); sleep 2
kill -9 "$daemon"; wait "$daemon" 2>/dev/null; start
check "a setting tried during a crash is not applied again" "ug gpu tune | grep -q 'NOT applied again: .*90 W, core +250'"
check "and the kept one is still in force" "until_ 5 'ug gpu tune | grep -q \"power limit   45 W\"'"
ug gpu reset >/dev/null
check "reset goes back to the card's defaults" "ug gpu tune | grep -q 'power limit   60 W' && ! ug gpu tune | grep -q 'kept by'"

if [ "$fails" -gt 0 ]; then echo "--- daemon log"; cat "$W/daemon.log"; fi
echo "$fails failure(s)"
exit $((fails > 0))
