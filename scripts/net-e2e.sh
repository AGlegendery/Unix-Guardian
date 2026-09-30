#!/usr/bin/env bash
# End-to-end check of per-app traffic counting with real nftables, inside a
# private user + network namespace where the host's firewall is out of reach:
#
#   cargo build --workspace
#   systemd-run --user --scope -q -p Delegate=yes \
#     unshare -rn ./scripts/test-bus.sh ./scripts/net-e2e.sh
#
# `Delegate=yes` lets the test make a child cgroup for its traffic generator;
# without it the per-app checks are skipped. The daemon runs with --session,
# and uses the real nftables code only because it sees the private namespace.
set -uo pipefail
if [ "$(cat /proc/self/uid_map | awk '{print $1,$2,$3}')" = "0 0 4294967295" ]; then
  echo "FAIL  not in a private user namespace; run under 'unshare -rn' (see the top of this script)" >&2
  exit 1
fi
if busctl --user list --activatable 2>/dev/null | grep -q org.unixguardian; then
  echo "FAIL  the test bus can activate an installed Unix Guardian service; run with scripts/test-bus.sh" >&2
  exit 1
fi
NFT="$(command -v nft || echo /usr/sbin/nft)"
B="${1:-$PWD/target/debug}"
W="$(mktemp -d)"
fails=0
check() { if eval "$2"; then echo "PASS  $1"; else echo "FAIL  $1"; fails=$((fails+1)); fi; }
until_() { local d=$((SECONDS+$1)); shift; until eval "$@"; do (( SECONDS < d )) || return 1; sleep 0.3; done; }
ug() { "$B/ug" --session "$@"; }
ip link set lo up

# Someone else's rules, which must come through untouched.
"$NFT" -f - <<'EOF'
table inet theirs {
  chain input { type filter hook input priority 0; policy accept; tcp dport 22 counter accept; }
}
EOF
before="$("$NFT" list table inet theirs)"

"$B/ug-daemon" --session --state-dir "$W/state" --log warn 2>>"$W/daemon.log" & daemon=$!
trap 'kill $daemon ${gen:-} 2>/dev/null; rmdir "$child" 2>/dev/null; rm -rf "$W"' EXIT
until_ 20 "ug net --json >/dev/null 2>&1" || { echo "FAIL  daemon did not start"; cat "$W/daemon.log"; exit 1; }

check "counting is available and real (not simulated)" "out=\$(busctl --user call org.unixguardian.Daemon1 /org/unixguardian/Daemon1 org.unixguardian.Daemon1.Net Status) && grep -q '^(\?bsbas)\? true \"\" false' <<<\"\$out\""
check "our table exists" "\"$NFT\" list tables | grep -q 'inet unix_guardian'"
check "the table never drops anything" "! \"$NFT\" list table inet unix_guardian | grep -q drop"
check "other tables are untouched" "[ \"\$(\"$NFT\" list table inet theirs)\" = \"\$before\" ]"

own="/sys/fs/cgroup$(cut -d: -f3 /proc/self/cgroup)"
child="$own/ug-net-e2e"
if mkdir "$child" 2>/dev/null; then
  rel="${child#/sys/fs/cgroup/}"
  # A server in the child group sends 5 MB to a client outside it.
  python3 - "$child/cgroup.procs" <<'PY' &
import os, socket, sys, time
open(sys.argv[1], "w").write(str(os.getpid()))
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", 9977)); s.listen()
time.sleep(6)  # the daemon's 5-second scan must see the group first
c, _ = s.accept(); c.sendall(b"x" * 5_000_000); c.close()
time.sleep(30)
PY
  gen=$!
  sleep 7
  python3 -c "
import socket; c = socket.create_connection(('127.0.0.1', 9977)); n = 0
while (d := c.recv(1 << 16)): n += len(d)
print(n)" >"$W/got"
  check "the generator delivered 5 MB" "[ \"\$(cat $W/got)\" = 5000000 ]"
  sent() { ug net --json --all | python3 -c "import json,sys; print(next((a['tx_bytes'] for a in json.load(sys.stdin) if a['id']=='$rel'), -1))"; }
  check "its group is listed with at least 5 MB sent" "until_ 10 '[ \"\$(sent)\" -ge 5000000 ]'"
  kill "$gen"; wait "$gen" 2>/dev/null; gen=
  rmdir "$child"
  check "a finished group leaves the list" "until_ 15 '[ \"\$(sent)\" = -1 ]'"
  stale() { "$NFT" -j list table inet unix_guardian | python3 -c "import json,sys; print(sum(str(e['elem']['val']).isdigit() for x in json.load(sys.stdin)['nftables'] for e in x.get('set',{}).get('elem',[])))"; }
  check "and no stale element is left in the sets" "until_ 10 '[ \"\$(stale)\" = 0 ]'"
else
  echo "SKIP  per-app counting (cannot create a child cgroup; run under systemd-run --user --scope -p Delegate=yes)"
fi

"$NFT" flush ruleset
check "a flushed ruleset gets our table back" "until_ 15 'ug net --json >/dev/null && \"$NFT\" list tables | grep -q unix_guardian'"
check "while leaving others' removals alone" "! \"$NFT\" list tables | grep -q theirs"

kill -TERM "$daemon"; wait "$daemon" 2>/dev/null
check "a clean stop removes our table" "! \"$NFT\" list tables | grep -q unix_guardian"

if [ "$fails" -gt 0 ]; then echo "--- daemon log"; cat "$W/daemon.log"; fi
echo "$fails failure(s)"
exit $((fails > 0))
