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

# Nothing here may be D-Bus-activated from an installed copy: with Unix Guardian
# installed, a test bus that uses the standard service directories starts
# /usr/local/bin/ug-agent (with the user's real XDG directories) the moment the
# test's own agent is not answering, and the test then writes into the real
# playtime database. Found the hard way; see 05-findings.md.
if busctl --user list --activatable 2>/dev/null | grep -q org.unixguardian; then
  echo "FAIL  the test bus can activate an installed Unix Guardian service; run with scripts/test-bus.sh" >&2
  exit 1
fi
B="${1:-$PWD/target/debug}"
W="$(mktemp -d)"
export XDG_DATA_HOME="$W/data" XDG_CONFIG_HOME="$W/cfg"
mkdir -p "$XDG_CONFIG_HOME/unix-guardian/profiles" "$W/bin"
printf 'cpu_preferred_cores = true\npause_apps = ["ugfakechat"]\n' > "$XDG_CONFIG_HOME/unix-guardian/profiles/competitive.toml"
ln -s "$(command -v sleep)" "$W/bin/ugfakechat"
# A copy, not a link: an app is matched by the program actually running.
cp "$(command -v sleep)" "$W/bin/ugfakeeditor"
# A home of its own for the agent: no real library, and a Steam account
# whose own count for CS2 is ten hours.
mkdir -p "$W/home/.local/share/Steam/steamapps" "$W/home/.local/share/Steam/userdata/1/config"
echo '"libraryfolders" {}' > "$W/home/.local/share/Steam/steamapps/libraryfolders.vdf"
# Laid out as Steam writes it: one key per line, tab-indented.
printf '"UserLocalConfigStore"\n{\n\t"Software"\n\t{\n\t\t"Valve"\n\t\t{\n\t\t\t"Steam"\n\t\t\t{\n\t\t\t\t"apps"\n\t\t\t\t{\n\t\t\t\t\t"730"\n\t\t\t\t\t{\n\t\t\t\t\t\t"LastPlayed"\t\t"1790000000"\n\t\t\t\t\t\t"Playtime"\t\t"600"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n}\n' \
  > "$W/home/.local/share/Steam/userdata/1/config/localconfig.vdf"

# Everything this script starts carries the token, so an installed agent
# watching the same user's processes ignores the fake games (and this run's
# agent ignores the user's real ones).
export UG_TEST_RUN="e2e-$$-$RANDOM"
fails=0
check() { if eval "$2"; then echo "PASS  $1"; else echo "FAIL  $1"; fails=$((fails+1)); fi; }
state() { awk '{print $3}' "/proc/$1/stat" 2>/dev/null; }
until_() { local d=$((SECONDS+$1)); shift; until eval "$@"; do (( SECONDS < d )) || return 1; sleep 0.2; done; }

# The old card switch's session-wide file (it broke KDE, 2026-10-04): the
# agent moves it aside at start and leaves environment.d.disabled alone.
mkdir -p "$XDG_CONFIG_HOME/environment.d" "$XDG_CONFIG_HOME/environment.d.disabled"
printf '__NV_PRIME_RENDER_OFFLOAD=1\n__GLX_VENDOR_LIBRARY_NAME=nvidia\nUG_DEFAULT_GPU=0000:01:00.0\n' \
  | tee "$XDG_CONFIG_HOME/environment.d/90-unix-guardian-gpu.conf" > "$XDG_CONFIG_HOME/environment.d.disabled/90-unix-guardian-gpu.conf"
printf '# Written by Unix Guardian: load MangoHud into Vulkan games.\nMANGOHUD=1\n' > "$XDG_CONFIG_HOME/environment.d/90-unix-guardian-mangohud.conf"
"$W/bin/ugfakechat" 300 & chat=$!
HOME="$W/home" "$B/ug-agent" --quiet --log warn & agent=$!
trap 'kill $agent $chat ${game:-} ${other:-} ${editor:-} ${fakesteam:-} 2>/dev/null; rm -rf "$W"' EXIT
until_ 15 "'$B/ug' game running >/dev/null 2>&1" || { echo "FAIL  agent did not start"; exit 1; }

check "the old session-wide GPU and MangoHud files are moved out of environment.d" "[ -z \"\$(ls -A '$XDG_CONFIG_HOME/environment.d')\" ]"
check "they are kept, not deleted" "grep -q __NV_PRIME '$XDG_CONFIG_HOME'/unix-guardian/retired/90-unix-guardian-gpu.conf.* && grep -q MANGOHUD '$XDG_CONFIG_HOME'/unix-guardian/retired/90-unix-guardian-mangohud.conf.*"
check "environment.d.disabled is left alone" "[ -f '$XDG_CONFIG_HOME/environment.d.disabled/90-unix-guardian-gpu.conf' ]"
check "the choice is kept for ug run" "grep -qx 0000:01:00.0 '$XDG_CONFIG_HOME/unix-guardian/default-gpu'"

# The user unit and the XDG autostart fallback may both start an agent; the
# second must step aside without failing, or systemd would keep restarting it.
check "a second agent in the same session exits cleanly" "timeout 10 '$B/ug-agent' --quiet --log error"
check "and the first one is still serving" "'$B/ug' game running >/dev/null 2>&1"

SteamAppId=730 bash -c 'exec -a cs2 sleep 120' & game=$!
check "a Steam game is detected by its SteamAppId" "until_ 8 \"'$B/ug' game running | grep -q 'Counter-Strike 2'\""
check "it runs with its database profile (competitive)" "'$B/ug' game running | grep -q 'competitive profile'"
check "ug profile set switches the running game's profile" "'$B/ug' profile set steam:730 quiet >/dev/null && '$B/ug' game running | grep -q 'quiet profile'"
check "the choice is listed under that profile" "'$B/ug' profile list --json | python3 -c 'import json,sys; assert [p for p in json.load(sys.stdin) if p[\"id\"]==\"quiet\" and p[\"games\"]==[\"steam:730\"]]'"
check "an unknown profile is refused with the choices" "out=\$('$B/ug' profile set steam:730 turbo 2>&1); [ \$? -ne 0 ] && grep -q 'available:.*competitive' <<<\"\$out\""
check "ug profile option changes one setting and keeps the rest of the file" "'$B/ug' profile option competitive cpu_priority off >/dev/null && '$B/ug' profile list --json | python3 -c 'import json,sys; c=[p for p in json.load(sys.stdin) if p[\"id\"]==\"competitive\"][0]; assert not c[\"cpu_priority\"] and c[\"pause_apps\"]==[\"ugfakechat\"] and c[\"cpu_preferred_cores\"]'"
check "ug profile reset returns to the database profile" "'$B/ug' profile reset steam:730 >/dev/null && '$B/ug' game running | grep -q 'competitive profile'"
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
check "the agent serves monitoring history (empty without a daemon)" "'$B/ug' history --hours 1 >/dev/null && '$B/ug' traffic >/dev/null"
check "playtime exports as CSV, one row per session" "'$B/ug' playtime --csv | grep -q '^Counter-Strike 2,steam:730,' && [ \"\$('$B/ug' playtime --csv | head -1)\" = game,key,started,ended,minutes ]"
check "the CSV export ends cleanly when the reader stops early" "'$B/ug' playtime --csv 2>&1 | head -1 >/dev/null; [ \"\${PIPESTATUS[0]}\" = 0 ]"

# Tracked apps: time counted, never a game, the choice kept.
"$W/bin/ugfakeeditor" 120 & editor=$!
check "a program can be tracked as an app" "'$B/ug' playtime track ugfakeeditor --name 'Fake Editor' >/dev/null"
check "its time is counted under Apps" "until_ 8 \"'$B/ug' playtime | sed -n '/^Apps/,\\\$p' | grep -q 'Fake Editor'\""
check "and it is not a game" "'$B/ug' game running | grep -q 'No game is running'"
check "the choice is kept for the next login" "grep -q ugfakeeditor '$XDG_CONFIG_HOME/unix-guardian/playtime.json'"
check "untracking keeps the time counted" "'$B/ug' playtime untrack ugfakeeditor >/dev/null && '$B/ug' playtime | grep -q 'Fake Editor'"

# Steam import: all time becomes Steam's ten hours, not ten plus what was
# recorded here; a week stays what was recorded; undo takes it back.
pt_json() { "$B/ug" playtime --json "$@" | python3 -c 'import json,sys; print(sum(e["seconds"] for e in json.load(sys.stdin) if e["key"]=="steam:730"))'; }
check "Steam's playtime is imported" "'$B/ug' playtime import-steam | grep -q 'across 1 games'"
check "all time is Steam's count, not counted twice" "[ \"\$(pt_json)\" = 36000 ]"
check "a week has only what was recorded here" "[ \"\$(pt_json --week)\" -lt 100 ]"
check "importing again does not add up" "'$B/ug' playtime import-steam >/dev/null && [ \"\$(pt_json)\" = 36000 ]"
check "undo removes the import" "'$B/ug' playtime import-steam --undo >/dev/null && [ \"\$(pt_json)\" -lt 100 ]"
# Steam launch options: shown first, refused while Steam runs, written with a
# backup, and undone to the byte. The GPU part needs a GPU (not in CI).
LC="$W/home/.local/share/Steam/userdata/1/config/localconfig.vdf"
cp "$LC" "$W/localconfig.before"
check "launch options are shown before anything is written" "'$B/ug' game launch 730 | grep -q 'now:    (none)' && cmp -s '$LC' '$W/localconfig.before'"
if ls /sys/class/drm/ 2>/dev/null | grep -qE '^card[0-9]+$'; then
  cp "$(command -v sleep)" "$W/bin/steam"
  "$W/bin/steam" 60 & fakesteam=$!
  check "nothing is written while Steam runs" "out=\$('$B/ug' game launch 730 --gpu discrete --apply 2>&1); grep -q 'Close Steam' <<<\"\$out\" && cmp -s '$LC' '$W/localconfig.before'"
  kill $fakesteam; wait $fakesteam 2>/dev/null
  if pgrep -u "$(id -u)" -x steam >/dev/null || pgrep -u "$(id -u)" -x steamwebhelper >/dev/null; then
    echo "SKIP  writing launch options (the real Steam is running)"
  else
    check "the GPU choice is written, with a backup" "'$B/ug' game launch 730 --gpu discrete --apply | grep -q 'backed up' && grep -q 'LaunchOptions' '$LC' && ls '$XDG_DATA_HOME'/unix-guardian/backups/localconfig-1-*.vdf >/dev/null"
    check "undo puts Steam's file back to the byte" "'$B/ug' game launch 730 --undo >/dev/null && cmp -s '$LC' '$W/localconfig.before'"
  fi
else
  echo "SKIP  writing launch options (no GPU here)"
fi

# Global shortcuts: a test bus has no portal, so turning them on says what
# to bind instead; the actions themselves work through `ug shortcut <id>`.
check "shortcuts are off until turned on" "'$B/ug' shortcut | grep -q 'off (ug shortcut on)'"
check "without a portal they are unavailable, with the commands to bind" "'$B/ug' shortcut on | grep -q 'ug shortcut toggle-gamemode'"
check "the choice is remembered" "grep -q true '$XDG_CONFIG_HOME/unix-guardian/shortcuts.json' && '$B/ug' shortcut off >/dev/null && grep -q false '$XDG_CONFIG_HOME/unix-guardian/shortcuts.json'"
check "game mode can be turned on by hand without a game" "'$B/ug' shortcut toggle-gamemode | grep -q 'Game mode on' && '$B/ug' shortcut | grep -q 'turned on by hand'"
check "and off again" "'$B/ug' shortcut toggle-gamemode | grep -q 'Game mode off' && ! '$B/ug' shortcut | grep -q 'by hand'"
check "the overlay shortcut says when the overlay is not set up" "out=\$('$B/ug' shortcut toggle-overlay 2>&1); [ \$? -ne 0 ] && grep -q 'not set up' <<<\"\$out\""
check "an unknown shortcut is refused with the known ones" "out=\$('$B/ug' shortcut fly 2>&1); [ \$? -ne 0 ] && grep -q 'open-window' <<<\"\$out\""

# Settings: shown, changed one at a time, kept in settings.json, and an
# out-of-range value refused.
check "settings show the defaults" "'$B/ug' settings | grep -q 'Look for games every   2 s'"
check "settings change only what is given" "'$B/ug' settings --scan-interval 3 --close-window-on-game on | grep -q 'every   3 s' && '$B/ug' settings | grep -q 'Notifications          on'"
check "settings are kept in settings.json" "grep -q '\"scan_interval_s\": 3' '$XDG_CONFIG_HOME/unix-guardian/settings.json'"
SteamAppId=730 bash -c 'exec -a cs2 sleep 20' & game=$!
check "games are still found at the new interval" "until_ 8 \"'$B/ug' game running | grep -q 'Counter-Strike 2'\""
kill $game; wait $game 2>/dev/null
check "an interval out of range is refused" "! '$B/ug' settings --scan-interval 60 2>/dev/null && '$B/ug' settings | grep -q 'every   3 s'"

echo; [ "$fails" -eq 0 ] && echo "agent-e2e: OK" || { echo "agent-e2e: $fails failed"; exit 1; }
