#!/usr/bin/env python3
"""Run a command and report the progress percentages it prints, against time (H3).

    progress-marks.py LOGFILE -- COMMAND [ARGS...]

The command's output goes to LOGFILE. Prints `RESULT ...` lines. Passes when the percentage
climbs in several steps rather than jumping from 0 to 100% at the end. Exit status is the
command's if it failed, 1 if the progress did not climb, else 0.
"""
import os
import re
import subprocess
import sys
import time


def main():
    if len(sys.argv) < 4 or sys.argv[2] != "--":
        raise SystemExit(__doc__)
    log, cmd = sys.argv[1], sys.argv[3:]
    t0 = time.time()
    marks = []  # (seconds, percent)
    p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    tail = b""
    with open(log, "wb") as out:
        while True:
            chunk = os.read(p.stdout.fileno(), 4096)
            if not chunk:
                break
            out.write(chunk)
            tail = (tail + chunk)[-64:]
            now = time.time() - t0
            # A percentage may straddle two reads, so scan the tail but only count new values.
            for m in re.finditer(rb"(\d{1,3})%", tail):
                v = int(m.group(1))
                if not marks or v > marks[-1][1]:
                    marks.append((now, v))
    rc = p.wait()
    total = time.time() - t0
    steps = [v for _, v in marks if 0 < v < 100]
    print(f"RESULT exit={rc} total_seconds={total:.1f}")
    print("RESULT progress=" + " ".join(f"{v}%@{t:.1f}s" for t, v in marks))
    steady = len(steps) >= 3
    print(f"RESULT intermediate_steps={len(steps)} steady={int(steady)}")
    if rc != 0:
        return rc
    return 0 if steady else 1


sys.exit(main())
