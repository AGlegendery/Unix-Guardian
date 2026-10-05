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

check "counting is available and real (not simulated)" "out=\$(busctl --user call org.unixguardian.Daemon1 /org/unixguardian/Daemon1 org.unixguardian.Daemon1.Net Status) && grep -q '^(\?bsbas[a-z]*)\? true \"\" false' <<<\"\$out\""
check "our table exists" "\"$NFT\" list tables | grep -q 'inet unix_guardian'"
check "nothing is blocked without a rule" "! \"$NFT\" -j list table inet unix_guardian | grep -q '\"name\": \"block[0-9]\", [^}]*\"elem\"'"
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
  stale() { "$NFT" -j list table inet unix_guardian | python3 -c "import json,sys; print(sum(str(e['elem']['val'] if isinstance(e, dict) and isinstance(e.get('elem'), dict) else e).isdigit() for x in json.load(sys.stdin)['nftables'] for e in x.get('set',{}).get('elem',[])))"; }
  check "and no stale element is left in the sets" "until_ 10 '[ \"\$(stale)\" = 0 ]'"
else
  echo "SKIP  per-app counting (cannot create a child cgroup; run under systemd-run --user --scope -p Delegate=yes)"
fi

# Blocking and caps, against a peer in its own network namespace over veth.
app="$own/ug-e2e-app.service"   # the name `ug net` shows: ug-e2e-app
if mkdir "$app" 2>/dev/null; then
  cat >"$W/peer.py" <<'PY'
import socket, threading
def sink(port):
    s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind(("10.9.0.2", port)); s.listen()
    while True:
        c, _ = s.accept(); n = 0
        while (d := c.recv(1 << 16)): n += len(d)
        c.sendall(str(n).encode()); c.close()
def source(port):
    s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind(("10.9.0.2", port)); s.listen()
    while True:
        c, _ = s.accept(); c.sendall(b"y" * 2_000_000); c.close()
threading.Thread(target=sink, args=(7001,), daemon=True).start()
source(7002)
PY
  ip link add ugA type veth peer name ugB
  unshare -n bash -c 'while ! ip link show ugB >/dev/null 2>&1; do sleep 0.1; done
    ip link set lo up; ip addr add 10.9.0.2/24 dev ugB; ip link set ugB up; exec python3 '"$W/peer.py" &
  peer=$!
  sleep 0.3
  ip link set ugB netns "$peer"
  ip addr add 10.9.0.1/24 dev ugA; ip link set ugA up; ip route add default via 10.9.0.2
  trap 'kill $daemon $peer ${gen:-} 2>/dev/null; rmdir "$app" "$child" 2>/dev/null; rm -rf "$W"' EXIT
  sleep 0.5
  # Runs a Python snippet as the app (inside its cgroup); prints seconds taken.
  as_app() { sh -c 'echo $$ >"$1/cgroup.procs"; shift; exec python3 -c "$@"' _ "$app" "$1"; }
  upload='import socket,time; t=time.time(); c=socket.create_connection(("10.9.0.2",7001),timeout=20); c.sendall(b"x"*2_000_000); c.shutdown(1); c.recv(64); print("%.1f"%(time.time()-t))'
  download='import socket,time; t=time.time(); c=socket.create_connection(("10.9.0.2",7002),timeout=20); n=0
while (d:=c.recv(1<<16)): n+=len(d)
print("%.1f"%(time.time()-t))'
  fast="$(as_app "$upload")"
  check "without a rule 2 MB goes up quickly ($fast s)" "awk 'BEGIN{exit !($fast < 1.5)}'"
  # A long-lived process keeps the group alive, as a running app would.
  sh -c 'echo $$ >"$1/cgroup.procs"; exec sleep 300' _ "$app" & gen=$!
  sleep 6   # the daemon's 5-second scan names the new group
  check "the app is listed by name" "ug net --json --all | grep -q '\"name\": \"ug-e2e-app\"'"
  ug net limit ug-e2e-app --up 4m >/dev/null
  check "an upload cap installs our HTB tree on the default-route interface" "tc qdisc show dev ugA | grep -q 'htb 5547:'"
  slow="$(as_app "$upload")"
  check "a 4 Mbit/s upload cap makes 2 MB take about 4 s ($slow s)" "awk 'BEGIN{exit !($slow >= 3.2 && $slow < 8)}'"
  other="$(python3 -c "$upload")"
  check "other programs are not capped ($other s)" "awk 'BEGIN{exit !($other < 1.5)}'"
  fastdown="$(as_app "$download")"
  ug net limit ug-e2e-app --down 4m >/dev/null
  check "a download cap moves to the IFB device and frees the upload" "ip link show ug-ifb0 >/dev/null 2>&1 && ! tc qdisc show dev ugA | grep -q 'htb 5547:'"
  slowdown="$(as_app "$download")"
  check "a 4 Mbit/s download cap makes 2 MB take about 4 s ($fastdown s before, $slowdown s after)" "awk 'BEGIN{exit !($slowdown >= 3.2 && $slowdown < 8 && $fastdown < 1.5)}'"
  ug net block ug-e2e-app >/dev/null
  check "a blocked app cannot connect" "! as_app \"\$upload\" >/dev/null 2>&1"
  check "the app shows as blocked" "ug net --json --all | grep -A12 '\"name\": \"ug-e2e-app\"' | grep -q '\"state\": \"blocked\"'"
  check "while other programs still can" "python3 -c \"\$upload\" >/dev/null"
  check "rules are listed" "ug net rules | grep -q 'ug-e2e-app .*blocked'"
  ug net allow ug-e2e-app >/dev/null
  check "allowing it again restores its connection" "as_app \"\$upload\" >/dev/null"
  check "and removes every trace of shaping" "! ip link show ug-ifb0 >/dev/null 2>&1 && ! tc qdisc show dev ugA | grep -q 5547 && ! tc qdisc show dev ugA | grep -q ingress"
  # Slow down background apps: every app without its own rule shares one cap.
  ug net background on --up 4m --down 8m >/dev/null
  bg_state() { ug net --json --all | grep -A12 '"name": "ug-e2e-app"' | grep -q '"state": "background"'; }
  check "background slowing marks the app as background" "until_ 10 bg_state"
  bgslow="$(as_app "$upload")"
  check "a 4 Mbit/s background cap makes 2 MB take about 4 s ($bgslow s)" "awk 'BEGIN{exit !($bgslow >= 3.2 && $bgslow < 8)}'"
  ug net background off >/dev/null
  bgfast="$(as_app "$upload")"
  check "turning it off frees it again ($bgfast s)" "awk 'BEGIN{exit !($bgfast < 1.5)}'"
  ug net limit ug-e2e-app --up 1m --down 1m >/dev/null
  kill -TERM "$daemon"; wait "$daemon" 2>/dev/null
  check "a clean stop removes caps too" "! ip link show ug-ifb0 >/dev/null 2>&1 && ! tc qdisc show dev ugA | grep -q 5547"
  check "rules survive a restart (saved)" "grep -q ug-e2e-app '$W/state/net.json'"
  "$B/ug-daemon" --session --state-dir "$W/state" --log warn 2>>"$W/daemon.log" & daemon=$!
  until_ 20 "ug net --json >/dev/null 2>&1"
  check "and come back into force" "until_ 10 'tc qdisc show dev ugA | grep -q \"htb 5547:\"'"
  ug net allow ug-e2e-app >/dev/null

  # A VPN in proxy mode: apps reach the internet through a local proxy.
  vpn="$own/ug-xray.service"   # named like xray: recognised as a VPN
  mkdir "$vpn"
  cat >"$W/proxy.py" <<'PY'
import socket, sys, threading
open(sys.argv[1], "w").write(str(__import__("os").getpid()))
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind(("127.0.0.1", 1080)); s.listen()
def pipe(a, b):
    while (d := a.recv(1 << 16)): b.sendall(d)
    b.shutdown(socket.SHUT_WR)
while True:
    c, _ = s.accept(); u = socket.create_connection(("10.9.0.2", 7001))
    threading.Thread(target=pipe, args=(c, u), daemon=True).start()
    threading.Thread(target=pipe, args=(u, c), daemon=True).start()
PY
  python3 "$W/proxy.py" "$vpn/cgroup.procs" & proxy=$!
  sleep 6   # named by the next scan
  counted() { ug net --json --all | python3 -c "import json,sys; print(next((a['$2'] for a in json.load(sys.stdin) if a['name']=='$1'), -1))"; }
  app_before="$(counted ug-e2e-app tx_bytes)"; vpn_before="$(counted ug-xray tx_bytes)"
  as_app 'import socket; c=socket.create_connection(("127.0.0.1",1080)); c.sendall(b"z"*2_000_000); c.shutdown(1); c.recv(64)' >/dev/null
  sleep 3
  app_sent=$(( $(counted ug-e2e-app tx_bytes) - app_before )); vpn_sent=$(( $(counted ug-xray tx_bytes) - vpn_before ))
  check "an app using a local proxy is credited with its 2 MB ($app_sent)" "[ $app_sent -ge 2000000 ] && [ $app_sent -lt 2600000 ]"
  check "the proxy counts only its own outside traffic, not the loopback copy ($vpn_sent)" "[ $vpn_sent -ge 2000000 ] && [ $vpn_sent -lt 2600000 ]"
  check "the proxy is recognised as a VPN" "ug net --json --all | grep -A14 '\"name\": \"ug-xray\"' | grep -q '\"category\": \"vpn\"'"
  ug net limit ug-e2e-app --up 1m >/dev/null
  check "a cap in proxy mode says plainly that it cannot apply" "until_ 10 'ug net | grep -q \"running as a proxy\"'"
  ug net allow ug-e2e-app >/dev/null
  kill "$proxy"; wait "$proxy" 2>/dev/null
  until_ 10 "! grep -q . '$vpn/cgroup.procs'"; rmdir "$vpn"

  # A VPN in TUN mode: the internet route goes through a TUN device.
  ip tuntap add dev ugtun0 mode tun && ip link set ugtun0 up && ip addr add 10.8.0.1/24 dev ugtun0
  ip route add 1.1.1.1/32 dev ugtun0
  ug net limit ug-e2e-app --up 1m >/dev/null
  check "caps follow the internet route onto the TUN device" "until_ 10 'tc qdisc show dev ugtun0 | grep -q \"htb 5547:\"'"
  check "and leave the other interface alone" "! tc qdisc show dev ugA | grep -q 5547"
  check "the status says the internet goes through TUN" "busctl --user call org.unixguardian.Daemon1 /org/unixguardian/Daemon1 org.unixguardian.Daemon1.Net Status | grep -q '\"ugtun0\" \"[^\"]*\" true'"
  ug net allow ug-e2e-app >/dev/null
  check "and removing the cap clears the TUN device" "until_ 10 '! tc qdisc show dev ugtun0 | grep -q 5547'"
  ip link del ugtun0
else
  echo "SKIP  blocking and caps (cannot create a child cgroup; run under systemd-run --user --scope -p Delegate=yes)"
fi

"$NFT" flush ruleset
check "a flushed ruleset gets our table back" "until_ 15 'ug net --json >/dev/null && \"$NFT\" list tables | grep -q unix_guardian' || { ug net; \"$NFT\" list tables; false; }"
check "while leaving others' removals alone" "! \"$NFT\" list tables | grep -q theirs"

kill -TERM "$daemon"; wait "$daemon" 2>/dev/null
check "a clean stop removes our table" "! \"$NFT\" list tables | grep -q unix_guardian"

if [ "$fails" -gt 0 ]; then echo "--- daemon log"; cat "$W/daemon.log"; fi
echo "$fails failure(s)"
exit $((fails > 0))
