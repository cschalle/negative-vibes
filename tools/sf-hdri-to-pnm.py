#!/usr/bin/env python3
"""sf-hdri-to-pnm.py -- SilverFast HDR/HDRi RAW into the raw pair we read.

For the comparison of DESIGN.md §11.5.  SilverFast SE Plus can export an
unconverted scan ("48 Bit HDR RAW", or "64 Bit HDRi RAW" with the infrared
plane), which is the one way to put *the same pixels* through both
converters and compare conversion alone, with the scanner out of the
question.  It writes them as a multi-page TIFF; nv-store reads a 16-bit PNM
with an optional `-ir.pgm` beside it, and merges the pair into one
four-channel frame.  This is that conversion and nothing else: no scaling,
no byte juggling beyond PNM's big-endian order.

  sf-hdri-to-pnm.py IN.tif OUT.pnm      (writes OUT-ir.pgm too, if present)

Then: nv-cli render OUT.pnm -o rendered.ppm

Requires numpy and tifffile.
"""
import sys

import numpy as np
import tifffile


def write_pnm(path, a):
    """16-bit binary PNM.  P6 for three channels, P5 for one.  PNM is
    big-endian by specification, which is also what nv-store expects."""
    h, w = a.shape[:2]
    ch = 1 if a.ndim == 2 else a.shape[2]
    magic = b"P5" if ch == 1 else b"P6"
    with open(path, "wb") as f:
        f.write(b"%s\n%d %d\n65535\n" % (magic, w, h))
        f.write(a.astype(">u2").tobytes())


def main():
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    src, dst = sys.argv[1], sys.argv[2]
    if not dst.endswith(".pnm"):
        raise SystemExit("output must end in .pnm (the IR partner is named from its stem)")

    with tifffile.TiffFile(src) as t:
        pages = [p.asarray() for p in t.pages]
        soft = t.pages[0].tags.get("Software")
        print(f"{src}: {len(pages)} page(s), written by {soft.value if soft else 'unknown'}")

    rgb = pages[0]
    if rgb.ndim != 3 or rgb.shape[2] < 3:
        raise SystemExit(f"first page is {rgb.shape}, expected RGB")
    if rgb.dtype != np.uint16:
        raise SystemExit(f"first page is {rgb.dtype}, expected uint16 (use a 48/64-bit RAW export)")
    write_pnm(dst, rgb[:, :, :3])
    print(f"  RGB  {rgb.shape[1]}x{rgb.shape[0]} -> {dst}")

    # The HDRi exports carry the infrared plane as a second page; the plain
    # HDR ones do not, and then there is simply no partner to write.
    ir = next((p for p in pages[1:] if p.ndim == 2 and p.shape == rgb.shape[:2]), None)
    if ir is None:
        print("  no infrared page (a plain HDR RAW); dust removal will have nothing to use")
        return
    ir_path = dst[:-4] + "-ir.pgm"
    write_pnm(ir_path, ir)
    print(f"  IR   {ir.shape[1]}x{ir.shape[0]} -> {ir_path}")


if __name__ == "__main__":
    main()
