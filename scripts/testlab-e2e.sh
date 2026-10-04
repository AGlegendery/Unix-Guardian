#!/usr/bin/env bash
# End-to-end check of the test lab (`ug testlab`), without root.
#
#   cargo build --workspace && ./scripts/test-bus.sh ./scripts/testlab-e2e.sh
#
# The daemon runs on a private bus against a scratch copy of the development
# laptop's captured /sys (game mode and the wake alarm write only there; power
# actions and internet rules are simulated), the agent with scratch XDG
# directories and a scratch home. Checks that the lab turns things on, puts
# every one of them back, removes personal details from the report, and that
# `ug testlab --recover` puts back what an interrupted run left.
set -uo pipefail

if busctl --user list --activatable 2>/dev/null | grep -q org.unixguardian; then
  echo "FAIL  the test bus can activate an installed Unix Guardian service; run with scripts/test-bus.sh" >&2
  exit 1
fi
B="${1:-$PWD/target/debug}"
FIXTURE="${FIXTURE:-crates/ug-testfs/fixtures/dev-laptop-rtx3050}"
W="$(mktemp -d)"
cp -a "$FIXTURE" "$W/root"
for n in 0 1 2 3; do
  d="$W/root/sys/devices/system/cpu/cpufreq/policy$n"; mkdir -p "$d"
  echo powersave > "$d/scaling_governor"
  echo "performance powersave" > "$d/scaling_available_governors"
  echo balance_power > "$d/energy_performance_preference"
done
mkdir -p "$W/root/proc/sys/vm" "$W/home"
echo 60 > "$W/root/proc/sys/vm/swappiness"
echo 65530 > "$W/root/proc/sys/vm/max_map_count"
echo 20 > "$W/root/proc/sys/vm/compaction_proactiveness"
export XDG_DATA_HOME="$W/data" XDG_CONFIG_HOME="$W/cfg"
export UG_TEST_RUN="e2e-$$-$RANDOM"
fails=0
check() { if eval "$2"; then echo "PASS  $1"; else echo "FAIL  $1"; fails=$((fails+1)); fi; }
until_() { local d=$((SECONDS+$1)); shift; until eval "$@"; do (( SECONDS < d )) || return 1; sleep 0.2; done; }
gov() { cat "$W/root/sys/devices/system/cpu/cpufreq/policy0/scaling_governor"; }
line() { grep -F "$1" "$W/report.txt"; }

"$B/ug-daemon" --session --sysroot "$W/root" --state-dir "$W/state" --log warn 2>"$W/daemon.log" & daemon=$!
until_ 20 "'$B/ug' --session gpu active >/dev/null 2>&1" || { echo "FAIL  daemon did not start"; exit 1; }
HOME="$W/home" "$B/ug-agent" --daemon-session --quiet --log warn 2>"$W/agent.log" & agent=$!
trap 'kill $daemon $agent 2>/dev/null; rm -rf "$W"' EXIT
until_ 15 "'$B/ug' game running >/dev/null 2>&1" || { echo "FAIL  agent did not start"; exit 1; }

check "without a terminal it asks for --yes instead of starting" "! '$B/ug' --session testlab </dev/null >/dev/null 2>&1 && [ ! -e '$XDG_DATA_HOME/unix-guardian/testlab-undo.json' ]"
"$B/ug" --session testlab --yes --out "$W/report.txt" >"$W/stdout.txt" 2>"$W/progress.txt"
check "it runs to the end and saves the report" "grep -q '^RESULT: ' '$W/report.txt' && grep -q 'Send this report to unixguardian@atomicmail.io' '$W/report.txt'"
check "the printed report is the saved one" "cmp -s '$W/report.txt' '$W/stdout.txt'"
check "progress reached 100 %" "grep -q '100 %' '$W/progress.txt'"
check "the service and agent were reached" "line '[PASS] service: reachable' && line '[PASS] agent: reachable'"
check "game mode went on" "line '[PASS] game mode: turn on'"
check "and off, restoring the CPU settings" "line '[PASS] game mode: turn off and restore' && line '[PASS] game mode: CPU settings back as they were' && [ \"\$(gov)\" = powersave ]"
check "a profile was created, changed and deleted" "line '[PASS] profiles: create, change and delete one' && [ ! -e '$XDG_CONFIG_HOME/unix-guardian/profiles/ug-testlab-probe.toml' ]"
check "an app was tracked and untracked" "line '[PASS] playtime: track an app and stop' && ! '$B/ug' playtime apps 2>/dev/null | grep -q ug-testlab-probe"
check "a power action was scheduled and cancelled" "line '[PASS] power: schedule and cancel' && ! '$B/ug' --session power list 2>/dev/null | grep -q ug-testlab-probe"
check "the wake alarm was set and cleared" "line '[PASS] power: wake alarm set and clear' || line 'power: wake alarm — this machine has no'"
check "an internet rule was added and removed" "line '[PASS] internet control: add and remove a block rule' || line '[WARN] internet control: status'"
check "the CPU benchmark ran" "line 'CPU all threads' && line '[PASS] cpu: arithmetic check'"
check "the driver check ran" "line '[PASS] drivers: hardware and driver check'"
check "everything was put back" "line 'Everything the test turned on was turned off again.' && [ ! -e '$XDG_DATA_HOME/unix-guardian/testlab-undo.json' ]"
check "manual tests are listed" "line 'NOT TESTED AUTOMATICALLY' && line 'log-out, log-in or restart'"
check "no FAIL lines" "! grep -F '[FAIL]' '$W/report.txt'"
host="$(cat /proc/sys/kernel/hostname)"
# A host named after its distribution (parrot, debian) keeps that word: the
# report must still say which distribution it ran on.
distro="$(. /etc/os-release; echo "$ID")"
[ "$host" = "$distro" ] && host="$USER"
check "the user name, host name and home folder are not in the report" "! grep -qw -- '$USER' '$W/report.txt' && ! grep -qw -- '$host' '$W/report.txt' && ! grep -qF -- '$HOME' '$W/report.txt'"
check "no MAC address or disk UUID in the report" "! grep -Eiq '([0-9a-f]{2}:){5}[0-9a-f]{2}|[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}' '$W/report.txt'"

# An interrupted run: a profile it made and its undo note are left behind.
"$B/ug" profile new ug-testlab-probe >/dev/null 2>&1
printf '[{"undo":"delete-profile","id":"ug-testlab-probe"}]' > "$XDG_DATA_HOME/unix-guardian/testlab-undo.json"
check "(the interrupted run's profile exists)" "'$B/ug' profile list | grep -q ug-testlab-probe"
check "ug testlab --recover puts it back" "'$B/ug' --session testlab --recover 2>&1 | grep -q 'test profile ug-testlab-probe deleted' && ! '$B/ug' profile list | grep -q ug-testlab-probe && [ ! -e '$XDG_DATA_HOME/unix-guardian/testlab-undo.json' ]"

if (( fails )); then
  echo "--- report"; cat "$W/report.txt"; echo "--- progress"; tail -5 "$W/progress.txt"
  echo "--- daemon log"; tail -20 "$W/daemon.log"; echo "--- agent log"; tail -20 "$W/agent.log"
fi
echo "$fails failed"
exit $(( fails > 0 ))
