#!/usr/bin/env bash
# End-to-end check of the global shortcuts against a fake KDE desktop
# (scripts/fake-kde-shortcuts.py: the GlobalShortcuts portal and kglobalaccel):
# binding, keys the user picks (`ug shortcut keys`), keys another program
# holds, a press, and the choice kept across agent restarts.
#
#   cargo build --workspace && ./scripts/test-bus.sh ./scripts/shortcuts-e2e.sh
#
# Private bus and scratch XDG directories only; never the real desktop.
set -uo pipefail
if busctl --user list --activatable 2>/dev/null | grep -q org.unixguardian; then
  echo "FAIL  the test bus can activate an installed Unix Guardian service; run with scripts/test-bus.sh" >&2
  exit 1
fi
if busctl --user status org.kde.kglobalaccel >/dev/null 2>&1; then
  echo "FAIL  this bus has the real kglobalaccel; run with scripts/test-bus.sh" >&2
  exit 1
fi
B="${1:-$PWD/target/debug}"
W="$(mktemp -d)"
export XDG_DATA_HOME="$W/data" XDG_CONFIG_HOME="$W/cfg"
mkdir -p "$XDG_CONFIG_HOME/unix-guardian" "$W/home/.local/share/Steam/steamapps"
echo '"libraryfolders" {}' > "$W/home/.local/share/Steam/steamapps/libraryfolders.vdf"
export UG_TEST_RUN="e2e-$$-$RANDOM"
J="$XDG_CONFIG_HOME/unix-guardian/shortcuts.json"
fails=0
check() { if eval "$2"; then echo "PASS  $1"; else echo "FAIL  $1"; fails=$((fails+1)); fi; }
until_() { local d=$((SECONDS+$1)); shift; until eval "$@"; do (( SECONDS < d )) || return 1; sleep 0.2; done; }
keys_of() { "$B/ug" shortcut | awk -v id="$1" '$1 == id {print $2}'; }
start_agent() {
  HOME="$W/home" "$B/ug-agent" --quiet --log warn & agent=$!
  until_ 15 "'$B/ug' shortcut >/dev/null 2>&1" || { echo "FAIL  agent did not start"; exit 1; }
}

python3 "$(dirname "$0")/fake-kde-shortcuts.py" & fake=$!
trap 'kill ${agent:-} $fake 2>/dev/null; rm -rf "$W"' EXIT
until_ 10 "busctl --user status org.kde.kglobalaccel >/dev/null 2>&1" || { echo "FAIL  fake desktop did not start"; exit 1; }
start_agent

check "off, the suggested keys are shown" "[ \"\$(keys_of toggle-gamemode)\" = '(Meta+Shift+G)' ]"
check "keys picked while off are saved for later" "'$B/ug' shortcut keys toggle-gamemode Meta+Alt+G | grep -q 'used when the shortcuts are turned on' && grep -q 'ALT+LOGO+g' '$J'"
check "and shown as the user's" "'$B/ug' shortcut | grep -q '(Meta+Alt+G, yours)'"
check "a key without Ctrl, Alt or Meta is refused" "! '$B/ug' shortcut keys toggle-gamemode g 2>/dev/null"
check "the same keys for two shortcuts are refused" "out=\$('$B/ug' shortcut keys open-window Alt+Meta+G 2>&1); [ \$? -ne 0 ] && grep -q 'already the shortcut' <<<\"\$out\""

check "turned on, the desktop binds the picked keys" "'$B/ug' shortcut on | grep -q 'Shortcuts   on' && [ \"\$(keys_of toggle-gamemode)\" = 'Meta+Alt+G' ]"
check "and the suggested ones for the rest" "[ \"\$(keys_of toggle-overlay)\" = 'Meta+Shift+O' ]"
check "keys picked while on are set in KDE at once" "'$B/ug' shortcut keys toggle-overlay Meta+Shift+P | grep -q 'now does' && [ \"\$(keys_of toggle-overlay)\" = 'Meta+Shift+P' ]"
check "keys another program holds are refused, naming it" "out=\$('$B/ug' shortcut keys toggle-overlay Meta+E 2>&1); [ \$? -ne 0 ] && grep -q 'KWin' <<<\"\$out\" && [ \"\$(keys_of toggle-overlay)\" = 'Meta+Shift+P' ]"
check "a press does what the shortcut does" "busctl --user call org.freedesktop.portal.Desktop /org/freedesktop/portal/desktop org.unixguardian.Test Press s toggle-gamemode && until_ 5 \"'$B/ug' shortcut | grep -q 'turned on by hand'\""
"$B/ug" shortcut toggle-gamemode >/dev/null

"$B/ug" shortcut off >/dev/null
"$B/ug" shortcut keys toggle-overlay Ctrl+Meta+O >/dev/null
check "keys picked while off win over what the desktop kept" "'$B/ug' shortcut on >/dev/null && until_ 5 \"[ \\\"\\\$(keys_of toggle-overlay)\\\" = 'Meta+Ctrl+O' ]\""
check "no keys goes back to the suggested ones" "'$B/ug' shortcut keys toggle-overlay >/dev/null && [ \"\$(keys_of toggle-overlay)\" = 'Meta+Shift+O' ] && ! grep -q toggle-overlay '$J'"

kill $agent; wait $agent 2>/dev/null
start_agent
check "after a restart the shortcuts are bound again with the picked keys" "until_ 5 \"[ \\\"\\\$(keys_of toggle-gamemode)\\\" = 'Meta+Alt+G' ]\""
check "an unknown shortcut is refused" "! '$B/ug' shortcut keys fly Meta+F 2>/dev/null"

# XFCE: no portal; "Set up" writes the keys into xfconf (a fake
# xfconf-query over a file), leaves a key XFCE already uses, and undo takes
# out only what it wrote.
kill $agent $fake 2>/dev/null; wait $agent $fake 2>/dev/null
mkdir -p "$W/bin"
X="$W/xfconf"
printf '/commands/custom/<Shift><Super>o\txfce4-screenshooter\n' > "$X"
cat > "$W/bin/xfconf-query" <<FAKE
#!/usr/bin/env bash
# -c channel -p prop [-r | -s value | -n -t string -s value]
f="$X"; p="\$4"; shift 4
get() { awk -F'\t' -v p="\$p" '\$1 == p {print \$2; f=1} END {exit !f}' "\$f"; }
del() { awk -F'\t' -v p="\$p" '\$1 != p' "\$f" > "\$f.new"; mv "\$f.new" "\$f"; }
case "\${1:-}" in
  "") get || { echo "Property \$p does not exist" >&2; exit 1; } ;;
  -r) del ;;
  -s) del; printf '%s\t%s\n' "\$p" "\$2" >> "\$f" ;;
  -n) del; printf '%s\t%s\n' "\$p" "\$5" >> "\$f" ;;
esac
FAKE
chmod +x "$W/bin/xfconf-query"
PATH="$W/bin:$PATH" XDG_CURRENT_DESKTOP=XFCE start_agent
check "XFCE without the portal offers the Set up" "until_ 5 \"'$B/ug' shortcut | grep -q 'ug shortcut setup writes them into XFCE'\""
out="$("$B/ug" shortcut setup 2>&1)"
check "Set up writes the keys into XFCE's settings" "grep -q '4 shortcuts written into XFCE' <<<\"\$out\" && grep -qP '^/commands/custom/<Alt><Super>g\t.*ug shortcut toggle-gamemode$' '$X'"
check "and leaves a key XFCE already uses, saying so" "grep -q 'already runs xfce4-screenshooter' <<<\"\$out\" && grep -qP '^/commands/custom/<Shift><Super>o\txfce4-screenshooter$' '$X'"
check "new keys are written there at once" "'$B/ug' shortcut keys open-window Ctrl+Alt+U | grep -q 'now does' && grep -qP '^/commands/custom/<Primary><Alt>u\t.*open-window$' '$X' && ! grep -qF '<Shift><Super>u' '$X'"
check "undo removes only what was written" "'$B/ug' shortcut setup undo | grep -q 'removed from XFCE' && ! grep -q 'ug shortcut' '$X' && grep -q xfce4-screenshooter '$X'"

echo; [ "$fails" -eq 0 ] && echo "shortcuts-e2e: OK" || { echo "shortcuts-e2e: $fails failed"; exit 1; }
