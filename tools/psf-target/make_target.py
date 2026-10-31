#!/usr/bin/env python3
"""Generate STLs for a slanted-edge PSF target that fits a 35 mm slide holder.

Why a carrier rather than a printed edge
----------------------------------------
The quantity being measured is a ~2 px transition at 3600 dpi, about 14 um.
For the target not to dominate the measurement its edge has to be straight to
well under that at spatial periods shorter than about a millimetre -- slower
wander than that the edge tracker follows out, roughness faster than it does
not.  FDM leaves roughly 100 um of edge roughness, which is 14 px: seven
times the feature.  So nothing printed is used as an optical edge.

Instead the print is a carrier for a strip of opaque material whose own
factory edge does the optics.  In descending order of how good that edge is:

  black developed film   0.14 mm, slit to about +-10 um because it has to
                         run through a camera; opaque at D > 3.  Best.
  double-edge razor      0.10 mm, ground straight to under a micron, but
                         needs a blade to hand.
  black silicone tape    ~0.5 mm, slit but soft, and the edge tends to be
                         rounded rather than square.  Worth trying, and the
                         scan says immediately whether it was good enough.

Every scan also contains the holder's own mask edge, which measures about
3 px.  If the strip's edge comes back much softer than that, the strip is
the limit and not the scanner.

Registration
------------
The strip is located by its *optical* edge butting against a fence, not by
its width against a back wall, so any strip from about 20 to 25 mm wide
works without changing the part.  The fence only exists outside the aperture,
where it cannot occlude anything that gets measured; across the open area the
edge is free.

Geometry
--------
Outline follows a 35 mm slide mount, 50.8 mm square, with a 36 x 24 mm
aperture.  The strip lies on the base plate, so its underside sits at `base`
mm above the mount's bottom face; print the variants to find which height the
scanner actually focuses on.  Ridges around it form an open pocket: all
additive, no overhangs, nothing thinner than the nozzle.

The strip is tilted off-axis by `tilt` degrees.  This is not cosmetic.  A
rigid edge aligned with the pixel grid gives every scan line the same
sub-pixel phase, so there is nothing to interleave and the measurement
collapses.  Film gets away with it only because it bows.

Two parts are emitted: `-v` with the edge near-vertical, which measures the
sensor axis, and `-h` with it near-horizontal, which measures the feed axis.
"""
import argparse
import math
import struct

MOUNT = 50.8          # slide mount outline, mm
APER_W, APER_H = 36.0, 24.0
STRIP_L = 46.0        # pocket length along the edge; cut the strip to ~45 mm
STRIP_W = 22.0        # nominal strip width; the back wall sits behind this.
                      # 22 mm is a double-edge razor blade, which is the
                      # widest thing the pocket is meant to take.
WALL = 1.6            # ridge thickness; 4 passes of a 0.4 mm nozzle
CLEAR = 0.35          # pocket clearance per side
RIDGE = 1.0           # ridge height above the base plate


def quad_prism(pts, z0, z1):
    """Triangles for a convex quad extruded between two heights."""
    (ax, ay), (bx, by), (cx, cy), (dx, dy) = pts
    top = [((ax, ay, z1), (bx, by, z1), (cx, cy, z1)),
           ((ax, ay, z1), (cx, cy, z1), (dx, dy, z1))]
    bot = [((ax, ay, z0), (cx, cy, z0), (bx, by, z0)),
           ((ax, ay, z0), (dx, dy, z0), (cx, cy, z0))]
    side = []
    loop = [(ax, ay), (bx, by), (cx, cy), (dx, dy)]
    for i in range(4):
        (x0, y0), (x1, y1) = loop[i], loop[(i + 1) % 4]
        side += [((x0, y0, z0), (x1, y1, z0), (x1, y1, z1)),
                 ((x0, y0, z0), (x1, y1, z1), (x0, y0, z1))]
    return top + bot + side


def rect(x0, x1, y0, y1, z0, z1):
    return quad_prism([(x0, y0), (x1, y0), (x1, y1), (x0, y1)], z0, z1)


def bar(theta, u0, u1, v0, v1, z0, z1, half=MOUNT / 2):
    """A rectangle in the blade's own frame, rotated into the part frame.

    u runs across the blade from its cutting edge at u = 0 into the body at
    negative u; v runs along the edge.  Corners are clamped to the outline so
    a tilted pocket cannot push material off the plate.
    """
    c, s = math.cos(math.radians(theta)), math.sin(math.radians(theta))
    pts = []
    for u, v in ((u0, v0), (u1, v0), (u1, v1), (u0, v1)):
        x = u * c + v * (-s)
        y = u * s + v * c
        pts.append((max(-half, min(half, x)), max(-half, min(half, y))))
    return quad_prism(pts, z0, z1)


def build(theta, base, strip_w, strip_l):
    """One carrier: base plate with an aperture, plus the strip pocket."""
    h = MOUNT / 2
    aw, ah = APER_W / 2, APER_H / 2
    tris = []
    # Base plate as four bands around the aperture -- no holes to triangulate.
    tris += rect(-h, -aw, -h, h, 0, base)
    tris += rect(aw, h, -h, h, 0, base)
    tris += rect(-aw, aw, -h, -ah, 0, base)
    tris += rect(-aw, aw, ah, h, 0, base)

    z0, z1 = base, base + RIDGE
    pv = strip_l / 2 + CLEAR
    back = -(strip_w + CLEAR)

    # The fence: what the optical edge is pushed against, so the edge lands at
    # u = 0 whatever the strip's width.  Kept clear of the aperture, which is
    # why it comes in two pieces rather than one bar.
    keep = (APER_H if abs(math.cos(math.radians(theta))) > 0.5 else APER_W) / 2 + 1.0
    tris += bar(theta, 0.0, WALL, keep, pv, z0, z1)
    tris += bar(theta, 0.0, WALL, -pv, -keep, z0, z1)
    # Back wall: stops the strip drifting away from the fence.
    tris += bar(theta, back - WALL, back, -pv - WALL, pv + WALL, z0, z1)
    # Ends: keep it from sliding along its own edge.
    tris += bar(theta, back - WALL, WALL, pv, pv + WALL, z0, z1)
    tris += bar(theta, back - WALL, WALL, -pv - WALL, -pv, z0, z1)
    return tris


def write_stl(path, tris, name):
    with open(path, "wb") as f:
        f.write(name.encode()[:80].ljust(80, b"\0"))
        f.write(struct.pack("<I", len(tris)))
        for a, b, c in tris:
            ux, uy, uz = (b[i] - a[i] for i in range(3))
            vx, vy, vz = (c[i] - a[i] for i in range(3))
            nx, ny, nz = uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx
            n = math.sqrt(nx * nx + ny * ny + nz * nz) or 1.0
            f.write(struct.pack("<3f", nx / n, ny / n, nz / n))
            for p in (a, b, c):
                f.write(struct.pack("<3f", *p))
            f.write(struct.pack("<H", 0))


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--tilt", type=float, default=5.0,
                    help="degrees off axis; 2-15 is usable, 5 is comfortable")
    ap.add_argument("--base", type=float, default=1.0,
                    help="blade height above the mount's bottom face, mm")
    ap.add_argument("--strip-width", type=float, default=STRIP_W,
                    help="nominal strip width in mm; 20-25 all fit")
    ap.add_argument("--strip-length", type=float, default=STRIP_L,
                    help="pocket length along the edge, mm")
    ap.add_argument("--out", default=".")
    a = ap.parse_args()
    # theta rotates the blade's across-axis, so the edge itself runs along
    # (-sin theta, cos theta): theta near 0 gives a near-vertical edge.
    for suffix, theta, what in (("v", a.tilt, "near-vertical edge, sensor axis"),
                                ("h", 90.0 + a.tilt, "near-horizontal edge, feed axis")):
        tris = build(theta, a.base, a.strip_width, a.strip_length)
        p = f"{a.out}/psf-target-{suffix}-base{a.base:g}.stl"
        write_stl(p, tris, f"PSF target {suffix} tilt {a.tilt} base {a.base}")
        print(f"{p}: {len(tris)} triangles, {what}, "
              f"edge {a.tilt:g} deg off axis, strip floor at {a.base:g} mm")


if __name__ == "__main__":
    main()
