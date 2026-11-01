#!/usr/bin/env python3
"""Turn the measured line spread functions into the table nv-process embeds.

Reads `psf-feed-3600.csv` and `psf-sensor-3600.csv` -- the edge and line
spread functions measured from a razor blade in a printed carrier, see
README.md -- and writes `crates/nv-process/src/deconv_psf.rs`.

What is stored is the *continuous* modulation transfer, not the transfer as any
particular grid samples it.  The difference matters.  A sampled picture carries
the response folded back over its own Nyquist, and these optics reach well past
Nyquist at 3600 dpi, so the folding is not negligible -- but which grid to fold
onto depends on the resolution and on how much a proxy has reduced the picture.
Folding here would bake one answer in; folding in the consumer keeps one table
honest for every grid.

The transfer runs to 2 cycles per reference pixel, four times Nyquist, because
the folding needs the response above Nyquist to fold.  Above about 1 cycle per
pixel there is nothing left: 0.07 on the feed axis, 0.02 on the sensor axis.

Each axis is taken from the sharpest fifth of its own target scan.  The two
parts sit at slightly different tilts, so comparing their whole fields would
compare two different amounts of defocus rather than two axes; their sharpest
fifths are both on the focal plane, which is the plane a correction is for.

    python3 gen_transfer.py            # writes the module
    python3 gen_transfer.py --check    # fails if the module is out of date
"""
import argparse
import pathlib
import sys

import numpy as np

HERE = pathlib.Path(__file__).resolve().parent
OUT = HERE.parent.parent / "crates" / "nv-process" / "src" / "deconv_psf.rs"
SAMPLES = 161
F_MAX = 2.0
REFERENCE_PX_PER_MM = 3600.0 / 25.4
# Which column of which file is each axis's line spread function.  The feed
# file carries both a whole-field and a sharpest-fifth measurement; the sensor
# file carries only the sharpest third, which is the same kind of thing.
AXES = (("FEED", "psf-feed-3600.csv", "lsf_best"),
        ("SENSOR", "psf-sensor-3600.csv", "lsf"))


def read_lsf(name, column):
    """The line spread function from one CSV, as (pixel offsets, values)."""
    lines = [l for l in (HERE / name).read_text().splitlines() if not l.startswith("#")]
    header = lines[0].split(",")
    col = header.index(column)
    rows = np.array([[float(v) for v in l.split(",")] for l in lines[1:]])
    return rows[:, 0], rows[:, col]


def transfer(px, lsf):
    """The continuous modulation transfer, normalised to 1 at zero frequency.

    The spread function is clipped at zero first.  Its tails are measured at
    the level of a few parts in a thousand and go slightly negative where the
    noise does; left in, those excursions are a signal at the top of the band,
    which is exactly where a deconvolution leans hardest.
    """
    l = np.clip(lsf, 0.0, None)
    l = l / l.sum()
    mag = np.abs(np.fft.rfft(np.fft.ifftshift(l)))
    mag /= mag[0]
    freq = np.fft.rfftfreq(len(l), px[1] - px[0])
    want = np.linspace(0.0, F_MAX, SAMPLES)
    return np.interp(want, freq, mag, right=0.0)


def render():
    tables = {name: transfer(*read_lsf(f, c)) for name, f, c in AXES}
    out = [
        "//! Generated from `tools/psf-target/psf-feed-3600.csv` and",
        "//! `psf-sensor-3600.csv` by `tools/psf-target/gen_transfer.py`. Do not edit.",
        "//!",
        f"//! The scanner's modulation transfer along each axis, before any sampling",
        f"//! grid is imposed on it: {SAMPLES} points from 0 to {F_MAX:g} cycles per pixel of a",
        "//! 3600 dpi scan, normalised to 1 at zero frequency. It runs past Nyquist",
        "//! because the optics do, and because folding it back over whatever grid a",
        "//! picture is actually sampled on is the caller's job -- doing it here would",
        "//! bake in one resolution.",
        "//!",
        "//! Each axis comes from the sharpest fifth of its own target scan, so the two",
        "//! describe the same focal plane rather than two different tilts.",
        "",
        "/// Pixels per millimetre the transfer was measured at: 3600 dpi.",
        f"pub const REFERENCE_PX_PER_MM: f32 = {REFERENCE_PX_PER_MM:.3f};",
        "",
        "/// Highest frequency in the tables, in cycles per reference pixel.",
        f"pub const F_MAX: f32 = {F_MAX:.1f};",
        "",
        "/// Samples in each table, from 0 to [`F_MAX`] inclusive.",
        f"pub const SAMPLES: usize = {SAMPLES};",
        "",
    ]
    for name, _, _ in AXES:
        out.append(f"pub const {name}: [f32; SAMPLES] = [")
        t = tables[name]
        for i in range(0, SAMPLES, 6):
            out.append("    " + " ".join(f"{v:.6}," for v in t[i:i + 6]))
        out.append("];")
        out.append("")
    return "\n".join(out), tables


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--check", action="store_true",
                    help="exit non-zero if the checked-in module differs")
    a = ap.parse_args()
    text, tables = render()
    if a.check:
        if not OUT.exists() or OUT.read_text() != text:
            print(f"{OUT} is out of date; run gen_transfer.py", file=sys.stderr)
            return 1
        print(f"{OUT.name} matches the measurements")
        return 0
    OUT.write_text(text)
    print(f"wrote {OUT}")
    f = np.linspace(0.0, F_MAX, SAMPLES)
    for name, _, _ in AXES:
        t = tables[name]
        got = " ".join(f"{q}:{np.interp(q / REFERENCE_PX_PER_MM, f, t):.3f}"
                       for q in (10, 20, 30, 40, 50, 60, 70))
        print(f"  {name:6} cyc/mm {got}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
