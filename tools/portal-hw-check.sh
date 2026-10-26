#!/bin/bash
# portal-hw-check.sh: the scanner portal prototype against libsane, on the OpticFilm 8200i.
#
# Scans the same frame the direct way (libsane in the app) and through the portal prototype,
# and checks that the results agree, using the thresholds of DESIGN.md §11.4 (H1, H2, H3, H4,
# H5). Read docs/scanner-portal/HARDWARE-TEST.md before running it on the scanner.
#
#   tools/portal-hw-check.sh [--dpi 600] [--out DIR] [--yes] [--skip ir,cancel] [--streamcheck] [--dev-backend]
#   tools/portal-hw-check.sh --dry-run        # SANE test backend: checks the plumbing, no hardware
#
# By default the portal is the prototype service. With --frontend c it is the frontend written in
# C for xdg-desktop-portal, with its scanner helper and a stand-in for the dialog that grants
# without asking; XDP_BUILD must name a meson build directory of the scanner-frontend branch:
#   XDP_BUILD=/path/to/xdg-desktop-portal/build tools/portal-hw-check.sh --frontend c [--dry-run]
#
# It never detaches the scanner from the VM or power-cycles it; it tells you how.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
CHECK="python3 $HERE/scan-check.py"

ORIG_ARGS=("$@")   # the parsing below shifts them away; the re-exec into the private bus needs them
DRY=0; DPI=600; OUT=""; YES=0; SKIP=""; STREAMCHECK=0; DEVBACKEND=0; FRONTEND=proto
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) DRY=1 ;;
    --dpi) DPI=$2; shift ;;
    --out) OUT=$2; shift ;;
    --yes) YES=1 ;;
    --skip) SKIP=$2; shift ;;
    --streamcheck) STREAMCHECK=1 ;;
    --dev-backend) DEVBACKEND=1 ;;
    --frontend) FRONTEND=$2; shift ;;
    -h|--help) sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

# Everything portal-related runs on a private session bus, so the prototype never touches
# the real one and its bus name is certainly free.
if [ -z "${NV_HW_IN_BUS:-}" ]; then
  exec env NV_HW_IN_BUS=1 dbus-run-session -- "$0" "${ORIG_ARGS[@]}"
fi

case "$FRONTEND" in
  proto) PORTAL_BUS=org.freedesktop.NvPortalProto ;;
  c)
    : "${XDP_BUILD:?--frontend c needs XDP_BUILD, the xdg-desktop-portal build directory}"
    [ -x "$XDP_BUILD/desktop-portal/xdg-desktop-portal" ] || { echo "no xdg-desktop-portal in $XDP_BUILD/desktop-portal" >&2; exit 2; }
    PORTAL_BUS=org.freedesktop.portal.Desktop ;;
  *) echo "--frontend is proto or c" >&2; exit 2 ;;
esac
# What the app and the checks set to use the portal and not libsane
PENV="NV_SCANNER_BACKEND=portal NV_PORTAL_BUS_NAME=$PORTAL_BUS"

[ -n "$OUT" ] || OUT=$(mktemp -d "${TMPDIR:-/tmp}/portal-hw.XXXXXX")
mkdir -p "$OUT"
# Every step that talks to the scanner runs under a time limit: a hang must end, not scan for
# half an hour. Scans at 600 dpi take about 20 s; override with NV_HW_TIMEOUT (seconds).
TMO=${NV_HW_TIMEOUT:-300}
limited() { timeout --signal=TERM --kill-after=10 "$TMO" "$@"; }
skipped() { case ",$SKIP," in *",$1,"*) return 0 ;; esac; return 1; }

# ---- environment -----------------------------------------------------------------------
if [ $DRY -eq 1 ]; then
  CFG=$(mktemp -d); echo test > "$CFG/dll.conf"
  export SANE_CONFIG_DIR=$CFG NV_SANE_ALLOW_VIRTUAL=1
  MATCH="test:"; BUILD=debug; DPI=75; DRIVER="SANE test backend"
else
  # A private SANE configuration that lists only the 8200i's own backend. The system configuration
  # also enables the stock genesys backend, which lists this scanner too ("8200i SE"), and
  # opening that entry reset the scanner (kernel: "device firmware changed", new USB address),
  # so nothing here may open anything but plustek8200i. The driver finds its own library either
  # way; the packaged plustek8200i.conf is only comments, but it is copied if it exists.
  CFG=$(mktemp -d)
  echo plustek8200i > "$CFG/dll.conf"
  [ -f /etc/sane.d/plustek8200i.conf ] && cp /etc/sane.d/plustek8200i.conf "$CFG/"
  export SANE_CONFIG_DIR=$CFG
  if [ $DEVBACKEND -eq 1 ]; then
    # The backend built in ~/projects/scanner (DESIGN.md §11.4), not the installed package.
    export LD_LIBRARY_PATH=${NV_BACKEND_DIR:-$HOME/projects/scanner/backends/backend}${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
    DRIVER="dev build $LD_LIBRARY_PATH/libsane-plustek8200i.so.1"
  else
    unset LD_LIBRARY_PATH
    DRIVER="installed $(rpm -q sane-backends-plustek8200i 2>/dev/null || echo 'sane-backends-plustek8200i (not installed by rpm)')"
  fi
  echo "driver: $DRIVER"
  export SANE_DEBUG_PLUSTEK8200I=${SANE_DEBUG_PLUSTEK8200I:-3}
  [ $STREAMCHECK -eq 1 ] && export PLUSTEK8200I_STREAMCHECK=1
  MATCH="plustek8200i"; BUILD=release
fi

PASS=0; FAIL=0; WARN=0; SUMMARY="$OUT/summary.txt"; : > "$SUMMARY"
record() { # status name detail
  printf '%-5s %-34s %s\n' "$1" "$2" "$3" | tee -a "$SUMMARY"
  case "$1" in PASS) PASS=$((PASS+1));; FAIL) FAIL=$((FAIL+1));; *) WARN=$((WARN+1));; esac
}
say() { echo; echo "== $*"; }

cleanup() {
  [ -n "${SVC:-}" ] && kill "$SVC" 2>/dev/null
  [ -n "${GRANT:-}" ] && kill "$GRANT" 2>/dev/null
  [ -n "${CFG:-}" ] && rm -rf "$CFG"
}
trap cleanup EXIT

# The scanner's USB address, read from sysfs (this asks the device for nothing). If it changes
# during the run, the scanner was reset and re-enumerated, and every device name resolved before
# is stale, so the run stops instead of scanning with a stale name.
usb_addr() {
  local d
  for d in /sys/bus/usb/devices/*; do
    if [ "$(cat "$d/idVendor" 2>/dev/null)" = 07b3 ] && [ "$(cat "$d/idProduct" 2>/dev/null)" = 1825 ]; then
      echo "$(cat "$d/busnum"):$(cat "$d/devnum")"; return
    fi
  done
}
usb_stable() { # where
  [ $DRY -eq 1 ] && return 0
  local now; now=$(usb_addr)
  if [ "$now" != "$USB0" ]; then
    record FAIL "USB address unchanged ($1)" "was $USB0, now ${now:-gone}: the scanner reset or re-enumerated; stopping"
    exit 1
  fi
}

# ---- build -----------------------------------------------------------------------------
say "building ($BUILD)"
FLAG=""; [ "$BUILD" = release ] && FLAG="--release"
(cd "$ROOT" && cargo build $FLAG --features portal -p nv-cli -p nv-portal-proto 2>&1 | tail -2) || { echo "build failed"; exit 1; }
BIN="$ROOT/target/$BUILD"
NVCI="$BIN/nv-cli"; SVCBIN="$BIN/nv-portal-proto"
cd "$ROOT"   # nv-cli reads data/devices.toml relative to here

# ---- C0: the scanner is usable, directly ------------------------------------------------
say "C0 preflight"
LIST=$(scanimage -L 2>/dev/null)
DEVLINE=$(echo "$LIST" | grep -i -- "$MATCH" | head -1)
OTHERS=$(echo "$LIST" | grep '^device' | grep -vic -- "$MATCH")
if [ -z "$DEVLINE" ]; then
  record FAIL "C0 scanner visible to libsane" "scanimage -L lists no '$MATCH'"
  cat <<EOF

The scanner is not usable from Linux. If the Windows VM has it:
  virsh -c qemu:///system detach-device win11 ~/VirtualMachines/harness/etc/hostdev-plustek.xml --live
If it is wedged (timeouts, "DMA setup failed"): ~/projects/scanner/kasa_control.py off; sleep 5;
~/projects/scanner/kasa_control.py on; then wait for: lsusb | grep 07b3
EOF
  exit 1
fi
DEV=$(echo "$DEVLINE" | sed -E "s/^device \`([^']*)'.*/\1/")
record PASS "C0 scanner visible to libsane" "$DEV"
if [ "$OTHERS" -gt 0 ]; then
  record FAIL "C0 only the 8200i's backend is listed" "$OTHERS other device(s) listed: the private configuration is not in effect; stopping before anything is opened"
  exit 1
fi
record PASS "C0 only the 8200i's backend is listed" ""
if [ $DRY -eq 0 ]; then
  USB0=$(usb_addr)
  [ -n "$USB0" ] || { record FAIL "C0 scanner found on USB" "no 07b3:1825 in sysfs"; exit 1; }
  record PASS "C0 USB address recorded" "$USB0"
fi
record PASS "C0 driver under test" "$DRIVER"
if [ $DRY -eq 0 ] && [ $YES -eq 0 ]; then
  read -r -p "Reference frame (tulip negative, strip holder) loaded and untouched from now on? [Enter] " _
fi

# ---- the portal prototype ---------------------------------------------------------------
if [ "$FRONTEND" = c ]; then
  # The frontend, with the stand-in for the dialog that grants the first scanner it is offered.
  # Only the 8200i's backend is configured, so that is the scanner. The helper's messages go to
  # the frontend's stderr, which is what the service log is for the check at the end.
  PORTALS=$(mktemp -d); cat > "$PORTALS/nvtest.portal" <<PORTALFILE
[portal]
DBusName=org.freedesktop.impl.portal.desktop.nvtest
Interfaces=org.freedesktop.impl.portal.Scanner
UseIn=nvtest
PORTALFILE
  export XDG_CURRENT_DESKTOP=nvtest XDG_DESKTOP_PORTAL_DIR=$PORTALS
  export XDG_DATA_HOME=$PORTALS/data XDG_STATE_HOME=$PORTALS/state XDG_CACHE_HOME=$PORTALS/cache
  export XDP_SCANNER_HELPER=$XDP_BUILD/desktop-portal/xdg-desktop-portal-scanner-helper
  python3 "$ROOT/crates/nv-portal-proto/tests/impl_auto_grant.py" org.freedesktop.impl.portal.desktop.nvtest \
      "$ROOT/docs/scanner-portal/org.freedesktop.impl.portal.Scanner.xml" > "$OUT/dialog-standin.log" 2>&1 &
  GRANT=$!
  "$XDP_BUILD/desktop-portal/xdg-desktop-portal" -r 2> "$OUT/service.log" &
  SVC=$!
  WHAT="frontend in C (pid $SVC)"
else
  "$SVCBIN" --select "$MATCH" 2> "$OUT/service.log" &
  SVC=$!
  WHAT="portal prototype (pid $SVC)"
fi
for _ in $(seq 1 50); do busctl --user status "$PORTAL_BUS" >/dev/null 2>&1 && break; sleep 0.2; done
busctl --user status "$PORTAL_BUS" >/dev/null 2>&1 || { record FAIL "C0 portal starts" "see $OUT/service.log"; exit 1; }
record PASS "C0 portal starts" "$WHAT"
PORTAL="env $PENV"

# ---- C1: capabilities agree (no scan) ---------------------------------------------------
say "C1 capabilities and quirks through the portal"
# One device's block of `nv-cli probe`, without its header (the name differs: SANE name vs portal id).
# $2 picks the block whose header contains it; empty takes the first block.
block() { awk -v pat="$2" '/^───/ { if (done) exit; on = (pat == "" || index($0, pat) > 0); if (on) done = 1; next } on && NF { print }' "$1"; }
# stderr apart: with SANE_DEBUG_PLUSTEK8200I set the driver logs there and would pollute the diff.
"$NVCI" probe > "$OUT/probe-direct.txt" 2> "$OUT/probe-direct.err"
$PORTAL "$NVCI" probe > "$OUT/probe-portal.txt" 2> "$OUT/probe-portal.err"
block "$OUT/probe-direct.txt" "$MATCH" > "$OUT/probe-direct.block"
block "$OUT/probe-portal.txt" "" > "$OUT/probe-portal.block"
if [ -s "$OUT/probe-direct.block" ] && diff "$OUT/probe-direct.block" "$OUT/probe-portal.block" >"$OUT/probe.diff"; then
  record PASS "C1 capabilities identical" "$(grep -c . "$OUT/probe-portal.block") lines"
else
  record FAIL "C1 capabilities identical" "see $OUT/probe.diff (a quirks entry not matching through the portal shows up here)"
fi
# Informational: what the portal hides from the app.
"$NVCI" probe --verbose 2>/dev/null | grep -oE '\] [a-z0-9-]+' | sort -u > "$OUT/options-direct.txt"
$PORTAL "$NVCI" probe --verbose 2>/dev/null | grep -oE '\] [a-z0-9-]+' | sort -u > "$OUT/options-portal.txt"
HIDDEN=$(comm -23 "$OUT/options-direct.txt" "$OUT/options-portal.txt" | sed 's/^] //' | tr '\n' ' ')
NEWOPT=$(comm -13 "$OUT/options-direct.txt" "$OUT/options-portal.txt" | sed 's/^] //' | tr '\n' ' ')
echo "  options libsane shows and the portal hides: ${HIDDEN:-(none)}"
echo "  options only the portal shows: ${NEWOPT:-(none)}"

usb_stable "after the probes"

# ---- helpers for the comparisons --------------------------------------------------------
align() { # name ref file  -> PASS/FAIL by H1: same size, |dy| <= 0.2 rows, dx 0, ncc >= 0.999
  local out; out=$($CHECK align "$2" "$3" 2>&1)
  echo "$out" | sed 's/^/    /' >&2
  if echo "$out" | grep -q "sizes differ"; then echo "FAIL|$1|sizes differ"; return; fi
  python3 -I - "$1" "$out" <<'PY'
import re, sys
name, out = sys.argv[1], sys.argv[2]
m = re.search(r"dy=([+-][\d.]+) rows\s+dx=([+-]\d+) px\s+\(ncc ([\d.]+|nan)\)", out)
if not m:
    print("FAIL|" + name + "|could not read: " + out.strip()[:80].replace("\n", " ")); sys.exit(0)
dy, dx, ncc = float(m.group(1)), int(m.group(2)), float(m.group(3))
ok = abs(dy) <= 0.2 and dx == 0 and ncc >= 0.999
print(("PASS|" if ok else "FAIL|") + name + f"|dy={dy:+.2f} dx={dx:+d} ncc={ncc:.4f}")
PY
}
tally() { # read "STATUS|name|detail" lines on stdin into the summary; call as: tally < <(cmd)
  while IFS='|' read -r st name rest; do record "$st" "$name" "$rest"; done
}
exact() { # name a b : informational for hardware (scans differ by noise), the test for --dry-run
  local out; out=$($CHECK diff "$2" "$3" 2>&1); echo "    $out"
  if [ $DRY -eq 1 ]; then
    if echo "$out" | grep -q "^0 of "; then record PASS "$1" "bit-identical"; else record FAIL "$1" "$out"; fi
  fi
}

# ---- C2: reference ---------------------------------------------------------------------
say "C2 reference scan with scanimage ($DPI dpi)"
REFARGS="--mode Color --resolution $DPI"
scanimage -d "$DEV" --help 2>/dev/null | grep -q -- "--depth" && [ $DRY -eq 0 ] && REFARGS="$REFARGS --depth 16"
if [ $DRY -eq 0 ]; then
  scanimage -d "$DEV" --format=pnm $REFARGS > "$OUT/ref.pnm" 2> "$OUT/ref.log" && record PASS "C2 reference scan" "$REFARGS" || { record FAIL "C2 reference scan" "see $OUT/ref.log"; exit 1; }
else
  record PASS "C2 reference scan" "skipped in --dry-run (test picture is flat)"
fi

usb_stable "after the reference scan"

# ---- C3 / C4: the app, direct and through the portal -------------------------------------
say "C3 app scan, libsane in the app"
limited "$NVCI" scan "$DEV" --dpi "$DPI" -o "$OUT/direct.pnm" > "$OUT/direct.log" 2>&1 && record PASS "C3 direct scan completes" "$(grep -E '^Saved' "$OUT/direct.log" | sed 's/ to .*//')" || record FAIL "C3 direct scan completes" "see $OUT/direct.log"

say "C4 app scan through the portal (progress timed)"
python3 -I "$HERE/progress-marks.py" "$OUT/portal.log" -- timeout --signal=TERM --kill-after=10 "$TMO" env $PENV "$NVCI" scan --dpi "$DPI" -o "$OUT/portal.pnm" > "$OUT/portal-progress.txt" 2>&1
PRC=$?
cat "$OUT/portal-progress.txt" | sed 's/^/    /'
if grep -q '^Saved' "$OUT/portal.log"; then record PASS "C4 portal scan completes" "$(grep -E '^Saved' "$OUT/portal.log" | sed 's/ to .*//')"; else record FAIL "C4 portal scan completes" "see $OUT/portal.log"; fi
if [ $DRY -eq 1 ]; then
  record WARN "H3 progress climbs (portal)" "not judged in --dry-run: the test backend is too fast"
elif [ $PRC -eq 0 ]; then
  record PASS "H3 progress climbs (portal)" "$(grep -o 'intermediate_steps=[0-9]*' "$OUT/portal-progress.txt")"
else
  record FAIL "H3 progress climbs (portal)" "$(grep -o 'progress=.*' "$OUT/portal-progress.txt")"
fi

usb_stable "after the app scans"
if [ -s "$OUT/direct.pnm" ] && [ -s "$OUT/portal.pnm" ]; then
  say "H1 direct and portal against the reference, and against each other"
  if [ $DRY -eq 0 ]; then
    tally < <(align "H1 direct vs reference" "$OUT/ref.pnm" "$OUT/direct.pnm")
    tally < <(align "H1 portal vs reference" "$OUT/ref.pnm" "$OUT/portal.pnm")
    tally < <(align "H1 portal vs direct" "$OUT/direct.pnm" "$OUT/portal.pnm")
  fi
  exact "portal output equals direct output" "$OUT/direct.pnm" "$OUT/portal.pnm"
fi

# ---- C5: repeatability through the portal -------------------------------------------------
if ! skipped repeat && [ $DRY -eq 0 ]; then
  say "C5 repeatability through the portal (H2)"
  limited env $PENV "$NVCI" scan --dpi "$DPI" -o "$OUT/portal2.pnm" > "$OUT/portal2.log" 2>&1
  [ -s "$OUT/portal2.pnm" ] && tally < <(align "H2 portal repeat vs first" "$OUT/portal.pnm" "$OUT/portal2.pnm") || record FAIL "H2 portal repeat" "see $OUT/portal2.log"
fi

usb_stable "after the repeat scan"

# ---- C6: infrared -------------------------------------------------------------------------
c6() {
  limited "$NVCI" scan "$DEV" --dpi "$DPI" --ir -o "$OUT/direct-ir.pnm" > "$OUT/direct-ir.log" 2>&1
  limited env $PENV "$NVCI" scan --dpi "$DPI" --ir -o "$OUT/portal-ir.pnm" > "$OUT/portal-ir.log" 2>&1
  for w in direct portal; do
    if [ -s "$OUT/$w-ir.pnm" ] && [ -s "$OUT/$w-ir-ir.pgm" ]; then
      align "H5 $w colour vs reference" "$OUT/ref.pnm" "$OUT/$w-ir.pnm"
      $CHECK iralign "$OUT/$w-ir.pnm" "$OUT/$w-ir-ir.pgm" 2>&1 | tee "$OUT/$w-iralign.txt" | sed 's/^/    /' >&2
      if grep -q "dy=+0 dx=+0" "$OUT/$w-iralign.txt"; then echo "PASS|H5 $w IR aligned with colour|dy 0, dx 0"; else echo "FAIL|H5 $w IR aligned with colour|$(grep 'IR vs green' "$OUT/$w-iralign.txt")"; fi
    else
      echo "FAIL|H5 $w IR scan produced files|see $OUT/$w-ir.log"
    fi
  done
  python3 -I - "$OUT/direct-iralign.txt" "$OUT/portal-iralign.txt" <<'PY'
import re, sys
def med(p):
    try:
        m = re.search(r"median (\d+)", open(p).read())
    except OSError:
        return None
    return int(m.group(1)) if m else None
d, p = med(sys.argv[1]), med(sys.argv[2])
if d and p:
    rel = abs(d - p) / d
    print(("PASS|" if rel <= 0.1 else "FAIL|") + f"H5 IR level direct vs portal|median {d} vs {p} ({rel*100:.1f}% apart)")
else:
    print("FAIL|H5 IR level direct vs portal|could not read the IR medians")
PY
}
if ! skipped ir && [ $DRY -eq 0 ]; then
  say "C6 RGB + infrared, direct and through the portal (H5)"
  tally < <(c6)
fi

usb_stable "after the infrared scans"

# ---- C8: cancel, then scan again, through the portal ---------------------------------------
if ! skipped cancel; then
  say "C8 cancel at 30% of a big scan, then scan again (H4)"
  if [ $DRY -eq 1 ]; then CD=300; RD=75; SETS="--set mode=Color"; else CD=1200; RD=$DPI; SETS="--set mode=Color"; fi
  # The nv-cli scans above have released the scanner; the service holds it only while a session is open.
  limited env PORTAL_NAME="$PORTAL_BUS" python3 "$ROOT/crates/nv-portal-proto/tests/hw_cancel.py" $SETS --cancel-dpi $CD --rescan-dpi $RD --out "$OUT/portal-after-cancel.pnm" 2>&1 | tee "$OUT/cancel.txt" | sed 's/^/    /'
  if grep -q 'cancelled=1 fast=1' "$OUT/cancel.txt"; then record PASS "H4 portal cancel returns promptly" "$(grep -o 'cancel_seconds=[0-9.]*' "$OUT/cancel.txt")"; else record FAIL "H4 portal cancel returns promptly" "see $OUT/cancel.txt"; fi
  if grep -q 'rescan_ok=1' "$OUT/cancel.txt"; then
    record PASS "H4 scan after cancel completes" ""
    [ $DRY -eq 0 ] && tally < <(align "H4 scan after cancel vs reference" "$OUT/ref.pnm" "$OUT/portal-after-cancel.pnm")
  else
    record FAIL "H4 scan after cancel completes" "see $OUT/cancel.txt"
  fi
fi

usb_stable "after the cancel check"

# ---- C9: logs ------------------------------------------------------------------------------
say "C9 logs"
BAD=$(grep -c -i -E "DMA setup failed|timed out|timeout|wedged" "$OUT/service.log" 2>/dev/null)
if [ "${BAD:-0}" -eq 0 ]; then record PASS "C9 no timeouts or DMA errors in the service log" ""; else record WARN "C9 timeouts or DMA errors in the service log" "$BAD lines in $OUT/service.log"; fi
if [ $STREAMCHECK -eq 1 ]; then
  grep -i "samples differ" "$OUT/service.log" | tail -3 | sed 's/^/    /'
fi

# ---- summary --------------------------------------------------------------------------------
echo
echo "================ $PASS passed, $FAIL failed, $WARN warnings ================"
echo "files and logs: $OUT"
[ $DRY -eq 1 ] && echo "(dry run: the SANE test backend, so thresholds were not applied)"
[ $FAIL -eq 0 ]
