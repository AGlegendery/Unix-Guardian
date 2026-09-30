#!/usr/bin/env bash
# End-to-end check of game mode, agent → daemon, without root.
#
#   cargo build --workspace && dbus-run-session -- ./scripts/gamemode-e2e.sh
#
# The daemon runs on a private bus against a scratch copy of the development
# laptop's captured /sys, with writable CPU-frequency and sysctl files, so every
# change lands in the copy. Development runs never touch the real system's
# services; this script also checks the journal to prove no polkit prompt or
# unit change happened on the real machine. Exits non-zero on any failure.
set -uo pipefail
B="${1:-$PWD/target/debug}"
FIXTURE="${FIXTURE:-crates/ug-testfs/fixtures/dev-laptop-rtx3050}"
W="$(mktemp -d)"
since="$(date '+%Y-%m-%d %H:%M:%S')"
cp -a "$FIXTURE" "$W/root"
for n in 0 1 2 3; do
  d="$W/root/sys/devices/system/cpu/cpufreq/policy$n"; mkdir -p "$d"
  echo powersave > "$d/scaling_governor"
  echo "performance powersave" > "$d/scaling_available_governors"
  echo balance_power > "$d/energy_performance_preference"
done
mkdir -p "$W/root/proc/sys/vm"
echo 60 > "$W/root/proc/sys/vm/swappiness"
echo 65530 > "$W/root/proc/sys/vm/max_map_count"
echo 20 > "$W/root/proc/sys/vm/compaction_proactiveness"
export XDG_DATA_HOME="$W/data" XDG_CONFIG_HOME="$W/cfg"

fails=0
check() { if eval "$2"; then echo "PASS  $1"; else echo "FAIL  $1"; fails=$((fails+1)); fi; }
until_() { local d=$((SECONDS+$1)); shift; until eval "$@"; do (( SECONDS < d )) || return 1; sleep 0.2; done; }
active() { busctl --user get-property org.unixguardian.Daemon1 /org/unixguardian/Daemon1 org.unixguardian.Daemon1.GameMode Active 2>/dev/null; }
gov() { cat "$W/root/sys/devices/system/cpu/cpufreq/policy0/scaling_governor"; }
epp() { cat "$W/root/sys/devices/system/cpu/cpufreq/policy0/energy_performance_preference"; }
swp() { cat "$W/root/proc/sys/vm/swappiness"; }
original() { [ "$(gov)" = powersave ] && [ "$(epp)" = balance_power ] && [ "$(swp)" = 60 ]; }

"$B/ug-daemon" --session --sysroot "$W/root" --state-dir "$W/state" --log warn 2>"$W/daemon.log" & daemon=$!
until_ 20 "'$B/ug' --session gpu active >/dev/null 2>&1" || { echo "FAIL  daemon did not start"; exit 1; }
"$B/ug-agent" --daemon-session --quiet --log info 2>"$W/agent.log" & agent=$!
trap 'kill $daemon $agent ${game:-} 2>/dev/null; rm -rf "$W"' EXIT
until_ 15 "'$B/ug' game running >/dev/null 2>&1" || { echo "FAIL  agent did not start"; exit 1; }

check "starts from the machine's own settings" original
SteamAppId=730 bash -c 'exec -a cs2 sleep 120' & game=$!
check "a game turns game mode on" "until_ 8 \"active | grep -q true\""
check "CPU governor and EPP set to performance" "until_ 3 \"[ \\\"\$(gov)/\$(epp)\\\" = performance/performance ]\""
check "swappiness lowered to 10" "[ \"\$(swp)\" = 10 ]"
check "the undo list is on disk before anything else" "[ -s '$W/state/gamemode.json' ]"

# A second daemon (a stray manual start, a unit started twice) must neither take
# the bus name nor run crash recovery on the undo list the first one is using.
check "a second daemon refuses to start while one is running" "timeout 10 '$B/ug-daemon' --session --sysroot '$W/root' --state-dir '$W/state' --log error; r=\$?; [ \$r -ne 0 ] && [ \$r -ne 124 ]"
check "and the running game keeps its settings" "active | grep -q true && [ \"\$(gov)\" = performance ] && [ \"\$(swp)\" = 10 ]"

kill "$game"; wait "$game" 2>/dev/null
check "the game ending turns game mode off" "until_ 10 \"active | grep -q false\""
check "every setting is exactly as before" original

SteamAppId=730 bash -c 'exec -a cs2 sleep 120' & game=$!
until_ 8 "active | grep -q true"
kill -9 "$agent"; wait "$agent" 2>/dev/null
check "a killed agent's game mode is reverted by the daemon" "until_ 6 \"active | grep -q false\" && original"

check "only the fake game was ever detected" "! grep -E 'game started' '$W/agent.log' | grep -vq 'Counter-Strike 2'"
if command -v journalctl >/dev/null; then
  check "no polkit prompt appeared on the real desktop" "[ \"\$(journalctl --since '$since' --no-pager 2>/dev/null | grep -c 'polkit-.*REQUEST')\" = 0 ]"
  check "no real service was stopped or started" "! journalctl --since '$since' --no-pager 2>/dev/null | grep -qE '(Stopped|Started) .*(apt-daily|fstrim|man-db|logrotate|e2scrub|fwupd-refresh|plocate|dnf-makecache)'"
fi

echo; [ "$fails" -eq 0 ] && echo "gamemode-e2e: OK" || { echo "gamemode-e2e: $fails failed (logs: daemon.log/agent.log removed on exit; rerun with 'trap - EXIT')"; exit 1; }
