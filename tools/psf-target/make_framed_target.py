#!/usr/bin/env python3
"""Generate STLs for a slanted-edge PSF target with a full-thickness frame.

This supersedes make_target.py when the blade has to sit low.  The two are
the same idea -- a printed carrier whose only job is to hold a strip whose
own factory edge does the optics -- and make_target.py's module docstring
still explains why nothing printed is used as an optical edge.  What changes
here is where the blade sits and what holds the part still.

Why a frame
-----------
make_target.py makes the base plate `base` mm thick and stands the pocket
ridges on top, so the part is exactly `base + 1.0` mm overall.  Measured on
the three printed heights at 3600 dpi, over the field they share:

    base   10-90% rise across the field       best   mean
    0.7    7.34 .. 3.78 px                    3.78   5.43
    1.0    7.22 .. 5.83 px                    5.83   6.48
    1.3    6.43 .. 8.05 px                    6.43   7.08

Two things follow.  Blur falls as the blade goes down and is falling faster
at 0.7 than at 1.0 (3.49 px/mm against 2.02), so the focal plane is below
0.7 mm and the sweep has to continue.  And the gradient across the field is
different for every part -- the 0.7 falls 3.56 px, the 1.0 falls 1.39, the
1.3 *rises* 1.62 -- on one holder with one set of blades.  Each part is
sitting at its own tilt, which is what a 1.7 to 2.3 mm part does in a slot
that takes 1.3 to 3.2 mm.

So the plate cannot simply be made thinner: that loses grip just as the
blade reaches the interesting heights.  Here the part is a solid block of
`--thick` mm with the pocket sunk into it as a trench, which decouples the
two.  The blade goes where the optics want it and the outline stays the
thickness the holder wants.

The trench is an island, not a channel: a 43 mm blade in a 50.8 mm mount
leaves about 2 mm of material beyond each end and 3 mm behind it, so the
frame is unbroken at full thickness all the way round.

Why the aperture is flared
--------------------------
A 36 x 24 aperture through a 2.6 mm block is a 2.6 mm deep light tunnel,
and the earlier parts only had `base` mm of it.  Above the blade's plane the
hole therefore opens outward at 45 degrees, after a small ledge, so nothing
beyond the blade shades the illumination.  A hole that widens upward is
additive: every layer rests on one that reaches further in, so there is no
overhang and no support.  Below the blade's plane the hole stays at 36 x 24,
which is only `base` mm deep.

A printed edge instead of a blade
---------------------------------
--printed makes a part with no pocket at all: the occluder is printed, as a
plate lying on the build plate, and its own outline is the optical edge.
Whether that is good enough is a question the scan answers rather than this
file.  The aperture wall of the first printed targets measured a median
10-90% rise of 3.99 px with a p10-p90 of 3.74 to 16.62 px, so the typical
row was fine and the rough ones were very rough -- but that wall is 0.7 mm
of stacked layers, and this plate is one or two layers laid against the
build plate, which is the flattest and crispest surface an FDM printer
makes.

Two things follow from where it has to sit.  It cannot float over the
aperture, so it lies on the plate and its edge is at z = 0 to --printed mm,
which is below every height the blade carriers reach: useful, since the
blade measurements say the focal plane is below 0.7 mm.  And it is a wall
rather than a sheet, so every depth through its thickness contributes its
own defocused edge at the same place, which broadens the measurement.  Keep
--printed to one or two layers for that reason; it is the thickness, not the
roughness, that sets the floor here.

Marking
-------
Each part carries its own blade height engraved into the top face, with the
orientation letter under it, because the heights differ by a few tenths of a
millimetre and are otherwise impossible to tell apart by eye.

Geometry is emitted as a height field, decomposed into boxes.  Boxes abut
rather than interpenetrate, every box is watertight on its own, and the
result is the union a slicer expects -- the same approach, and the same
flat-bottomed additive result, as make_target.py's prisms.
"""
import argparse
import math
import struct

MOUNT = 50.8           # slide mount outline, mm
APER_W, APER_H = 36.0, 24.0
STRIP_L = 44.0         # pocket length along the edge; a blade is 43 mm
STRIP_W = 22.0         # nominal strip width -- a double-edge blade
WALL = 1.6             # material left between the trench and the outline
CLEAR = 0.35           # pocket clearance per side
LEDGE = 0.6            # flat ledge at the aperture rim before the flare
OFFSET = 1.8           # how far the blade's edge sits off the mount centre.
                       # The pocket is rotated, so its far corners reach
                       # further out than its width: at tilt 5 degrees an
                       # unshifted pocket comes within 1.1 mm of the outline
                       # and the frame there would be a sliver.  Shifting the
                       # whole pocket toward the fence side buys that back
                       # (2.9 mm at 1.8), and costs only a little of the open
                       # half of the aperture, which needs a fraction of a
                       # millimetre beyond the edge.
CELL = 0.2             # height-field cell, mm
QUANT = 0.05           # height quantisation, mm
LAYER = 0.2            # the flare is stepped at the layer height, because
                       # that is what a printer will make of it either way,
                       # and a smooth ramp would only cost triangles

FONT = {
    "0": ["01110", "10001", "10011", "10101", "11001", "10001", "01110"],
    "1": ["00100", "01100", "00100", "00100", "00100", "00100", "01110"],
    "2": ["01110", "10001", "00001", "00010", "00100", "01000", "11111"],
    "3": ["11111", "00010", "00100", "00010", "00001", "10001", "01110"],
    "4": ["00010", "00110", "01010", "10010", "11111", "00010", "00010"],
    "5": ["11111", "10000", "11110", "00001", "00001", "10001", "01110"],
    "6": ["00110", "01000", "10000", "11110", "10001", "10001", "01110"],
    "7": ["11111", "00001", "00010", "00100", "01000", "01000", "01000"],
    "8": ["01110", "10001", "10001", "01110", "10001", "10001", "01110"],
    "9": ["01110", "10001", "10001", "01111", "00001", "00010", "01100"],
    ".": ["00000", "00000", "00000", "00000", "00000", "01100", "01100"],
    "H": ["10001", "10001", "10001", "11111", "10001", "10001", "10001"],
    "P": ["11110", "10001", "10001", "11110", "10000", "10000", "10000"],
    "V": ["10001", "10001", "10001", "10001", "10001", "01010", "00100"],
}


def glyph_cells(text, cx, cy, cell):
    """World-space rectangles covering the lit pixels of `text`, centred."""
    adv = 6 * cell
    w = len(text) * adv - cell
    x0 = cx - w / 2
    y0 = cy - 7 * cell / 2
    out = []
    for k, ch in enumerate(text):
        rows = FONT.get(ch)
        if rows is None:
            continue
        for r, row in enumerate(rows):
            for c, bit in enumerate(row):
                if bit == "1":
                    x = x0 + k * adv + c * cell
                    y = y0 + (6 - r) * cell
                    out.append((x, x + cell, y, y + cell))
    return out


def outside_rect(x, y, hw, hh):
    """How far (x, y) lies outside the rectangle; 0 on or inside it."""
    return math.hypot(max(0.0, abs(x) - hw), max(0.0, abs(y) - hh))


def flare(x, y, floor, thick):
    """Height of the 45-degree flare that opens the aperture upward."""
    d = outside_rect(x, y, APER_W / 2, APER_H / 2)
    rise = math.floor(max(0.0, d - LEDGE) / LAYER) * LAYER
    return min(thick, floor + rise)


def printed_field(theta, occ, thick, text, engrave):
    """Top height per cell for a part whose occluder is printed, not held."""
    n = int(round(MOUNT / CELL))
    half = MOUNT / 2
    aw, ah = APER_W / 2, APER_H / 2
    c, s = math.cos(math.radians(theta)), math.sin(math.radians(theta))

    marks = []
    for line, cy in text:
        marks += glyph_cells(line, 18.5, cy, 0.36)

    field = []
    for j in range(n):
        y = -half + (j + 0.5) * CELL
        row = []
        for i in range(n):
            x = -half + (i + 0.5) * CELL
            u = x * c + y * s
            if abs(x) <= aw and abs(y) <= ah:
                row.append(occ if u <= OFFSET else None)
                continue
            h = flare(x, y, occ, thick)
            if h >= thick - 1e-9:
                for x0, x1, y0, y1 in marks:
                    if x0 <= x < x1 and y0 <= y < y1:
                        h = thick - engrave
                        break
            row.append(round(h / QUANT) * QUANT)
        field.append(row)
    return field, n


def height_field(theta, base, thick, strip_w, strip_l, text, engrave):
    """Top height per cell, or None where the part is open."""
    n = int(round(MOUNT / CELL))
    half = MOUNT / 2
    aw, ah = APER_W / 2, APER_H / 2
    c, s = math.cos(math.radians(theta)), math.sin(math.radians(theta))
    pv = strip_l / 2 + CLEAR
    front = OFFSET
    back = front - (strip_w + CLEAR)
    # The pocket is a 22 x 45 mm rectangle rotated inside a 50.8 mm square,
    # so its corners reach further than its sides and it does not sit
    # centred.  Slide it along its own length until its bounding box is
    # centred on the longer axis, which is the one with no room to spare.
    corners = [(u * c - v * s, u * s + v * c)
               for u in (back, front) for v in (-pv, pv)]
    xs = [q[0] for q in corners]
    ys = [q[1] for q in corners]
    if max(xs) - min(xs) >= max(ys) - min(ys):
        shift = (max(xs) + min(xs)) / 2 / s if abs(s) > 1e-6 else 0.0
    else:
        shift = -(max(ys) + min(ys)) / 2 / c if abs(c) > 1e-6 else 0.0
    # The fence has to stop short of the aperture, or it would occlude the
    # very edge it registers.  Which half-width that is depends on whether
    # the blade runs across the frame or down it.
    keep = (ah if abs(c) > 0.5 else aw) + 1.0

    marks = []
    for line, cy in text:
        marks += glyph_cells(line, 18.5, cy, 0.36)

    field = []
    for j in range(n):
        y = -half + (j + 0.5) * CELL
        row = []
        for i in range(n):
            x = -half + (i + 0.5) * CELL
            if abs(x) <= aw and abs(y) <= ah:
                row.append(None)
                continue
            # Blade frame: u across the strip from its optical edge at u = 0,
            # v along it.
            u = x * c + y * s
            v = -x * s + y * c
            v -= shift
            in_pocket = (back <= u <= front) and abs(v) <= pv
            near_pocket = (back - WALL <= u <= front + WALL) and abs(v) <= pv + WALL
            if in_pocket:
                h = base
            else:
                h = thick if near_pocket else flare(x, y, base, thick)
                # The fence is the trench's own u = 0 wall, so it only exists
                # where the pocket is not open to the aperture.
                if front < u <= front + WALL and abs(v) < keep:
                    h = min(h, flare(x, y, base, thick))
            if h >= thick - 1e-9:
                for x0, x1, y0, y1 in marks:
                    if x0 <= x < x1 and y0 <= y < y1:
                        h = thick - engrave
                        break
            row.append(round(h / QUANT) * QUANT)
        field.append(row)
    return field, n


def boxes(field, n):
    """Greedy maximal rectangles of equal height, as (x0, x1, y0, y1, h)."""
    half = MOUNT / 2
    seen = [[False] * n for _ in range(n)]
    out = []
    for j in range(n):
        for i in range(n):
            if seen[j][i] or field[j][i] is None:
                continue
            h = field[j][i]
            i1 = i
            while i1 + 1 < n and not seen[j][i1 + 1] and field[j][i1 + 1] == h:
                i1 += 1
            j1 = j
            while j1 + 1 < n:
                row = field[j1 + 1]
                if any(seen[j1 + 1][k] or row[k] != h for k in range(i, i1 + 1)):
                    break
                j1 += 1
            for jj in range(j, j1 + 1):
                for ii in range(i, i1 + 1):
                    seen[jj][ii] = True
            out.append((-half + i * CELL, -half + (i1 + 1) * CELL,
                        -half + j * CELL, -half + (j1 + 1) * CELL, h))
    return out


def box_tris(x0, x1, y0, y1, z1):
    p = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    t = [((x0, y0, z1), (x1, y0, z1), (x1, y1, z1)),
         ((x0, y0, z1), (x1, y1, z1), (x0, y1, z1)),
         ((x0, y0, 0.0), (x1, y1, 0.0), (x1, y0, 0.0)),
         ((x0, y0, 0.0), (x0, y1, 0.0), (x1, y1, 0.0))]
    for k in range(4):
        (ax, ay), (bx, by) = p[k], p[(k + 1) % 4]
        t += [((ax, ay, 0.0), (bx, by, 0.0), (bx, by, z1)),
              ((ax, ay, 0.0), (bx, by, z1), (ax, ay, z1))]
    return t


def write_stl(path, tris, name):
    with open(path, "wb") as f:
        f.write(name.encode()[:80].ljust(80, b"\0"))
        f.write(struct.pack("<I", len(tris)))
        for a, b, c in tris:
            ux, uy, uz = (b[i] - a[i] for i in range(3))
            vx, vy, vz = (c[i] - a[i] for i in range(3))
            nx, ny, nz = uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx
            m = math.sqrt(nx * nx + ny * ny + nz * nz) or 1.0
            f.write(struct.pack("<3f", nx / m, ny / m, nz / m))
            for q in (a, b, c):
                f.write(struct.pack("<3f", *q))
            f.write(struct.pack("<H", 0))


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--tilt", type=float, default=5.0,
                    help="degrees off axis; 2-15 is usable, 5 is comfortable")
    ap.add_argument("--base", type=float, action="append",
                    help="blade height above the mount's bottom face, mm; repeatable")
    ap.add_argument("--thick", type=float, default=2.6,
                    help="outline thickness, mm; the holder takes 1.3 to 3.2")
    ap.add_argument("--engrave", type=float, default=0.4,
                    help="depth of the engraved markings, mm")
    ap.add_argument("--strip-width", type=float, default=STRIP_W)
    ap.add_argument("--strip-length", type=float, default=STRIP_L)
    ap.add_argument("--printed", type=float, nargs="?", const=0.4, default=None,
                    help="no pocket: print the occluder, this many mm thick "
                         "(default 0.4, two layers). --base is then ignored")
    ap.add_argument("--only", choices=("h", "v"), help="emit one orientation")
    ap.add_argument("--out", default=".")
    a = ap.parse_args()
    if a.printed is not None:
        for suffix, theta, what in (("v", a.tilt, "near-vertical edge, sensor axis"),
                                    ("h", 90.0 + a.tilt, "near-horizontal edge, feed axis")):
            if a.only and a.only != suffix:
                continue
            text = [(f"{a.printed:.2f}", 21.0), ("P" + suffix.upper(), 17.0)]
            field, n = printed_field(theta, a.printed, a.thick, text, a.engrave)
            tris = []
            for bx in boxes(field, n):
                tris += box_tris(*bx)
            p = f"{a.out}/psf-printed-{suffix}-{a.printed:.2f}.stl"
            write_stl(p, tris, f"PSF printed {suffix} tilt {a.tilt} edge {a.printed}")
            print(f"{p}: {len(tris)} triangles, {what}, printed edge "
                  f"{a.printed:g} mm thick on the build plate, outline {a.thick:g} mm, "
                  f"engraved {a.printed:.2f}P{suffix.upper()}")
        return

    bases = a.base or [0.20, 0.35, 0.50, 0.65]

    for base in bases:
        if base + 0.6 > a.thick:
            raise SystemExit(f"--base {base} leaves no frame above it at "
                             f"--thick {a.thick}")
        for suffix, theta, what in (("v", a.tilt, "near-vertical edge, sensor axis"),
                                    ("h", 90.0 + a.tilt, "near-horizontal edge, feed axis")):
            if a.only and a.only != suffix:
                continue
            text = [(f"{base:.2f}", 21.0), (suffix.upper(), 17.0)]
            field, n = height_field(theta, base, a.thick, a.strip_width,
                                    a.strip_length, text, a.engrave)
            tris = []
            for bx in boxes(field, n):
                tris += box_tris(*bx)
            p = f"{a.out}/psf-framed-{suffix}-base{base:.2f}.stl"
            write_stl(p, tris, f"PSF framed {suffix} tilt {a.tilt} base {base}")
            print(f"{p}: {len(tris)} triangles, {what}, blade at {base:g} mm, "
                  f"outline {a.thick:g} mm, engraved {base:.2f}{suffix.upper()}")


if __name__ == "__main__":
    main()
