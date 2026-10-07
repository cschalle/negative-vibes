#!/usr/bin/env python3
"""scan-check.py -- measurements for scans and for Negative Vibes output.

These are the checks used while bringing up the plustek8200i SANE driver,
collected in one place so the application can be tested the same way.

  stats    FILE                 size, per-channel levels, clipping, flatness
  align    REF FILE [--range N] shift of FILE against REF, to 0.05 row
  bands    REF FILE [--dy N]    match per horizontal band (top to bottom)
  iralign  RGB IR               offset of an IR plane against the colour image
  diff     A B                  exact comparison (bit-identity tests)
  down     IN OUT FACTOR        block-average downsample (compare resolutions)
  preview  IN OUT.png [--positive]
                                quick 8-bit rendering (inverted unless --positive)
  progress [--] SCANIMAGE-ARGS  run scanimage -p, print progress against time

Files: PNM (P5/P6, 8 or 16 bit), or TIFF if python3-tifffile is installed.
Images are compared on the mean of their channels; FILE may be at another
resolution only for `align` after `down`.

Requires numpy; `preview` also needs Pillow.
"""
import argparse
import os
import re
import subprocess
import sys
import time

import numpy as np


# --------------------------------------------------------------------------
# Loading

def _load_pnm(path):
    with open(path, "rb") as f:
        magic = f.readline().strip()
        if magic not in (b"P5", b"P6"):
            raise SystemExit(f"{path}: not a binary PNM (P5/P6)")

        def token():
            t = b""
            while True:
                c = f.read(1)
                if not c:
                    return t
                if c == b"#" and not t:
                    f.readline()
                    continue
                if c.isspace():
                    if t:
                        return t
                    continue
                t += c

        w, h, mx = int(token()), int(token()), int(token())
        ch = 3 if magic == b"P6" else 1
        dt = ">u2" if mx > 255 else "u1"
        a = np.frombuffer(f.read(), dtype=dt)
        if a.size < w * h * ch:
            raise SystemExit(f"{path}: truncated ({a.size} of {w * h * ch} samples)")
        return a[: w * h * ch].reshape(h, w, ch).astype(np.float64), mx


def _load_tiff(path):
    try:
        import tifffile
    except ImportError:
        raise SystemExit("reading TIFF needs python3-tifffile "
                         "(dnf install python3-tifffile)")
    a = tifffile.imread(path)
    if a.ndim == 2:
        a = a[:, :, None]
    mx = 65535 if a.dtype == np.uint16 else 255 if a.dtype == np.uint8 else 1.0
    return a.astype(np.float64), mx


def load(path):
    if path.lower().endswith((".tif", ".tiff")):
        return _load_tiff(path)
    return _load_pnm(path)


def gray(a):
    return a[:, :, :3].mean(axis=2) if a.shape[2] >= 3 else a[:, :, 0]


def ncc(x, y):
    x = (x - x.mean()) / (x.std() + 1e-12)
    y = (y - y.mean()) / (y.std() + 1e-12)
    return float((x * y).mean())


def interior(h, w, frac=0.1):
    return slice(int(h * frac), int(h * (1 - frac))), slice(int(w * frac), int(w * (1 - frac)))


# --------------------------------------------------------------------------
# Commands

def cmd_stats(args):
    a, mx = load(args.file)
    h, w, ch = a.shape
    ys, xs = interior(h, w)
    print(f"{args.file}: {w}x{h}, {ch} channel(s), max {mx}, aspect {w / h:.3f}")
    for c in range(ch):
        v = a[:, :, c]
        vi = v[ys, xs]
        lo, hi = np.percentile(vi, [0.5, 99.5])
        print(f"  ch{c}: mean {v.mean():8.0f}  p0.5 {lo:7.0f}  p99.5 {hi:7.0f}  "
              f"clip-high {100 * (v >= mx).mean():.2f}%  clip-low {100 * (v <= 0).mean():.2f}%  "
              f"distinct {len(np.unique(vi.astype(np.int64)))}")
    cm = a[ys, xs, :].mean(axis=(0, 2))
    print(f"  column-profile CV {100 * cm.std() / cm.mean():.2f}%  (flatness: only "
          "meaningful for a blank, evenly lit scan, where shading correction keeps it "
          "well under 1%)")


def _shift_search(g1, g2, m, mx_dx=8):
    h, w = g1.shape
    core = g1[m:h - m, m:w - m]
    best = (-9.0, 0, 0)
    for dy in range(-m, m + 1):
        for dx in range(-mx_dx, mx_dx + 1):
            v = ncc(core, g2[m + dy:h - m + dy, m + dx:w - m + dx])
            if v > best[0]:
                best = (v, dy, dx)
    # Sub-row refinement along y (parabola through the three best rows).
    v, dy, dx = best
    frac = 0.0
    if -m < dy < m:
        y0 = ncc(core, g2[m + dy - 1:h - m + dy - 1, m + dx:w - m + dx])
        y2 = ncc(core, g2[m + dy + 1:h - m + dy + 1, m + dx:w - m + dx])
        den = y0 - 2 * v + y2
        if den != 0:
            frac = 0.5 * (y0 - y2) / den
    return dy + frac, dx, v


def cmd_align(args):
    a, _ = load(args.ref)
    b, _ = load(args.file)
    g1, g2 = gray(a), gray(b)
    if g1.shape != g2.shape:
        h, w = min(g1.shape[0], g2.shape[0]), min(g1.shape[1], g2.shape[1])
        g1, g2 = g1[:h, :w], g2[:h, :w]
        print(f"note: sizes differ, comparing the top-left {w}x{h}")
    dy, dx, v = _shift_search(g1, g2, args.range)
    print(f"shift dy={dy:+.2f} rows  dx={dx:+d} px  (ncc {v:.4f})")
    print("  dy > 0: FILE's content sits lower than REF's (the frame started earlier)")


def cmd_bands(args):
    a, _ = load(args.ref)
    b, _ = load(args.file)
    g1, g2 = gray(a), gray(b)
    dy = args.dy
    if dy > 0:
        g2 = g2[dy:]
        g1 = g1[:len(g2)]
    elif dy < 0:
        g1 = g1[-dy:]
        g2 = g2[:len(g1)]
    h = min(len(g1), len(g2))
    n = args.n
    out = []
    for i in range(n):
        s = slice(i * h // n, (i + 1) * h // n)
        out.append(f"{ncc(g1[s, 20:-20], g2[s, 20:-20]):.4f}")
    print("band ncc, top to bottom:", " ".join(out))
    print("  even values: a uniform (sub-row) offset; a dip at the top: settling or vibration")


def _edges(a):
    gy, gx = np.gradient(a)
    return np.hypot(gx, gy)


def cmd_iralign(args):
    rgb, _ = load(args.rgb)
    ir, _ = load(args.ir)
    g = rgb[:, :, 1] if rgb.shape[2] >= 3 else rgb[:, :, 0]
    e1, e2 = _edges(g), _edges(ir[:, :, 0])
    h, w = e1.shape
    m = args.range
    core = e1[m:h - m, m:w - m]
    best = (-9.0, 0, 0)
    for dy in range(-m, m + 1):
        for dx in range(-m, m + 1):
            v = ncc(core, e2[m + dy:h - m + dy, m + dx:w - m + dx])
            if v > best[0]:
                best = (v, dy, dx)
    print(f"IR vs green edges: dy={best[1]:+d} dx={best[2]:+d} (ncc {best[0]:.3f})")
    v = ir[:, :, 0][interior(h, w)[0], interior(h, w)[1]]
    print(f"IR interior: median {np.median(v):.0f}, p1 {np.percentile(v, 1):.0f}, "
          f"p99 {np.percentile(v, 99):.0f}")


def cmd_diff(args):
    a, _ = load(args.a)
    b, _ = load(args.b)
    if a.shape != b.shape:
        print(f"DIFFERENT SIZE: {a.shape} vs {b.shape}")
        sys.exit(1)
    d = np.abs(a - b)
    n = int((d > 0).sum())
    print(f"{n} of {d.size} samples differ, max difference {d.max():.0f}, "
          f"mean {d.mean():.3f}")
    sys.exit(0 if n == 0 else 1)


def cmd_down(args):
    a, mx = load(args.inp)
    f = args.factor
    h, w, c = a.shape
    h -= h % f
    w -= w % f
    b = a[:h, :w].reshape(h // f, f, w // f, f, c).mean((1, 3))
    dt = ">u2" if mx > 255 else "u1"
    magic = "P6" if c == 3 else "P5"
    with open(args.out, "wb") as o:
        o.write(f"{magic}\n{w // f} {h // f}\n{int(mx)}\n".encode())
        o.write(np.clip(b + 0.5, 0, mx).astype(dt).tobytes())


def cmd_preview(args):
    from PIL import Image
    a, _ = load(args.inp)
    h, w, ch = a.shape
    ys, xs = interior(h, w)
    r = np.empty_like(a)
    for c in range(ch):
        v = a[:, :, c]
        lo, hi = np.percentile(v[ys, xs], [0.5, 99.5])
        r[:, :, c] = np.clip((v - lo) / (hi - lo + 1e-9), 0, 1)
    if not args.positive:
        r = 1 - r
    img = (r ** (1 / 2.2) * 255).astype(np.uint8)
    Image.fromarray(img if ch == 3 else img[:, :, 0]).save(args.out)
    print(f"wrote {args.out} (a rough check only, not the app's conversion)")


def cmd_progress(args):
    cmd = ["scanimage", "-p"] + [x for x in args.scanimage_args if x != "--"]
    out = args.output
    t0 = time.time()
    marks = []
    last = -10.0
    with open(out, "wb") as o:
        p = subprocess.Popen(cmd, stdout=o, stderr=subprocess.PIPE, env=os.environ)
        buf = b""
        while True:
            ch = p.stderr.read(1)
            if not ch:
                break
            if ch in b"\r\n":
                m = re.search(rb"Progress: ([0-9.]+)%", buf)
                if m:
                    v = float(m.group(1))
                    if v - last >= 10 or (v >= 100 and last < 100):
                        marks.append((time.time() - t0, v))
                        last = v
                buf = b""
            else:
                buf += ch
        p.wait()
    print(f"scanimage exit {p.returncode}, total {time.time() - t0:.1f} s, output {out}")
    print("progress: " + "  ".join(f"{v:.0f}%@{t:.1f}s" for t, v in marks))
    print("  a streaming driver climbs steadily; a jump from 0 to 100% at the end means "
          "the backend buffers the whole scan before sane_read")
    sys.exit(p.returncode)


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("stats"); s.add_argument("file"); s.set_defaults(fn=cmd_stats)
    s = sub.add_parser("align"); s.add_argument("ref"); s.add_argument("file")
    s.add_argument("--range", type=int, default=20); s.set_defaults(fn=cmd_align)
    s = sub.add_parser("bands"); s.add_argument("ref"); s.add_argument("file")
    s.add_argument("--dy", type=int, default=0); s.add_argument("-n", type=int, default=8)
    s.set_defaults(fn=cmd_bands)
    s = sub.add_parser("iralign"); s.add_argument("rgb"); s.add_argument("ir")
    s.add_argument("--range", type=int, default=6); s.set_defaults(fn=cmd_iralign)
    s = sub.add_parser("diff"); s.add_argument("a"); s.add_argument("b"); s.set_defaults(fn=cmd_diff)
    s = sub.add_parser("down"); s.add_argument("inp"); s.add_argument("out")
    s.add_argument("factor", type=int); s.set_defaults(fn=cmd_down)
    s = sub.add_parser("preview"); s.add_argument("inp"); s.add_argument("out")
    s.add_argument("--positive", action="store_true"); s.set_defaults(fn=cmd_preview)
    s = sub.add_parser("progress"); s.add_argument("-o", "--output", default="progress-scan.pnm")
    s.add_argument("scanimage_args", nargs=argparse.REMAINDER); s.set_defaults(fn=cmd_progress)
    args = ap.parse_args()
    args.fn(args)


if __name__ == "__main__":
    main()
