#!/usr/bin/env bash
# Runs a command on a private session bus that can D-Bus-activate nothing.
#
#   ./scripts/test-bus.sh ./scripts/agent-e2e.sh
#   ./scripts/test-bus.sh ./scripts/gamemode-e2e.sh target/debug
#
# Plain `dbus-run-session` uses the standard service directories, so once Unix
# Guardian is installed a test bus would start the installed agent — with the
# user's real XDG directories — whenever the test's own agent is not answering.
# Pointing XDG_DATA_DIRS/XDG_DATA_HOME at an empty directory leaves the bus
# with nothing to activate. The scripts set their own scratch XDG_* inside.
set -euo pipefail
empty="$(mktemp -d)"
trap 'rm -rf "$empty"' EXIT
XDG_DATA_DIRS="$empty" XDG_DATA_HOME="$empty" dbus-run-session -- "$@"
