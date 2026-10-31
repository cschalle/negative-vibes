"""Read back a generated STL, check its bounds, and draw a plan view.

The plan view also draws where the blade will sit and where the aperture is,
so it is possible to see that the cutting edge really does cross the middle
of the open area rather than clipping a corner of it.
"""
import math
import struct
import sys
import numpy as np
from PIL import Image, ImageDraw

MOUNT, APER_W, APER_H = 50.8, 36.0, 24.0
BLADE_L, BLADE_W = 43.0, 22.0


def read_stl(p):
    with open(p, "rb") as f:
        f.read(80)
        n = struct.unpack("<I", f.read(4))[0]
        tris = []
        for _ in range(n):
            f.read(12)
            tris.append([struct.unpack("<3f", f.read(12)) for _ in range(3)])
            f.read(2)
    return np.array(tris)


def plan(path, theta, base, out):
    tris = read_stl(path)
    v = tris.reshape(-1, 3)
    print(f"{path}: {len(tris)} triangles")
    print("  bounds  x %+.2f..%+.2f   y %+.2f..%+.2f   z %+.2f..%+.2f"
          % (v[:,0].min(), v[:,0].max(), v[:,1].min(), v[:,1].max(),
             v[:,2].min(), v[:,2].max()))
    ok = (abs(v[:,0]).max() <= MOUNT/2 + 1e-3) and (abs(v[:,1]).max() <= MOUNT/2 + 1e-3)
    print("  fits the 50.8 mm slide outline: %s" % ("yes" if ok else "NO"))
    print("  overall thickness %.2f mm (slide mounts accept 1.3-3.2)" % v[:,2].max())

    S = 14
    off = MOUNT/2
    im = Image.new("RGB", (int(MOUNT*S), int(MOUNT*S)), "white")
    d = ImageDraw.Draw(im)
    def px(x, y): return ((x+off)*S, (off-y)*S)
    # printed material, by height band
    for t in tris:
        if t[:,2].max() <= base + 1e-6:
            continue
        d.polygon([px(p[0], p[1]) for p in t], fill=(120,150,190))
    for t in tris:
        if t[:,2].max() > base + 1e-6:
            continue
        d.polygon([px(p[0], p[1]) for p in t], outline=(180,180,180))
    # aperture
    d.rectangle([px(-APER_W/2, APER_H/2), px(APER_W/2, -APER_H/2)],
                outline=(200,60,60), width=3)
    # where the blade lands, and its cutting edge
    c, s = math.cos(math.radians(theta)), math.sin(math.radians(theta))
    def tf(u, vv): return (u*c + vv*(-s), u*s + vv*c)
    corners = [tf(u, vv) for u, vv in ((0,-BLADE_L/2), (0,BLADE_L/2),
                                       (-BLADE_W,BLADE_L/2), (-BLADE_W,-BLADE_L/2))]
    d.polygon([px(*p) for p in corners], outline=(40,40,40), width=2)
    e0, e1 = tf(0, -BLADE_L/2), tf(0, BLADE_L/2)
    d.line([px(*e0), px(*e1)], fill=(220,120,0), width=5)
    im.save(out)
    print("  plan view -> %s  (red = aperture, orange = cutting edge)" % out)


plan(sys.argv[1], float(sys.argv[2]), float(sys.argv[3]), sys.argv[4])
