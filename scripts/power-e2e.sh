#!/usr/bin/env bash
# End-to-end check of power control: capabilities, "now", the schedule, and
# that the schedule survives the daemon being killed.
#
#   cargo build --workspace && ./scripts/test-bus.sh ./scripts/power-e2e.sh
#
# The daemon runs with --session, where power actions are only simulated and
# recorded (the Simulated property) — nothing here can shut the machine down.
set -uo pipefail
if busctl --user list --activatable 2>/dev/null | grep -q org.unixguardian; then
  echo "FAIL  the test bus can activate an installed Unix Guardian service; run with scripts/test-bus.sh" >&2
  exit 1
fi
B="${1:-$PWD/target/debug}"
W="$(mktemp -d)"
fails=0
check() { if eval "$2"; then echo "PASS  $1"; else echo "FAIL  $1"; fails=$((fails+1)); fi; }
until_() { local d=$((SECONDS+$1)); shift; until eval "$@"; do (( SECONDS < d )) || return 1; sleep 0.2; done; }
ug() { "$B/ug" --session "$@"; }
simulated() { busctl --user get-property org.unixguardian.Daemon1 /org/unixguardian/Daemon1 org.unixguardian.Daemon1.Power Simulated 2>/dev/null; }
start_daemon() { "$B/ug-daemon" --session --state-dir "$W/state" --log warn 2>>"$W/daemon.log" & daemon=$!; until_ 20 "ug power list >/dev/null 2>&1"; }

start_daemon || { echo "FAIL  daemon did not start"; exit 1; }
trap 'kill $daemon ${mon:-} 2>/dev/null; rm -rf "$W"' EXIT

check "every action is offered in a development run" "[ \"\$(ug power caps | grep -c ' available$')\" = 7 ]"
check "'now' without --yes does nothing" "! ug power now reboot >/dev/null 2>&1 && simulated | grep -q 'as 0'"
check "'now --yes' performs it (simulated)" "ug power now reboot --yes >/dev/null && simulated | grep -q '\"reboot\"'"
check "an unknown action is refused with the choices" "out=\$(ug power now explode --yes 2>&1); grep -q 'power-off' <<<\"\$out\""

busctl --user monitor --json=short org.unixguardian.Daemon1 >"$W/signals" 2>/dev/null & mon=$!
sleep 0.5
check "a job can be scheduled a few seconds ahead" "ug power at suspend --in 3s --note e2e | grep -q 'Scheduled #'"
check "it warns before running (Upcoming signal)" "until_ 4 \"grep -q '\\\"member\\\":\\\"Upcoming\\\"' '$W/signals'\""
check "it runs on time" "until_ 8 \"simulated | grep -q suspend\""
check "a one-off job leaves the queue after running" "ug power list | grep -q 'Nothing scheduled'"

ug power at power-off --in 1h >/dev/null
check "a job an hour away is listed" "ug power list | grep -q power-off"
kill -9 "$daemon"; wait "$daemon" 2>/dev/null
start_daemon
check "it survives the daemon being killed" "ug power list | grep -q power-off"
id=$(ug power list | awk '/power-off/{print substr($1,2)}')
check "it can be cancelled by number" "ug power cancel $id | grep -q Cancelled && ug power list | grep -q 'Nothing scheduled'"
check "cancelling a missing number fails" "! ug power cancel 999 >/dev/null 2>&1"
check "a time in the past is refused" "! busctl --user call org.unixguardian.Daemon1 /org/unixguardian/Daemon1 org.unixguardian.Daemon1.Power Schedule stss reboot 1000 once x >/dev/null 2>&1"
check "a wake alarm can be set and is listed" "ug power wake --in 8h >/dev/null && out=\$(ug power list) && grep -q 'Wake alarm' <<<\"\$out\""
check "and cleared" "ug power wake --clear >/dev/null && out=\$(ug power list) && ! grep -q 'Wake alarm' <<<\"\$out\""
check "a wake alarm in the past is refused" "! ug power wake --in 1s >/dev/null 2>&1"
# Conditions. The fake program is named ugwatchme so no real process matches.
ln -s "$(command -v sleep)" "$W/ugwatchme"
"$W/ugwatchme" 300 & watched=$!
check "waiting for a game when none runs is refused" "! ug power when power-off --game-over >/dev/null 2>&1"
check "waiting for a program that is not running is refused" "! ug power when power-off --process nosuchprog >/dev/null 2>&1"
check "a program's exit can be waited for" "ug power when reboot --process ugwatchme | grep -q 'Armed #w'"
check "it is listed" "out=\$(ug power list) && grep -q 'when ugwatchme exits' <<<\"\$out\""
sleep 12
# The daemon was restarted above, so the simulated log started afresh.
reboots() { simulated | grep -o '"reboot"' | wc -l; }
check "it does not fire while the program runs" "[ \"\$(reboots)\" = 0 ]"
kill "$watched"; wait "$watched" 2>/dev/null
check "it fires after the program has been gone for its window" "until_ 45 '[ \"\$(reboots)\" = 1 ]'"
check "a fired condition leaves the list" "out=\$(ug power list) && ! grep -q '#w' <<<\"\$out\""
check "a CPU threshold out of range is refused" "! ug power when suspend --cpu-idle --below 500 >/dev/null 2>&1"
check "a condition can be removed" "id=\$(ug power when suspend --download-done --minutes 30 | sed -n 's/Armed #w\\([0-9]*\\).*/\\1/p') && ug power unwatch \$id >/dev/null && out=\$(ug power list) && ! grep -q '#w' <<<\"\$out\""
real_types="$(cat /sys/class/power_supply/*/charge_types 2>/dev/null; cat /sys/class/power_supply/*/charge_control_end_threshold 2>/dev/null)"
check "battery status answers" "ug battery >/dev/null"
if ug battery | grep -q 'not available'; then
  echo "SKIP  battery protection (not supported on this machine)"
else
  check "battery protection can be turned on" "ug battery protect --limit 80 | grep -q 'Protection    on'"
  # Already full, full-once goes straight back to protecting.
  check "full-once and off are accepted" "out=\$(ug battery full-once) && grep -qE 'paused|Protection    on' <<<\"\$out\" && out=\$(ug battery off) && grep -q 'off' <<<\"\$out\""
fi
check "a limit out of range is refused" "! ug battery protect --limit 10 >/dev/null 2>&1"
check "the real battery settings were not touched" "[ \"\$(cat /sys/class/power_supply/*/charge_types 2>/dev/null; cat /sys/class/power_supply/*/charge_control_end_threshold 2>/dev/null)\" = \"\$real_types\" ]"
check "nothing was ever done for real (only simulated)" "grep -q 'power actions are simulated' '$W/daemon.log'"

echo; [ "$fails" -eq 0 ] && echo "power-e2e: OK" || { echo "power-e2e: $fails failed"; exit 1; }
