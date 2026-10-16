#!/usr/bin/env python3
"""H4 through the portal, on a real scanner: cancel a big scan part-way, then scan again.

Needs a running prototype service (see tools/portal-hw-check.sh). Cancels a frame at about
--cancel-at of its size, times how long the portal takes to say `cancelled`, then scans a
whole frame on the same session at --rescan-dpi and writes it as a PNM so the caller can
align it with a reference (H1).

Prints `RESULT key=value ...` lines; exits 1 if a check fails.
"""
import argparse
import os
import sys
import time

from gi.repository import GLib

from portal_client import Client, v


def typed(descs, name, text):
    d = next((d for d in descs if d["name"] == name), None)
    if d is None:
        raise SystemExit(f"the scanner has no option '{name}'")
    t = d["type"]
    if t == "bool":
        return v("b", text in ("1", "true", "yes"))
    if t == "int":
        return v("i", int(text))
    if t == "fixed":
        return v("d", float(text))
    return v("s", text)


def scan_frames(c, sess):
    frames = []
    while True:
        n = len(c.events)
        fd = c.start_frame(sess)
        with os.fdopen(fd, "rb") as f:
            data = f.read()
        i, ev = c.wait_state(sess, ("finished", "error", "cancelled", "no-documents"), n, timeout=300)
        started = next(e for e in c.events[n:] if e[0] == sess and e[1] == "started")[2]
        if ev is None or ev[1] != "finished":
            raise SystemExit(f"frame ended with {ev}")
        frames.append((started, data))
        if started["last_frame"]:
            return frames


def write_pnm(path, frames):
    p = frames[0][0]
    bpp = max(p["depth"], 8) // 8
    w, h = p["pixels_per_line"], p["lines"]

    def rows(frame):
        meta, data = frame
        channels = 3 if meta["format"] == "rgb" else 1
        row_bytes = meta["pixels_per_line"] * bpp * channels
        bpl = meta["bytes_per_line"]
        return [data[i * bpl:i * bpl + row_bytes] for i in range(len(data) // bpl)]

    def swap(b):
        if bpp == 2 and sys.byteorder == "little":
            a = bytearray(b)
            a[0::2], a[1::2] = b[1::2], b[0::2]
            return bytes(a)
        return b

    maxval = 65535 if p["depth"] > 8 else 255
    with open(path, "wb") as out:
        out.write(f"{'P5' if p['format'] == 'gray' else 'P6'}\n{w} {h}\n{maxval}\n".encode())
        if p["format"] == "red":
            r, g, b = (rows(f) for f in frames[:3])
            for y in range(len(r)):
                line = bytearray()
                for x in range(w):
                    for plane in (r, g, b):
                        line += plane[y][x * bpp:(x + 1) * bpp]
                out.write(swap(bytes(line)))
        else:
            for row in rows(frames[0]):
                out.write(swap(row))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cancel-dpi", type=int, default=1200)
    ap.add_argument("--cancel-at", type=float, default=0.3)
    ap.add_argument("--rescan-dpi", type=int, default=600)
    ap.add_argument("--max-cancel-seconds", type=float, default=2.0)
    ap.add_argument("--set", action="append", default=[], metavar="NAME=VALUE", help="options applied first, in order")
    ap.add_argument("--out", required=True, help="PNM file for the scan after the cancel")
    a = ap.parse_args()

    c = Client()
    code, results = c.select(multiple=v("b", False))
    if code != 0 or not results.get("scanners"):
        raise SystemExit(f"no scanner selected ({code})")
    sid = results["scanners"][0][0]
    sess = c.open(sid, "hwcancel")
    descs = c.opts(sess)
    ok = True

    def settings(dpi):
        pairs = [(k, typed(descs, k, val)) for k, val in (s.split("=", 1) for s in a.set)]
        pairs.append(("resolution", typed(descs, "resolution", str(dpi))))
        return pairs

    # ---- cancel part-way
    c.set(sess, settings(a.cancel_dpi))
    (est,) = c.call("GetFrameParameters", (sess, {}), "(oa{sv})")
    total = est["bytes_per_line"] * est["lines"]
    n = len(c.events)
    fd = c.start_frame(sess)
    i, ev = c.wait_state(sess, ("started", "error", "no-documents"), n, timeout=300)
    if ev is None or ev[1] != "started":
        raise SystemExit(f"frame did not start: {ev}")
    goal = int(total * a.cancel_at)
    got = 0
    marks = []
    t_start = time.time()
    while got < goal:
        chunk = os.read(fd, 1 << 20)
        if not chunk:
            break
        got += len(chunk)
        if len(marks) < 10 and got * 10 // goal >= len(marks):
            marks.append(round(time.time() - t_start, 1))
    if got < goal:
        print(f"RESULT cancel_ok=0 reason=frame ended after {got} of {total} bytes")
        return 1
    t0 = time.time()
    c.call("Cancel", (sess,), "(o)")
    i, ev = c.wait_state(sess, ("cancelled", "finished", "error"), n, timeout=30)
    took = time.time() - t0
    os.close(fd)
    cancelled = ev is not None and ev[1] == "cancelled"
    fast = took <= a.max_cancel_seconds
    print(f"RESULT cancel_bytes={got} of={total} fraction={got / total:.2f} cancel_seconds={took:.2f} cancelled={int(cancelled)} fast={int(fast)}")
    print(f"RESULT read_progress_seconds_per_tenth={marks}")
    ok &= cancelled and fast

    # ---- scan again on the same session
    try:
        c.set(sess, settings(a.rescan_dpi))
        frames = scan_frames(c, sess)
        write_pnm(a.out, frames)
        print(f"RESULT rescan_ok=1 frames={len(frames)} file={a.out}")
    except SystemExit as e:
        print(f"RESULT rescan_ok=0 reason={e}")
        ok = False
    c.call("Close", (), "()", iface="org.freedesktop.portal.Session", path=sess)
    return 0 if ok else 1


sys.exit(main())
