#!/usr/bin/env bash
# End-to-end check of the agent: detection, preferred-core pinning, pausing
# apps, GameMode registration and playtime — with no daemon and no root.
#
#   cargo build --workspace && dbus-run-session -- ./scripts/agent-e2e.sh
#
# Runs on a private bus with scratch XDG directories, so the desktop session and
# the user's real playtime database are never touched. Exits non-zero on any
# failed check. Pinning is only checked on CPUs that have preferred cores
# (Intel hybrid or AMD X3D).
set -uo pipefail
B="${1:-$PWD/target/debug}"
W="$(mktemp -d)"
export XDG_DATA_HOME="$W/data" XDG_CONFIG_HOME="$W/cfg"
mkdir -p "$XDG_CONFIG_HOME/unix-guardian/profiles" "$W/bin"
printf 'cpu_preferred_cores = true\npause_apps = ["ugfakechat"]\n' > "$XDG_CONFIG_HOME/unix-guardian/profiles/competitive.toml"
ln -s "$(command -v sleep)" "$W/bin/ugfakechat"

fails=0
check() { if eval "$2"; then echo "PASS  $1"; else echo "FAIL  $1"; fails=$((fails+1)); fi; }
state() { awk '{print $3}' "/proc/$1/stat" 2>/dev/null; }
until_() { local d=$((SECONDS+$1)); shift; until eval "$@"; do (( SECONDS < d )) || return 1; sleep 0.2; done; }

"$W/bin/ugfakechat" 300 & chat=$!
"$B/ug-agent" --quiet --log warn & agent=$!
trap 'kill $agent $chat ${game:-} ${other:-} 2>/dev/null; rm -rf "$W"' EXIT
until_ 15 "'$B/ug' game running >/dev/null 2>&1" || { echo "FAIL  agent did not start"; exit 1; }

SteamAppId=730 bash -c 'exec -a cs2 sleep 120' & game=$!
check "a Steam game is detected by its SteamAppId" "until_ 8 \"'$B/ug' game running | grep -q 'Counter-Strike 2'\""
check "it runs with its database profile (competitive)" "'$B/ug' game running | grep -q 'competitive profile'"
p_cores=$(cat /sys/devices/cpu_core/cpus 2>/dev/null || true)
if [ -n "$p_cores" ]; then
  check "it is pinned to the P-cores ($p_cores)" "until_ 4 \"grep -q 'Cpus_allowed_list:[[:space:]]*$p_cores\$' /proc/$game/status\""
else
  echo "SKIP  pinning (this CPU has no preferred cores)"
fi
check "a listed app is paused while playing" "until_ 4 \"[ \\\"\$(state $chat)\\\" = T ]\""

sleep 120 & other=$!
check "GameMode RegisterGame is accepted" "busctl --user call com.feralinteractive.GameMode /com/feralinteractive/GameMode com.feralinteractive.GameMode RegisterGame i $other | grep -q 'i 0'"
check "a registered process becomes a game" "until_ 6 \"'$B/ug' game running | grep -q 'game mode'\""
check "QueryStatus reports it active and registered" "busctl --user call com.feralinteractive.GameMode /com/feralinteractive/GameMode com.feralinteractive.GameMode QueryStatus i $other | grep -q 'i 2'"

kill "$game" "$other"; wait "$game" "$other" 2>/dev/null
check "both games are seen to stop" "until_ 10 \"'$B/ug' game running | grep -q 'No game is running'\""
check "the paused app runs again" "until_ 4 \"[ \\\"\$(state $chat)\\\" != T ]\""
check "playtime was recorded for both" "'$B/ug' playtime | grep -q 'Counter-Strike 2' && '$B/ug' playtime | grep -q 'sleep'"

echo; [ "$fails" -eq 0 ] && echo "agent-e2e: OK" || { echo "agent-e2e: $fails failed"; exit 1; }
