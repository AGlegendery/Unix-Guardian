#!/usr/bin/env bash
# End-to-end check of the threshold alerts: the agent reads a scratch tree
# with a hot CPU and full memory (UG_ALERTS_SYSROOT, honoured only in a test
# run), a stand-in notification server records what is shown, and `ug alert`
# drives it. A development daemon on an empty sysroot has no GPU.
#
#   cargo build --workspace && ./scripts/test-bus.sh ./scripts/alerts-e2e.sh
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
until_() { local d=$((SECONDS+$1)); shift; until eval "$@"; do (( SECONDS < d )) || return 1; sleep 0.3; done; }
ug() { "$B/ug" --session "$@"; }
export XDG_DATA_HOME="$W/data" XDG_CONFIG_HOME="$W/cfg" XDG_STATE_HOME="$W/xstate" UG_TEST_RUN="alerts-$$" UG_ALERTS_SYSROOT="$R"

# A CPU at 95 °C and memory 97 % full.
H="$R/sys/class/hwmon/hwmon0"
mkdir -p "$H" "$R/proc" "$W/sysroot"
echo coretemp >"$H/name"; echo "Package id 0" >"$H/temp1_label"; echo 95000 >"$H/temp1_input"
printf 'MemTotal: 1000000 kB\nMemAvailable: 30000 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n' >"$R/proc/meminfo"
printf '/dev/fake / ext4 rw 0 0\n' >"$R/proc/mounts"

python3 - "$W/shown" <<'PY' & fake=$!
import sys
from gi.repository import Gio, GLib
OUT = sys.argv[1]
XML = """<node><interface name="org.freedesktop.Notifications">
<method name="Notify"><arg type="s" direction="in"/><arg type="u" direction="in"/><arg type="s" direction="in"/>
<arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="as" direction="in"/>
<arg type="a{sv}" direction="in"/><arg type="i" direction="in"/><arg type="u" direction="out"/></method>
</interface></node>"""
n = [0]
def call(conn, sender, path, iface, method, params, inv):
    n[0] += 1
    with open(OUT, "a") as f:
        f.write(params.unpack()[3] + "\n")
    inv.return_value(GLib.Variant("(u)", (n[0],)))
def acquired(conn, name):
    node = Gio.DBusNodeInfo.new_for_xml(XML)
    conn.register_object("/org/freedesktop/Notifications", node.interfaces[0], call, None, None)
Gio.bus_own_name(Gio.BusType.SESSION, "org.freedesktop.Notifications", 0, acquired, None, None)
GLib.MainLoop().run()
PY
"$B/ug-daemon" --session --sysroot "$W/sysroot" --state-dir "$W/state" --log warn 2>>"$W/daemon.log" & daemon=$!
HOME="$W/home" "$B/ug-agent" --daemon-session --no-tray --no-gamemode --log warn 2>>"$W/agent.log" & agent=$!
trap 'kill $agent $daemon $fake 2>/dev/null; rm -rf "$W"' EXIT
until_ 20 "ug alert >/dev/null 2>&1" || { echo "FAIL  agent did not start"; cat "$W/agent.log"; exit 1; }

ug alert >"$W/first" 2>&1
check "the defaults: CPU, GPU and disk on, memory off" "ug alert | grep -q 'CPU temperature *on *at *90' && ug alert | grep -q 'GPU temperature *on' && ug alert | grep -q 'Memory use *off' && ug alert | grep -q 'Disk fill *on *at *95'"
check "no GPU here: said so, not a dead control" "until_ 10 \"ug alert | grep -q 'not supported on this hardware'\""
check "a value out of range is refused" "! ug alert set sensitivity 2 >/dev/null 2>&1 && ! ug alert set cooldown 7 >/dev/null 2>&1"
check "the CPU alert fires after the sensitivity window" "ug alert set sensitivity 3 >/dev/null && until_ 15 \"grep -q 'The CPU is at 95 °C' '$W/shown' 2>/dev/null\""
check "memory stays quiet while its alert is off" "! grep -q Memory '$W/shown'"
check "turned on, memory alerts too" "ug alert set memory on >/dev/null && until_ 15 \"grep -q 'Memory is 97 % full' '$W/shown'\""
sleep 6
check "the cooldown keeps each alert to one" "[ \$(grep -c 'The CPU' '$W/shown') = 1 ] && [ \$(grep -c 'Memory' '$W/shown') = 1 ]"
check "both are in the recent list, on disk" "ug alert | grep -q 'Recent alerts' && grep -q cpu_temp '$XDG_DATA_HOME/unix-guardian/alerts-recent.json'"
check "a test alert is shown" "ug alert test | grep -q 'shown' && grep -q 'Test alert' '$W/shown'"
check "settings are kept in alerts.json" "grep -q '\"sustain_s\": 3' '$XDG_CONFIG_HOME/unix-guardian/alerts.json'"
mkdir -p "$W/xstate/ryoku"; echo '{"dnd": true}' >"$W/xstate/ryoku/flags.json"
check "the desktop's own do not disturb is reported, not hidden" "ug alert test | grep -q \"desktop's own Do not disturb\""
rm "$W/xstate/ryoku/flags.json"
check "clear forgets them" "ug alert clear | grep -q 'Forgot 4' && ug alert | grep -q 'No alerts; everything normal'"
check "JSON has the sensors" "ug --json alert | grep -q '\"above_s\"'"

[ $fails = 0 ] || cat "$W/first"
echo "$fails failed"
exit $(( fails > 0 ))
