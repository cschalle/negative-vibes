#!/bin/bash
# Start a private session bus with the prototype service on the SANE test backend,
# and run scenarios.py against it. Build first: cargo build -p nv-portal-proto
set -e
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
exec dbus-run-session -- bash -c '
  cfg=$(mktemp -d); echo test > "$cfg/dll.conf"
  export SANE_CONFIG_DIR=$cfg
  "'"$root"'/target/debug/nv-portal-proto" --select test: 2>"$cfg/service.log" &
  svc=$!
  trap "kill $svc 2>/dev/null; rm -rf $cfg" EXIT
  sleep 1
  python3 "'"$here"'/scenarios.py"
'
