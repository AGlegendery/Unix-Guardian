#!/usr/bin/env bash
# End-to-end smoke test: daemon and CLI talking over a real D-Bus bus.
#
# Run it inside a private bus so it never touches the desktop session:
#   dbus-run-session -- ./scripts/smoke.sh [target/debug]
#
# Passes on a machine with no GPU and no sensors (a CI container): the point
# is the protocol and lifecycle, not the readings.
set -euo pipefail

# Nothing here may be D-Bus-activated from an installed copy: with Unix Guardian
# installed, a test bus that uses the standard service directories starts
# /usr/local/bin/ug-agent (with the user's real XDG directories) the moment the
# test's own agent is not answering, and the test then writes into the real
# playtime database. Found the hard way; see 05-findings.md.
if busctl --user list --activatable 2>/dev/null | grep -q org.unixguardian; then
  echo "FAIL  the test bus can activate an installed Unix Guardian service; run with scripts/test-bus.sh" >&2
  exit 1
fi

bin="${1:-target/debug}"
ug() { "$bin/ug" --session "$@"; }

"$bin/ug-daemon" --session --log warn &
daemon=$!
trap 'kill $daemon 2>/dev/null || true' EXIT

# Start-up includes GPU discovery, which on a laptop wakes a sleeping dGPU once
# (~1.5 s). Poll against a deadline, not an attempt count: a failed attempt
# returns instantly, so a count runs out long before the daemon is up.
deadline=$((SECONDS + 20))
until ug gpu active >/dev/null 2>&1; do
  if (( SECONDS >= deadline )); then echo "FAIL: daemon never claimed its bus name" >&2; exit 1; fi
  kill -0 "$daemon" 2>/dev/null || { echo "FAIL: daemon exited during start-up" >&2; exit 1; }
  sleep 0.2
done

echo "--- status";  ug status
echo "--- gpu list"; ug gpu list
echo "--- json is valid"
ug status --json | python3 -c 'import json,sys; d=json.load(sys.stdin); assert "snapshot" in d and "inventory" in d'

echo "--- streaming: 2 samples, then a clean exit through a closed pipe"
lines=$(ug monitor --interval 250 --json | head -2 | wc -l)
[ "$lines" -eq 2 ] || { echo "FAIL: expected 2 samples, got $lines" >&2; exit 1; }

echo "--- the sampling loop parks once the client is gone"
emitted=$(timeout 1.5 busctl --user monitor --json=short org.unixguardian.Daemon1 2>/dev/null | grep -c '"member":"Sample"' || true)
[ "$emitted" -eq 0 ] || { echo "FAIL: daemon still sampling for nobody ($emitted signals)" >&2; exit 1; }

echo "--- protocol version"
busctl --user get-property org.unixguardian.Daemon1 /org/unixguardian/Daemon1 org.unixguardian.Daemon1.System ProtocolVersion

echo; echo "smoke: OK"
