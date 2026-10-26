#!/bin/bash
# Run scenarios.py against the frontend written in C for xdg-desktop-portal, instead of against
# the prototype service. On a private session bus it starts the frontend with its scanner helper,
# the SANE `test` backend, and a stand-in for the dialog that grants without asking.
#
#   XDP_BUILD=/path/to/xdg-desktop-portal/build tests/run-scenarios-c-frontend.sh
#
# XDP_BUILD is a meson build directory of the scanner-frontend branch (build it with
# -Dsane=enabled).
set -e
: "${XDP_BUILD:?set XDP_BUILD to the xdg-desktop-portal build directory}"
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
export XDP_BUILD here root PYTHONUNBUFFERED=1
exec dbus-run-session -- bash -c '
  tmp=$(mktemp -d)
  mkdir "$tmp/portals" "$tmp/sane"
  echo test > "$tmp/sane/dll.conf"
  cat > "$tmp/portals/nvtest.portal" <<PORTAL
[portal]
DBusName=org.freedesktop.impl.portal.desktop.nvtest
Interfaces=org.freedesktop.impl.portal.Scanner
UseIn=nvtest
PORTAL
  export SANE_CONFIG_DIR=$tmp/sane XDG_CURRENT_DESKTOP=nvtest XDG_DESKTOP_PORTAL_DIR=$tmp/portals
  # The frontend starts the permission store and the document portal; keep them off the real data
  export XDG_DATA_HOME=$tmp/data XDG_STATE_HOME=$tmp/state XDG_CACHE_HOME=$tmp/cache
  # The sources moved from src/ to desktop-portal/ after 1.22, so a build of either has it
  for XDP_BIN in "$XDP_BUILD/desktop-portal" "$XDP_BUILD/src"; do [ -x "$XDP_BIN/xdg-desktop-portal" ] && break; done
  cat > "$tmp/helper" <<WRAP
#!/bin/sh
exec "$XDP_BIN/xdg-desktop-portal-scanner-helper" "\$@"
WRAP
  chmod +x "$tmp/helper"
  export XDP_SCANNER_HELPER=$tmp/helper
  export PORTAL_NAME=org.freedesktop.portal.Desktop
  python3 "$here/impl_auto_grant.py" org.freedesktop.impl.portal.desktop.nvtest \
      "$root/docs/scanner-portal/org.freedesktop.impl.portal.Scanner.xml" > "$tmp/backend.log" 2>&1 &
  backend=$!
  "$XDP_BIN/xdg-desktop-portal" -r > "$tmp/frontend.log" 2>&1 &
  frontend=$!
  trap "kill $backend $frontend 2>/dev/null; wait 2>/dev/null; rm -rf $tmp" EXIT
  for i in $(seq 100); do
    gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
        --method org.freedesktop.DBus.NameHasOwner org.freedesktop.portal.Desktop 2>/dev/null | grep -q true && break
    sleep 0.1
  done
  python3 "$here/scenarios.py" || { echo "--- frontend log"; tail -20 "$tmp/frontend.log"; exit 1; }
'
