#!/usr/bin/env bash
#
# Remake the README screenshots.
#
# The pictures in screenshots/ have to look the same as each other from one
# release to the next, so they are not taken on whatever desktop happens to be
# running. This script starts a private X server, runs the application on it
# with a fixed theme, font, accent colour and window size, and photographs each
# window. Nothing it does touches the desktop you are sitting at: the settings
# come from a read-only dconf database written here, and the application's own
# state goes to a scratch directory that is thrown away.
#
# Needs: Xvfb, xdotool and ImageMagick, and a built ./target/debug/negative-vibes.
# There is no window manager, so each window is drawn without a shadow and
# placed at the top left; frame() rounds the corners and paints the shadow
# afterwards, which is what GNOME's own window screenshot would have given.
#
# Usage: tools/screenshots.sh [outdir]            (default: screenshots/)
#
set -euo pipefail
set +m          # no "Terminated" notices when a run is stopped

SRC=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
OUT=${1:-$SRC/screenshots}
APP=$SRC/target/debug/negative-vibes
WORK=$(mktemp -d)
DISP=${NV_SHOT_DISPLAY:-:99}

# The window is 1280x800 at a scale factor of 2, so every picture is 2560x1600
# device pixels before the shadow margin is added.
SCALE=2
RADIUS=$((12 * SCALE))   # libadwaita's window corner radius
MARGIN=$((26 * SCALE))   # how far the shadow reaches

[ -x "$APP" ] || { echo "build it first: cargo build --bin negative-vibes" >&2; exit 1; }
for t in Xvfb xdotool magick import glib-compile-schemas dconf; do
  command -v "$t" >/dev/null || { echo "missing: $t" >&2; exit 1; }
done

cleanup() {
  pkill -f "$APP" 2>/dev/null || true
  [ -f "$WORK/xvfb.pid" ] && kill "$(cat "$WORK/xvfb.pid")" 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT

# ── The display ───────────────────────────────────────────────────────────────
Xvfb "$DISP" -screen 0 2800x1800x24 +extension RENDER >"$WORK/xvfb.log" 2>&1 &
echo $! >"$WORK/xvfb.pid"
export DISPLAY=$DISP
for _ in $(seq 1 50); do
  xdotool getdisplaygeometry >/dev/null 2>&1 && break
  command sleep 0.2
done

# ── The look ──────────────────────────────────────────────────────────────────
# GTK settings the desktop would otherwise supply over XSETTINGS.
mkdir -p "$WORK/cfg/gtk-4.0"
cat >"$WORK/cfg/gtk-4.0/settings.ini" <<'INI'
[Settings]
gtk-decoration-layout=:close
gtk-font-name=Adwaita Sans 11
gtk-application-prefer-dark-theme=false
gtk-enable-animations=false
INI

# A dconf database read straight from a file: no session bus is needed to read
# it and nothing can write to it, so the real one is never touched.
mkdir -p "$WORK/keys" "$WORK/cfg/dconf"
cat >"$WORK/keys/interface" <<'KEYS'
[org/gnome/desktop/interface]
color-scheme='default'
accent-color='purple'
font-name='Adwaita Sans 11'
KEYS
dconf compile "$WORK/cfg/dconf/user" "$WORK/keys"
echo "file-db:$WORK/cfg/dconf/user" >"$WORK/dconf-profile"

# The release schema, so the title is "Negative Vibes" and not the Devel id.
mkdir -p "$WORK/schemas"
cp "$SRC/data/io.github.cschalle.NegativeVibes.gschema.xml" "$WORK/schemas/"
glib-compile-schemas "$WORK/schemas"

# ── Running the application ───────────────────────────────────────────────────
# ADW_DEBUG_* pin the colour scheme and accent, which libadwaita otherwise takes
# from the settings portal; there is none here.
launch() {
  rm -rf "$WORK/state"
  mkdir -p "$WORK/state/data" "$WORK/state/cache" "$WORK/state/run"
  ( cd "$SRC" && env -u DBUS_SESSION_BUS_ADDRESS -u WAYLAND_DISPLAY \
      DISPLAY=$DISP GDK_BACKEND=x11 GDK_SCALE=$SCALE \
      ADW_DEBUG_COLOR_SCHEME=default ADW_DEBUG_ACCENT_COLOR=purple \
      DCONF_PROFILE="$WORK/dconf-profile" \
      XDG_CONFIG_HOME="$WORK/cfg" XDG_DATA_HOME="$WORK/state/data" \
      XDG_CACHE_HOME="$WORK/state/cache" XDG_RUNTIME_DIR="$WORK/state/run" \
      GSETTINGS_SCHEMA_DIR="$WORK/schemas" APP_ID=io.github.cschalle.NegativeVibes \
      PKGDATADIR="$SRC/data" "$@" "$APP" >"$WORK/app.log" 2>&1 & )
  for _ in $(seq 1 60); do
    main_window >/dev/null 2>&1 && break
    command sleep 0.5
  done
  command sleep 3
}

stop() { pkill -f "$APP" 2>/dev/null || true; command sleep 1; }

main_window() { xdotool search --name '^negative-vibes$' | tail -1; }
named_window() { xdotool search --name "^$1\$" | tail -1; }

# Keep the pointer off every widget: a hovered row lights up and the picture
# would show a highlight that nobody asked for.
park() { xdotool mousemove 2700 1750; command sleep 1; }

# frame IN OUT -- round the corners and paint the shadow the compositor would.
frame() {
  local in=$1 out=$2 w h
  w=$(magick identify -format "%w" "$in"); h=$(magick identify -format "%h" "$in")
  magick -size "${w}x${h}" xc:none -fill white \
    -draw "roundrectangle 0,0 $((w-1)),$((h-1)) $RADIUS,$RADIUS" "$WORK/mask.png"
  magick "$in" -alpha set "$WORK/mask.png" -compose DstIn -composite "$WORK/win.png"
  magick "$WORK/win.png" -background black -shadow 28x20+0+0 +repage "$WORK/s1.png"
  magick "$WORK/win.png" -background black -shadow 80x5+0+0  +repage "$WORK/s2.png"
  magick -size "$((w+2*MARGIN))x$((h+2*MARGIN))" xc:none \
    "$WORK/s1.png" -gravity center -composite \
    "$WORK/s2.png" -gravity center -composite \
    "$WORK/win.png" -gravity center -composite \
    -depth 8 -strip "$out"
  echo "  $out  $(magick identify -format '%wx%h' "$out")"
}

shot() { park; import -window "$1" "$WORK/raw.png"; frame "$WORK/raw.png" "$OUT/$2"; }

mkdir -p "$OUT"

# ── 1. First start ────────────────────────────────────────────────────────────
echo "main_screen_on_opening"
launch
shot "$(main_window)" main_screen_on_opening.png

# ── 2. Preferences, and the two output formats ────────────────────────────────
# The dialog is 560x640 logical. Its view switcher has six buttons across the
# bottom, so Output's centre is at 5/12 of the width; the Format row is the
# first in the list and its dropdown sits at the right-hand end.
echo "preferences"
W=$(main_window)
xdotool windowfocus "$W"; xdotool key --window "$W" ctrl+comma; command sleep 2
P=$(named_window Preferences)
# The first row has the keyboard focus when the dialog opens and draws a focus
# ring; a click on a group heading puts the window back in pointer mode.
xdotool mousemove $((70*SCALE)) $((82*SCALE)) click 1; command sleep 1
shot "$P" preferences.png

echo "preferences-outputformat1"
xdotool mousemove $((560*SCALE*5/12)) $((610*SCALE)) click 1; command sleep 2
shot "$P" preferences-outputformat1.png

echo "preferences-outputformat2"
xdotool mousemove $((472*SCALE)) $((159*SCALE)) click 1; command sleep 2  # Format dropdown
xdotool mousemove $((492*SCALE)) $((311*SCALE)) click 1; command sleep 2  # DNG (the raw scan)
shot "$P" preferences-outputformat2.png

# ── 3. Keyboard shortcuts ─────────────────────────────────────────────────────
echo "keyboardshortcuts"
xdotool key --window "$P" Escape 2>/dev/null || true; command sleep 2
xdotool windowfocus "$W"; xdotool key --window "$W" ctrl+question; command sleep 2
xdotool mousemove $((70*SCALE)) $((92*SCALE)) click 1; command sleep 1   # drop the focus ring
shot "$(named_window 'Keyboard Shortcuts')" keyboardshortcuts.png
stop

# ── 4. The page shown when no scanner has been chosen ─────────────────────────
# A SANE configuration directory with an empty dll.conf loads no backend, so
# the probe comes back with nothing, which is what a desktop with the scanner
# portal looks like before the user has allowed a scanner.
echo "choose-scanner"
mkdir -p "$WORK/sane"
: >"$WORK/sane/dll.conf"
launch env SANE_CONFIG_DIR="$WORK/sane"
shot "$(main_window)" choose-scanner.png
stop

# ── 5. A frame on the canvas ──────────────────────────────────────────────────
# Any 16-bit linear scan will do; NV_SHOT_RAW names it.
RAW=${NV_SHOT_RAW:-$HOME/nv-parity/2026-10-31/lz-1200.pnm}
if [ -r "$RAW" ]; then
  echo "previewofphoto"
  launch env NV_IMPORT_FILES="$RAW"
  command sleep 10
  shot "$(main_window)" previewofphoto.png
  stop
else
  echo "previewofphoto: skipped, no raw scan at $RAW" >&2
fi

echo "done"
