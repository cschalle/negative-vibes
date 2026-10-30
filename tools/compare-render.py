#!/usr/bin/env python3
"""compare-render.py -- quality measurements for comparing two renderings.

scan-check.py answers "is this the same scan?".  This answers "is this the
same picture, and which one is better?", which is what the SilverFast
comparison of DESIGN.md §11.5 asks for and nothing here computed.

  de       REF FILE              CIE deltaE 2000 between two renderings
  levels   FILE                  percentiles, clipping, distinct levels
  noise    FILE --roi X,Y,W,H    per-channel noise and SNR in a flat patch
  mtf      FILE --roi X,Y,W,H    slanted-edge MTF50 (needs a 2-15 deg edge)
  spectrum REF FILE              which of two scans resolves more detail
  defects  BEFORE AFTER          what a dust-removal pass changed
  selftest                       check the colour maths against known values

Files: PNM or TIFF, 8 or 16 bit, as scan-check.py loads them.

`de` compares colour, so both files must be renderings in the same output
space (sRGB unless --space says otherwise), not raw scans.  Two converters
rarely agree on overall exposure, and a flat offset would swamp every other
difference, so --align says what to equalise first:

  none      compare as exported -- includes exposure and white balance
  exposure  match median luminance (the converters' tone decisions remain)
  neutral   match each channel's median (removes cast as well; what is left
            is local tone, saturation and detail rendering)

Requires numpy; `de --heatmap` also needs Pillow.
"""
import argparse
import importlib.util
import os
import sys

import numpy as np

# scan-check.py already reads every format we care about, and its loaders are
# the ones the rest of the testing uses -- so share them rather than keep a
# second set that can drift.  The hyphen in the name rules out a plain import.
_spec = importlib.util.spec_from_file_location(
    "scan_check", os.path.join(os.path.dirname(os.path.abspath(__file__)), "scan-check.py"))
_sc = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_sc)
load, gray, interior = _sc.load, _sc.gray, _sc.interior


# --------------------------------------------------------------------------
# Colour

# Both products export in a display space, so the numbers are gamma-encoded.
# Undo that before anything that needs linear light or a Lab conversion.
_PRIMARIES = {
    # name: (RGB->XYZ matrix rows, gamma)
    "srgb": (np.array([[0.4124564, 0.3575761, 0.1804375],
                       [0.2126729, 0.7151522, 0.0721750],
                       [0.0193339, 0.1191920, 0.9503041]]), None),
    "adobe-rgb": (np.array([[0.5767309, 0.1855540, 0.1881852],
                            [0.2973769, 0.6273491, 0.0752741],
                            [0.0270343, 0.0706872, 0.9911085]]), 563 / 256),
    "prophoto": (np.array([[0.7976749, 0.1351917, 0.0313534],
                           [0.2880402, 0.7118741, 0.0000857],
                           [0.0000000, 0.0000000, 0.8252100]]), 1.8),
}
_D65 = np.array([0.95047, 1.00000, 1.08883])
_D50 = np.array([0.96422, 1.00000, 0.82521])


def to_linear(rgb, space):
    """rgb in [0,1] -> linear light."""
    _, gamma = _PRIMARIES[space]
    if gamma is None:                       # sRGB's piecewise curve
        return np.where(rgb <= 0.04045, rgb / 12.92,
                        ((np.clip(rgb, 0, None) + 0.055) / 1.055) ** 2.4)
    return np.clip(rgb, 0, None) ** gamma


def to_lab(rgb, space="srgb"):
    """rgb in [0,1], gamma-encoded -> L*a*b*."""
    m, _ = _PRIMARIES[space]
    white = _D50 if space == "prophoto" else _D65
    xyz = to_linear(rgb, space) @ m.T / white

    e, k = 216 / 24389, 24389 / 27
    f = np.where(xyz > e, np.cbrt(np.clip(xyz, 0, None)), (k * xyz + 16) / 116)
    return np.stack([116 * f[..., 1] - 16,
                     500 * (f[..., 0] - f[..., 1]),
                     200 * (f[..., 1] - f[..., 2])], axis=-1)


def delta_e_2000(lab1, lab2):
    """CIEDE2000.  Follows Sharma et al. (2005), including the hue-mean
    wrap-around cases that the original CIE text leaves implicit."""
    L1, a1, b1 = lab1[..., 0], lab1[..., 1], lab1[..., 2]
    L2, a2, b2 = lab2[..., 0], lab2[..., 1], lab2[..., 2]

    C1, C2 = np.hypot(a1, b1), np.hypot(a2, b2)
    Cbar = (C1 + C2) / 2
    G = 0.5 * (1 - np.sqrt(Cbar ** 7 / (Cbar ** 7 + 25.0 ** 7 + 1e-30)))
    a1p, a2p = (1 + G) * a1, (1 + G) * a2
    C1p, C2p = np.hypot(a1p, b1), np.hypot(a2p, b2)

    h1p = np.degrees(np.arctan2(b1, a1p)) % 360
    h2p = np.degrees(np.arctan2(b2, a2p)) % 360

    dLp = L2 - L1
    dCp = C2p - C1p
    dhp = h2p - h1p
    dhp = np.where(dhp > 180, dhp - 360, np.where(dhp < -180, dhp + 360, dhp))
    dhp = np.where(C1p * C2p == 0, 0.0, dhp)
    dHp = 2 * np.sqrt(C1p * C2p) * np.sin(np.radians(dhp) / 2)

    Lbp = (L1 + L2) / 2
    Cbp = (C1p + C2p) / 2
    hsum, hdiff = h1p + h2p, np.abs(h1p - h2p)
    hbp = np.where(C1p * C2p == 0, hsum,
                   np.where(hdiff <= 180, hsum / 2,
                            np.where(hsum < 360, (hsum + 360) / 2, (hsum - 360) / 2)))

    T = (1 - 0.17 * np.cos(np.radians(hbp - 30))
         + 0.24 * np.cos(np.radians(2 * hbp))
         + 0.32 * np.cos(np.radians(3 * hbp + 6))
         - 0.20 * np.cos(np.radians(4 * hbp - 63)))
    Sl = 1 + 0.015 * (Lbp - 50) ** 2 / np.sqrt(20 + (Lbp - 50) ** 2)
    Sc = 1 + 0.045 * Cbp
    Sh = 1 + 0.015 * Cbp * T
    Rt = (-2 * np.sqrt(Cbp ** 7 / (Cbp ** 7 + 25.0 ** 7 + 1e-30))
          * np.sin(np.radians(60 * np.exp(-(((hbp - 275) / 25) ** 2)))))

    return np.sqrt((dLp / Sl) ** 2 + (dCp / Sc) ** 2 + (dHp / Sh) ** 2
                   + Rt * (dCp / Sc) * (dHp / Sh))


# --------------------------------------------------------------------------
# Helpers

def load_any(path):
    """scan-check.py's loader, plus PNG -- which is what tests/goldens holds
    and what both products will happily export."""
    if path.lower().endswith(".png"):
        from PIL import Image
        im = Image.open(path)
        a = np.asarray(im).astype(np.float64)
        if a.ndim == 2:
            a = a[:, :, None]
        return a, 65535 if a.dtype == np.uint16 or im.mode.endswith("16") else 255
    return load(path)


def unit(path):
    """Load as float RGB in [0,1], three channels."""
    a, mx = load_any(path)
    a = a / float(mx)
    if a.shape[2] == 1:
        a = np.repeat(a, 3, axis=2)
    return a[:, :, :3]


def parse_roi(s, shape):
    if not s:
        h, w = shape[:2]
        ys, xs = interior(h, w, 0.25)       # middle half, a safe default
        return ys, xs
    x, y, w, h = (int(v) for v in s.split(","))
    return slice(y, y + h), slice(x, x + w)


def align_to(ref, img, mode):
    """Scale img's channels so it can be compared with ref on rendering
    rather than on overall setup.  Returns the adjusted img."""
    if mode == "none":
        return img
    if mode == "exposure":
        r = np.median(gray(ref)) / (np.median(gray(img)) + 1e-12)
        return np.clip(img * r, 0, 1)
    out = img.copy()
    for c in range(3):
        r = np.median(ref[:, :, c]) / (np.median(img[:, :, c]) + 1e-12)
        out[:, :, c] = np.clip(img[:, :, c] * r, 0, 1)
    return out


# --------------------------------------------------------------------------
# Commands

def cmd_de(args):
    ref, img = unit(args.ref), unit(args.file)
    if ref.shape != img.shape:
        raise SystemExit(f"size mismatch: {ref.shape[1]}x{ref.shape[0]} "
                         f"vs {img.shape[1]}x{img.shape[0]}; downsample first "
                         f"(scan-check.py down)")
    img = align_to(ref, img, args.align)

    de = delta_e_2000(to_lab(ref, args.space), to_lab(img, args.space))
    ys, xs = parse_roi(args.roi, ref.shape)
    d = de[ys, xs]

    print(f"ref  {args.ref}")
    print(f"file {args.file}")
    print(f"align {args.align}, space {args.space}, "
          f"{d.shape[1]}x{d.shape[0]} px compared")
    p = np.percentile(d, [50, 90, 95, 99])
    print(f"  deltaE2000  mean {d.mean():6.2f}  median {p[0]:6.2f}  "
          f"p90 {p[1]:6.2f}  p95 {p[2]:6.2f}  p99 {p[3]:6.2f}  max {d.max():6.2f}")
    # The perceptual landmarks worth naming: 1.0 is a just-noticeable
    # difference for adjacent patches, 2-3 is visible side by side.
    for t in (1, 2, 3, 5, 10):
        print(f"  over dE {t:<3}   {100 * (d > t).mean():5.1f}% of pixels")

    lab_r, lab_i = to_lab(ref, args.space), to_lab(img, args.space)
    for i, name in enumerate(("L*", "a*", "b*")):
        dv = (lab_i[..., i] - lab_r[..., i])[ys, xs]
        print(f"  {name} bias {dv.mean():+6.2f}  spread {dv.std():5.2f}")

    if args.heatmap:
        from PIL import Image
        v = np.clip(d / max(args.heatmap_max, 1e-6), 0, 1)
        # Blue (match) through yellow to red (worst): readable in greyscale too.
        rgb = np.stack([np.clip(2 * v, 0, 1), np.clip(1.5 * v, 0, 1) ** 2,
                        np.clip(1 - 2 * v, 0, 1)], axis=-1)
        Image.fromarray((rgb * 255).astype(np.uint8)).save(args.heatmap)
        print(f"  heatmap -> {args.heatmap} (full scale = dE {args.heatmap_max})")


def cmd_levels(args):
    a, mx = load_any(args.file)
    h, w, ch = a.shape
    ys, xs = parse_roi(args.roi, a.shape)
    print(f"{args.file}: {w}x{h}, {ch} channel(s), max {mx}")
    for c in range(ch):
        v = a[ys, xs, c]
        p = np.percentile(v, [0.1, 1, 50, 99, 99.9])
        # Distinct levels is the honest measure of what survived the
        # pipeline: a 16-bit file carrying 200 values is an 8-bit file.
        n = len(np.unique(v.astype(np.int64)))
        print(f"  ch{c}  p0.1 {p[0]:8.1f}  p1 {p[1]:8.1f}  median {p[2]:8.1f}  "
              f"p99 {p[3]:8.1f}  p99.9 {p[4]:8.1f}")
        print(f"       black-clipped {100 * (v <= 0).mean():5.2f}%  "
              f"white-clipped {100 * (v >= mx).mean():5.2f}%  "
              f"distinct {n}  ({100 * n / (mx + 1):.1f}% of {mx + 1})")


def cmd_noise(args):
    a, mx = load_any(args.file)
    ys, xs = parse_roi(args.roi, a.shape)
    patch = a[ys, xs] / float(mx)
    print(f"{args.file}: patch {patch.shape[1]}x{patch.shape[0]}")
    for c in range(patch.shape[2]):
        v = patch[:, :, c]
        # Detrend: a real "flat" patch of film still has a gentle gradient,
        # and counting that as noise flatters whichever scan is less even.
        yy, xx = np.mgrid[0:v.shape[0], 0:v.shape[1]]
        A = np.column_stack([np.ones(v.size), xx.ravel(), yy.ravel()])
        resid = v.ravel() - A @ np.linalg.lstsq(A, v.ravel(), rcond=None)[0]
        sd = resid.std()
        print(f"  ch{c}  mean {v.mean():.4f}  noise {sd:.5f}  "
              f"SNR {20 * np.log10(v.mean() / (sd + 1e-12)):5.1f} dB  "
              f"({sd * (mx + 1):.1f} of {mx + 1} levels)")


def mtf50_of(roi):
    """Slanted-edge MTF50 of a 2-D patch, in cycles per pixel.
    Returns (mtf50, edge angle in degrees, rows used, ESF length)."""
    h, w = roi.shape
    if w < 8 or h < 8:
        raise SystemExit("ROI too small for a slanted edge")

    # Locate the edge on each row by the centroid of |gradient|, then fit a
    # line: the slant is what lets rows be interleaved into an oversampled
    # edge profile, so a perfectly vertical edge gives a useless answer.
    g = np.abs(np.diff(roi, axis=1))
    rows = np.arange(h)
    cent = (g * np.arange(w - 1)).sum(axis=1) / (g.sum(axis=1) + 1e-12)
    ok = g.sum(axis=1) > 0
    if ok.sum() < 8:
        raise SystemExit("no edge found in ROI")
    slope, icept = np.polyfit(rows[ok], cent[ok], 1)
    angle = np.degrees(np.arctan(slope))

    bins = 4                                 # 4x oversampled, the ISO 12233 default
    pos = (np.arange(w)[None, :] - (slope * rows + icept)[:, None]).ravel()
    val = roi.ravel()
    idx = np.round(pos * bins).astype(int)
    lo, hi = idx.min(), idx.max()
    esf = np.bincount(idx - lo, weights=val, minlength=hi - lo + 1)
    cnt = np.bincount(idx - lo, minlength=hi - lo + 1)
    keep = cnt > 0
    esf = esf[keep] / cnt[keep]

    lsf = np.diff(esf)
    lsf *= np.hanning(len(lsf))              # suppress the ends' ringing
    m = np.abs(np.fft.rfft(lsf))
    m /= m[0] + 1e-12
    freq = np.fft.rfftfreq(len(lsf), d=1.0 / bins)   # cycles per pixel

    below = np.where(m < 0.5)[0]
    if len(below) == 0:
        raise SystemExit("MTF never falls to 0.5 in this ROI")
    i = below[0]
    f0, f1, m0, m1 = freq[i - 1], freq[i], m[i - 1], m[i]
    return (f0 + (m0 - 0.5) * (f1 - f0) / (m0 - m1 + 1e-12),
            angle, int(ok.sum()), len(esf))


def cmd_mtf(args):
    a, mx = load_any(args.file)
    ys, xs = parse_roi(args.roi, a.shape)
    roi = gray(a[ys, xs]) / float(mx)
    if args.horizontal:
        roi = roi.T

    mtf50, angle, nrows, nesf = mtf50_of(roi)
    if abs(angle) < 1 or abs(angle) > 20:
        print(f"  warning: edge angle {angle:.1f} deg is outside the usable "
              f"2-15 deg; MTF50 below is unreliable")

    print(f"{args.file}: edge angle {angle:.2f} deg, "
          f"{nrows} rows, {nesf} ESF samples")
    print(f"  MTF50 {mtf50:.4f} cycles/pixel")
    if args.dpi:
        # lp/mm is the number photographers compare scanners on.
        print(f"  MTF50 {mtf50 * args.dpi / 25.4:.1f} lp/mm at {args.dpi} dpi")
        print(f"  effective resolution {200 * mtf50:.0f}% of the "
              f"{args.dpi} dpi sampling ({2 * mtf50 * args.dpi:.0f} dpi equivalent)")


def radial_psd(img):
    """Radially averaged power spectrum of a 2-D patch, indexed by spatial
    frequency in cycles per pixel (0 to 0.5)."""
    v = img - img.mean()
    # A hard crop has edges of its own, and they put energy at every
    # frequency; window before transforming or the answer is mostly the crop.
    v = v * np.hanning(v.shape[0])[:, None] * np.hanning(v.shape[1])[None, :]
    p = np.abs(np.fft.fftshift(np.fft.fft2(v))) ** 2

    h, w = p.shape
    yy, xx = np.mgrid[0:h, 0:w]
    fy = (yy - h / 2) / h
    fx = (xx - w / 2) / w
    r = np.hypot(fx, fy)

    nb = 64
    edges = np.linspace(0, 0.5, nb + 1)
    idx = np.clip(np.digitize(r.ravel(), edges) - 1, 0, nb - 1)
    tot = np.bincount(idx, weights=p.ravel(), minlength=nb)
    cnt = np.bincount(idx, minlength=nb)
    return (edges[:-1] + edges[1:]) / 2, tot / np.maximum(cnt, 1)


def cmd_spectrum(args):
    """Which of two renderings of the same frame resolves more detail.

    The film rebate on a well-aligned strip sits within a fraction of a
    degree of vertical, and the slanted-edge method needs 2-15, so `mtf` has
    nothing to bite on in an ordinary frame.  Comparing the two scans'
    power spectra needs no edge: on identical subject matter, the one
    carrying more energy at high frequency resolved more of it.
    """
    a = unit(args.ref)
    b = unit(args.file)
    if a.shape != b.shape:
        raise SystemExit(f"size mismatch: {a.shape[1]}x{a.shape[0]} vs "
                         f"{b.shape[1]}x{b.shape[0]}; downsample first")
    ys, xs = parse_roi(args.roi, a.shape)
    ga, gb = gray(a[ys, xs]), gray(b[ys, xs])

    # Equalise contrast first: a brighter or more contrasty rendering has
    # more energy at every frequency, which is a tone difference, not a
    # sharpness one.
    ga = (ga - ga.mean()) / (ga.std() + 1e-12)
    gb = (gb - gb.mean()) / (gb.std() + 1e-12)

    f, pa = radial_psd(ga)
    _, pb = radial_psd(gb)
    ratio = np.sqrt(pb / (pa + 1e-20))       # amplitude, not power

    print(f"ref  {args.ref}")
    print(f"file {args.file}")
    print(f"{ga.shape[1]}x{ga.shape[0]} px, contrast-normalised")
    print(f"  {'cyc/px':>8} {'lp/mm':>8}  {'ref':>10} {'file':>10}  ratio")
    for lo, hi in ((0.05, 0.10), (0.10, 0.20), (0.20, 0.30), (0.30, 0.40), (0.40, 0.50)):
        m = (f >= lo) & (f < hi)
        if not m.any():
            continue
        lp = f"{(lo + hi) / 2 * args.dpi / 25.4:8.1f}" if args.dpi else "       -"
        print(f"  {(lo + hi) / 2:8.3f} {lp}  {pa[m].mean():10.3e} "
              f"{pb[m].mean():10.3e}  {np.sqrt(pb[m].mean() / (pa[m].mean() + 1e-20)):.3f}")
    hi = f >= 0.25
    r = float(np.sqrt(pb[hi].mean() / (pa[hi].mean() + 1e-20)))
    print(f"  above 0.25 cyc/px: file carries {r:.3f}x the amplitude of ref "
          f"({'file' if r > 1.02 else 'ref' if r < 0.98 else 'neither'} is sharper)")


def cmd_defects(args):
    before, after = unit(args.before), unit(args.after)
    if before.shape != after.shape:
        raise SystemExit("before and after differ in size")
    d = np.abs(gray(after) - gray(before))
    touched = d > args.threshold

    print(f"before {args.before}")
    print(f"after  {args.after}")
    print(f"  pixels changed by more than {args.threshold}: "
          f"{100 * touched.mean():.3f}%  ({touched.sum()} px)")
    if touched.any():
        print(f"  mean change where touched {d[touched].mean():.4f}, "
              f"max {d.max():.4f}")

    # Dust removal should brighten specks on a negative and leave structure
    # alone.  Losing local contrast across the whole frame is the failure
    # mode worth naming: that is detail being smoothed, not dust being fixed.
    def detail(x):
        k = gray(x)
        return float(np.abs(np.diff(k, axis=0)).mean() + np.abs(np.diff(k, axis=1)).mean())

    db, da = detail(before), detail(after)
    print(f"  local detail {db:.5f} -> {da:.5f} ({100 * (da - db) / (db + 1e-12):+.2f}%)")
    clean = ~touched
    if clean.any():
        cb = np.abs(np.diff(gray(before), axis=1))
        ca = np.abs(np.diff(gray(after), axis=1))
        m = clean[:, :-1]
        print(f"  detail in untouched areas "
              f"{cb[m].mean():.5f} -> {ca[m].mean():.5f} "
              f"({100 * (ca[m].mean() - cb[m].mean()) / (cb[m].mean() + 1e-12):+.2f}%)")


# Sharma, Wu & Dalal (2005), the pairs that exercise the parts of CIEDE2000
# everyone gets wrong: the hue mean's wrap-around, the Rt rotation in the
# blues, and the pairs constructed to come out at exactly 1.0.
_SHARMA = [
    ((50.0, 2.6772, -79.7751), (50.0, 0.0, -82.7485), 2.0425),
    ((50.0, 3.1571, -77.2803), (50.0, 0.0, -82.7485), 2.8615),
    ((50.0, 2.8361, -74.0200), (50.0, 0.0, -82.7485), 3.4412),
    ((50.0, -1.3802, -84.2814), (50.0, 0.0, -82.7485), 1.0000),
    ((50.0, -1.1848, -84.8006), (50.0, 0.0, -82.7485), 1.0000),
    ((50.0, -0.9009, -85.5211), (50.0, 0.0, -82.7485), 1.0000),
    ((50.0, 0.0, 0.0), (50.0, -1.0, 2.0), 2.3669),
    ((50.0, -1.0, 2.0), (50.0, 0.0, 0.0), 2.3669),
    ((50.0, 2.49, -0.001), (50.0, -2.49, 0.0009), 7.1792),
    ((50.0, 2.49, -0.001), (50.0, -2.49, 0.0011), 7.2195),
    ((50.0, -0.001, 2.49), (50.0, 0.0009, -2.49), 4.8045),
    ((50.0, 2.5, 0.0), (50.0, 0.0, -2.5), 4.3065),
    ((50.0, 2.5, 0.0), (73.0, 25.0, -18.0), 27.1492),
    ((50.0, 2.5, 0.0), (50.0, 3.1736, 0.5854), 1.0000),
    ((50.0, 2.5, 0.0), (50.0, 3.2972, 0.0), 1.0000),
    ((50.0, 2.5, 0.0), (50.0, 1.8634, 0.5757), 1.0000),
    ((50.0, 2.5, 0.0), (50.0, 3.2592, 0.3350), 1.0000),
    ((60.2574, -34.0099, 36.2677), (60.4626, -34.1751, 39.4387), 1.2644),
    ((63.0109, -31.0961, -5.8663), (62.8187, -29.7946, -4.0864), 1.2630),
    ((22.7233, 20.0904, -46.6940), (23.0331, 14.9730, -42.5619), 2.0373),
    ((2.0776, 0.0795, -1.1350), (0.9033, -0.0636, -0.5514), 0.9082),
]

# sRGB landmarks, to catch a broken transfer curve or matrix before it turns
# into a deltaE number nobody can explain.
_SRGB = [((1.0, 1.0, 1.0), (100.0, 0.0, 0.0)),
         ((0.0, 0.0, 0.0), (0.0, 0.0, 0.0)),
         ((0.5, 0.5, 0.5), (53.39, 0.0, 0.0)),
         ((1.0, 0.0, 0.0), (53.24, 80.09, 67.20))]


def cmd_selftest(args):
    bad = 0
    for a, b, want in _SHARMA:
        got = float(delta_e_2000(np.array(a)[None, :], np.array(b)[None, :])[0])
        if abs(got - want) > 1e-4:
            bad += 1
            print(f"  FAIL dE {a} vs {b}: got {got:.4f}, want {want:.4f}")
    print(f"CIEDE2000: {len(_SHARMA) - bad}/{len(_SHARMA)} Sharma vectors")

    for rgb, want in _SRGB:
        got = to_lab(np.array(rgb, dtype=float)[None, :])[0]
        if np.abs(got - np.array(want)).max() > 0.01:
            bad += 1
            print(f"  FAIL sRGB {rgb}: got {got}, want {want}")
    print(f"sRGB->Lab: {len(_SRGB)} landmarks")

    # A rendering compared with itself must be exactly zero, whatever the
    # alignment mode does.
    img = np.random.default_rng(0).random((32, 32, 3))
    for mode in ("none", "exposure", "neutral"):
        de = delta_e_2000(to_lab(img), to_lab(align_to(img, img, mode)))
        if de.max() > 1e-9:
            bad += 1
            print(f"  FAIL identity under --align {mode}: max dE {de.max():.2e}")
    print("identity: 3 alignment modes")

    # A Gaussian line spread of width s puts MTF50 at sqrt(2 ln2)/(2 pi s),
    # so a blurred synthetic edge is a slanted-edge measurement with a known
    # answer -- the only way to tell a sharpness number from a plausible one.
    from math import erf, sqrt
    verf = np.vectorize(erf)
    yy, xx = np.mgrid[0:200, 0:120]
    for sigma in (0.8, 1.2, 2.0):
        d = xx - (60.0 + np.tan(np.radians(5.0)) * (yy - 100))
        got = mtf50_of(0.5 * (1 + verf(d / (sigma * sqrt(2)))))[0]
        want = sqrt(2 * np.log(2)) / (2 * np.pi * sigma)
        if abs(got - want) / want > 0.02:
            bad += 1
            print(f"  FAIL MTF50 sigma {sigma}: got {got:.4f}, want {want:.4f}")
    print("MTF50: 3 analytic Gaussian edges")

    raise SystemExit(1 if bad else 0)


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("de", help="deltaE 2000 between two renderings")
    p.add_argument("ref"); p.add_argument("file")
    p.add_argument("--align", choices=("none", "exposure", "neutral"), default="neutral")
    p.add_argument("--space", choices=tuple(_PRIMARIES), default="srgb")
    p.add_argument("--roi", help="X,Y,W,H (default: middle half)")
    p.add_argument("--heatmap", help="write a difference map here")
    p.add_argument("--heatmap-max", type=float, default=10.0)
    p.set_defaults(fn=cmd_de)

    p = sub.add_parser("levels", help="percentiles, clipping, distinct levels")
    p.add_argument("file"); p.add_argument("--roi")
    p.set_defaults(fn=cmd_levels)

    p = sub.add_parser("noise", help="noise and SNR in a flat patch")
    p.add_argument("file"); p.add_argument("--roi")
    p.set_defaults(fn=cmd_noise)

    p = sub.add_parser("mtf", help="slanted-edge MTF50")
    p.add_argument("file"); p.add_argument("--roi")
    p.add_argument("--dpi", type=int, help="report lp/mm as well")
    p.add_argument("--horizontal", action="store_true",
                   help="the edge runs left-right, not up-down")
    p.set_defaults(fn=cmd_mtf)

    p = sub.add_parser("spectrum", help="which of two scans resolves more detail")
    p.add_argument("ref"); p.add_argument("file")
    p.add_argument("--roi"); p.add_argument("--dpi", type=int)
    p.set_defaults(fn=cmd_spectrum)

    p = sub.add_parser("defects", help="what a dust-removal pass changed")
    p.add_argument("before"); p.add_argument("after")
    p.add_argument("--threshold", type=float, default=0.02)
    p.set_defaults(fn=cmd_defects)

    p = sub.add_parser("selftest", help="check the colour maths against known values")
    p.set_defaults(fn=cmd_selftest)

    args = ap.parse_args()
    args.fn(args)


if __name__ == "__main__":
    main()
