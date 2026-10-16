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
# It never detaches the scanner from the VM or power-cycles it; it tells you how.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
CHECK="python3 $HERE/scan-check.py"

ORIG_ARGS=("$@")   # the parsing below shifts them away; the re-exec into the private bus needs them
DRY=0; DPI=600; OUT=""; YES=0; SKIP=""; STREAMCHECK=0; DEVBACKEND=0
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) DRY=1 ;;
    --dpi) DPI=$2; shift ;;
    --out) OUT=$2; shift ;;
    --yes) YES=1 ;;
    --skip) SKIP=$2; shift ;;
    --streamcheck) STREAMCHECK=1 ;;
    --dev-backend) DEVBACKEND=1 ;;
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
  if [ $DEVBACKEND -eq 1 ]; then
    # The backend built in ~/projects/scanner (DESIGN.md §11.4), not the installed package.
    export SANE_CONFIG_DIR=${SANE_CONFIG_DIR:-$HOME/.sane}
    export LD_LIBRARY_PATH=${NV_BACKEND_DIR:-$HOME/projects/scanner/backends/backend}${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
    if [ ! -f "$SANE_CONFIG_DIR/dll.conf" ]; then
      # Nothing lists the backend: use a throwaway directory that links the real one's files and
      # adds a dll.conf. The real one is not touched.
      OVERLAY=$(mktemp -d)
      ln -s "$SANE_CONFIG_DIR"/* "$OVERLAY"/ 2>/dev/null
      echo plustek8200i > "$OVERLAY/dll.conf"
      export SANE_CONFIG_DIR=$OVERLAY
    fi
    DRIVER="dev build $LD_LIBRARY_PATH/libsane-plustek8200i.so.1 (config $SANE_CONFIG_DIR)"
  else
    # The installed sane-backends-plustek8200i package: it registers itself in
    # /etc/sane.d/dll.d, so the system SANE configuration is all that is needed.
    unset SANE_CONFIG_DIR LD_LIBRARY_PATH
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
  [ $DRY -eq 1 ] && rm -rf "$CFG"
  [ -n "${OVERLAY:-}" ] && rm -rf "$OVERLAY"
}
trap cleanup EXIT

# ---- build -----------------------------------------------------------------------------
say "building ($BUILD)"
FLAG=""; [ "$BUILD" = release ] && FLAG="--release"
(cd "$ROOT" && cargo build $FLAG --features portal -p nv-cli -p nv-portal-proto 2>&1 | tail -2) || { echo "build failed"; exit 1; }
BIN="$ROOT/target/$BUILD"
NVCI="$BIN/nv-cli"; SVCBIN="$BIN/nv-portal-proto"
cd "$ROOT"   # nv-cli reads data/devices.toml relative to here

# ---- C0: the scanner is usable, directly ------------------------------------------------
say "C0 preflight"
DEVLINE=$(scanimage -L 2>/dev/null | grep -i -- "$MATCH" | head -1)
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
record PASS "C0 driver under test" "$DRIVER"
if [ $DRY -eq 0 ] && [ $YES -eq 0 ]; then
  read -r -p "Reference frame (tulip negative, strip holder) loaded and untouched from now on? [Enter] " _
fi

# ---- the portal prototype ---------------------------------------------------------------
"$SVCBIN" --select "$MATCH" 2> "$OUT/service.log" &
SVC=$!
for _ in $(seq 1 50); do busctl --user status org.freedesktop.NvPortalProto >/dev/null 2>&1 && break; sleep 0.2; done
busctl --user status org.freedesktop.NvPortalProto >/dev/null 2>&1 || { record FAIL "C0 portal prototype starts" "see $OUT/service.log"; exit 1; }
record PASS "C0 portal prototype starts" "pid $SVC"
PORTAL="env NV_SCANNER_BACKEND=portal"

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

# ---- C3 / C4: the app, direct and through the portal -------------------------------------
say "C3 app scan, libsane in the app"
limited "$NVCI" scan "$DEV" --dpi "$DPI" -o "$OUT/direct.pnm" > "$OUT/direct.log" 2>&1 && record PASS "C3 direct scan completes" "$(grep -E '^Saved' "$OUT/direct.log" | sed 's/ to .*//')" || record FAIL "C3 direct scan completes" "see $OUT/direct.log"

say "C4 app scan through the portal (progress timed)"
python3 -I "$HERE/progress-marks.py" "$OUT/portal.log" -- timeout --signal=TERM --kill-after=10 "$TMO" env NV_SCANNER_BACKEND=portal "$NVCI" scan --dpi "$DPI" -o "$OUT/portal.pnm" > "$OUT/portal-progress.txt" 2>&1
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
  limited env NV_SCANNER_BACKEND=portal "$NVCI" scan --dpi "$DPI" -o "$OUT/portal2.pnm" > "$OUT/portal2.log" 2>&1
  [ -s "$OUT/portal2.pnm" ] && tally < <(align "H2 portal repeat vs first" "$OUT/portal.pnm" "$OUT/portal2.pnm") || record FAIL "H2 portal repeat" "see $OUT/portal2.log"
fi

# ---- C6: infrared -------------------------------------------------------------------------
c6() {
  limited "$NVCI" scan "$DEV" --dpi "$DPI" --ir -o "$OUT/direct-ir.pnm" > "$OUT/direct-ir.log" 2>&1
  limited env NV_SCANNER_BACKEND=portal "$NVCI" scan --dpi "$DPI" --ir -o "$OUT/portal-ir.pnm" > "$OUT/portal-ir.log" 2>&1
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

# ---- C8: cancel, then scan again, through the portal ---------------------------------------
if ! skipped cancel; then
  say "C8 cancel at 30% of a big scan, then scan again (H4)"
  if [ $DRY -eq 1 ]; then CD=300; RD=75; SETS="--set mode=Color"; else CD=1200; RD=$DPI; SETS="--set mode=Color"; fi
  # The nv-cli scans above have released the scanner; the service holds it only while a session is open.
  limited python3 "$ROOT/crates/nv-portal-proto/tests/hw_cancel.py" $SETS --cancel-dpi $CD --rescan-dpi $RD --out "$OUT/portal-after-cancel.pnm" 2>&1 | tee "$OUT/cancel.txt" | sed 's/^/    /'
  if grep -q 'cancelled=1 fast=1' "$OUT/cancel.txt"; then record PASS "H4 portal cancel returns promptly" "$(grep -o 'cancel_seconds=[0-9.]*' "$OUT/cancel.txt")"; else record FAIL "H4 portal cancel returns promptly" "see $OUT/cancel.txt"; fi
  if grep -q 'rescan_ok=1' "$OUT/cancel.txt"; then
    record PASS "H4 scan after cancel completes" ""
    [ $DRY -eq 0 ] && tally < <(align "H4 scan after cancel vs reference" "$OUT/ref.pnm" "$OUT/portal-after-cancel.pnm")
  else
    record FAIL "H4 scan after cancel completes" "see $OUT/cancel.txt"
  fi
fi

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
