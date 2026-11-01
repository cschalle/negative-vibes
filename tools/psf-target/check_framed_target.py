#!/usr/bin/env python3
"""Check a framed PSF target: read its STL back and report what it is.

Rebuilding the height field would only re-run the generator's own arithmetic,
so this reads the STL instead and measures the solid that will actually be
printed.  It prints the bounding box, the thickness histogram, how much
material the frame has at its thinnest point, and whether the aperture is
clear; and with an output path it draws a plan view shaded by height.
"""
import math
import struct
import sys


def read_stl(path):
    with open(path, "rb") as f:
        data = f.read()
    n = struct.unpack("<I", data[80:84])[0]
    tris = []
    for i in range(n):
        o = 84 + i * 50 + 12
        tris.append(tuple(struct.unpack("<3f", data[o + k * 12:o + 12 + k * 12])
                          for k in range(3)))
    return tris


def column_heights(tris, cell=0.2, extent=50.8):
    """Top of the solid over a grid, from the upward-facing triangles."""
    n = int(round(extent / cell))
    half = extent / 2
    top = [[0.0] * n for _ in range(n)]
    seen = [[False] * n for _ in range(n)]
    for a, b, c in tris:
        if not (a[2] == b[2] == c[2]) or a[2] <= 0.0:
            continue
        xs = [p[0] for p in (a, b, c)]
        ys = [p[1] for p in (a, b, c)]
        # Index by cell centre, not by cell edge: boxes meet exactly on cell
        # boundaries, and rounding there lets a taller neighbour claim a cell
        # it does not cover.
        i0 = max(0, math.ceil((min(xs) + half) / cell - 0.5))
        i1 = min(n - 1, math.floor((max(xs) + half) / cell - 0.5))
        j0 = max(0, math.ceil((min(ys) + half) / cell - 0.5))
        j1 = min(n - 1, math.floor((max(ys) + half) / cell - 0.5))
        for j in range(j0, j1 + 1):
            for i in range(i0, i1 + 1):
                if a[2] > top[j][i]:
                    top[j][i] = a[2]
                seen[j][i] = True
    return top, seen, n


def main():
    path = sys.argv[1]
    out = sys.argv[2] if len(sys.argv) > 2 else None
    tris = read_stl(path)
    pts = [p for t in tris for p in t]
    for k, name in ((0, "x"), (1, "y"), (2, "z")):
        lo = min(p[k] for p in pts)
        hi = max(p[k] for p in pts)
        print(f"  {name}: {lo:7.3f} .. {hi:7.3f}  ({hi - lo:6.3f} mm)")
    top, seen, n = column_heights(tris)
    cell = 50.8 / n
    hist = {}
    for j in range(n):
        for i in range(n):
            if seen[j][i]:
                hist[round(top[j][i], 2)] = hist.get(round(top[j][i], 2), 0) + 1
    area = cell * cell
    print(f"  {len(tris)} triangles, {sum(hist.values()) * area:.0f} mm2 covered")
    print("  height          area")
    for h in sorted(hist, reverse=True)[:6]:
        print(f"   {h:5.2f} mm   {hist[h] * area:7.1f} mm2")
    tail = sorted(hist)[:3]
    for h in tail:
        print(f"   {h:5.2f} mm   {hist[h] * area:7.1f} mm2   (lowest)")
    # The aperture must be clear of material at the blade's plane.
    aw, ah = 18.0, 12.0
    bad = 0
    for j in range(n):
        y = -25.4 + (j + 0.5) * cell
        for i in range(n):
            x = -25.4 + (i + 0.5) * cell
            if abs(x) <= aw and abs(y) <= ah and seen[j][i]:
                bad += 1
    deep = 0.0
    for j in range(n):
        y = -25.4 + (j + 0.5) * cell
        for i in range(n):
            x = -25.4 + (i + 0.5) * cell
            if seen[j][i] and abs(x) <= aw and abs(y) <= ah:
                deep = max(deep, min(aw - abs(x), ah - abs(y)) + cell / 2)
    print(f"  material inside the 36x24 aperture: {bad * area:.2f} mm2, "
          f"reaching {deep:.2f} mm in")
    # Thinnest place in the outer 2 mm of the outline.
    rim = [top[j][i] for j in range(n) for i in range(n)
           if seen[j][i] and max(abs(-25.4 + (i + .5) * cell),
                                 abs(-25.4 + (j + .5) * cell)) > 23.4]
    print(f"  outline band, thinnest {min(rim):.2f} mm, thickest {max(rim):.2f} mm")
    # Narrowest full-thickness margin anywhere around the outline.
    full = max(hist)
    worst = 99.0
    for j in range(n):
        y = -25.4 + (j + 0.5) * cell
        for i in range(n):
            x = -25.4 + (i + 0.5) * cell
            if seen[j][i] and top[j][i] < full - 1e-6:
                worst = min(worst, 25.4 - max(abs(x), abs(y)))
    print(f"  narrowest full-thickness margin at the outline: {worst:.2f} mm")

    if out:
        mx = max(h for h in hist)
        rows = []
        for j in range(n - 1, -1, -1):
            row = []
            for i in range(n):
                if not seen[j][i]:
                    row.append((200, 40, 40))
                else:
                    v = int(40 + 215 * top[j][i] / mx)
                    row.append((v, v, v))
            rows.append(row)
        with open(out, "wb") as f:
            f.write(f"P6\n{n} {n}\n255\n".encode())
            for row in rows:
                for px in row:
                    f.write(bytes(px))
        print(f"  plan view written to {out} (red = open)")


if __name__ == "__main__":
    main()
