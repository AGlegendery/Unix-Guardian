#!/usr/bin/env bash
# End-to-end check of the recorder against a stand-in gpu-screen-recorder
# (a script that writes files where the real one would): settings, a
# recording started and stopped, the replay buffer saved, and the folder.
#
#   cargo build --workspace && ./scripts/test-bus.sh ./scripts/recorder-e2e.sh
#
# Private bus and scratch XDG directories only.
set -uo pipefail
if busctl --user list --activatable 2>/dev/null | grep -q org.unixguardian; then
  echo "FAIL  the test bus can activate an installed Unix Guardian service; run with scripts/test-bus.sh" >&2
  exit 1
fi
B="${1:-$PWD/target/debug}"
W="$(mktemp -d)"
export XDG_DATA_HOME="$W/data" XDG_CONFIG_HOME="$W/cfg" HOME="$W/home" XDG_SESSION_TYPE=wayland
mkdir -p "$XDG_CONFIG_HOME/unix-guardian" "$HOME/.local/share/Steam/steamapps" "$W/bin"
echo '"libraryfolders" {}' > "$HOME/.local/share/Steam/steamapps/libraryfolders.vdf"
export UG_TEST_RUN="e2e-$$-$RANDOM"
cat > "$W/bin/gpu-screen-recorder" <<'GSR'
#!/usr/bin/env bash
if [ "${1:-}" = "--list-capture-options" ]; then printf 'window\nportal\nDP-1|2560x1440\n'; exit 0; fi
echo "$*" >> "$GSR_LOG"
out=""; replay=""
while [ $# -gt 0 ]; do case "$1" in -o) out="$2"; shift;; -r) replay="$2"; shift;; esac; shift; done
if [ -n "$replay" ]; then
  trap 'f="$out/Replay $(date +%s%N).mp4"; echo clip > "$f"; echo "$f"' USR1
else
  trap 'echo video > "$out"; exit 0' INT
fi
trap 'exit 0' TERM
[ -n "$replay" ] && trap 'exit 0' INT
while true; do sleep 0.1; done
GSR
chmod +x "$W/bin/gpu-screen-recorder"
export PATH="$W/bin:$PATH" GSR_LOG="$W/gsr.log"
fails=0
check() { if eval "$2"; then echo "PASS  $1"; else echo "FAIL  $1"; fails=$((fails+1)); fi; }
until_() { local d=$((SECONDS+$1)); shift; until eval "$@"; do (( SECONDS < d )) || return 1; sleep 0.2; done; }

"$B/ug-agent" --quiet --log warn & agent=$!
trap 'kill $agent 2>/dev/null; rm -rf "$W"' EXIT
until_ 15 "'$B/ug' record >/dev/null 2>&1" || { echo "FAIL  agent did not start"; exit 1; }

check "the engine and the screens it offers are found" "'$B/ug' record | grep -q 'gpu-screen-recorder' && '$B/ug' record | grep -q 'offered: portal, DP-1'"
check "the size of a minute is estimated" "'$B/ug' record | grep -q '≈ 91 MB a minute'"
check "settings change one at a time and are kept" "'$B/ug' record set --capture DP-1 --mic on --replay 30 >/dev/null && grep -q '\"replay_seconds\": 30' '$XDG_CONFIG_HOME/unix-guardian/recorder.json' && '$B/ug' record | grep -q 'microphone on'"
check "a frame rate it does not offer is refused" "! '$B/ug' record set --fps 50 2>/dev/null"
out="$("$B/ug" record start)"
f="$(sed -n 's/^Recording to //p' <<<"$out")"
check "a recording starts with the chosen settings" "until_ 5 \"grep -q -- '-w DP-1 -f 60 -q high -a default_output -a default_input -c mp4 -o' '$W/gsr.log'\" && '$B/ug' record | grep -q 'recording to'"
check "a second start is refused while one runs" "! '$B/ug' record start 2>/dev/null"
check "stopping finishes the file" "'$B/ug' record stop | grep -q 'Saved' && [ \"\$(cat \"\$f\")\" = video ]"
check "it goes to ~/Videos/Unix Guardian" "[[ \"\$f\" == '$HOME/Videos/Unix Guardian/Recording '*.mp4 ]]"
check "the replay buffer runs" "'$B/ug' record replay | grep -q 'keeping the last 30 s'"
check "saving it writes a clip and reports it" "'$B/ug' record save | grep -q 'Saved .*/Replay '"
check "and it can be saved again" "'$B/ug' record save | grep -q 'Saved .*/Replay '"
check "stopping the buffer" "'$B/ug' record stop | grep -q 'Replay buffer stopped' && '$B/ug' record | grep -q 'Now           idle'"
check "the recording shortcut starts and stops a recording" "'$B/ug' shortcut toggle-recording | grep -q 'Recording to' && '$B/ug' shortcut toggle-recording | grep -q 'Recording saved'"
check "the replay shortcut starts the buffer, then saves it" "'$B/ug' shortcut save-replay | grep -q 'Keeping the last seconds' && '$B/ug' shortcut save-replay | grep -q 'Replay saved' && '$B/ug' record stop >/dev/null"
check "save without a buffer is refused" "! '$B/ug' record save 2>/dev/null"

echo; [ "$fails" -eq 0 ] && echo "recorder-e2e: OK" || { echo "recorder-e2e: $fails failed"; exit 1; }
