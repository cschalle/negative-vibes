# Slanted-edge PSF target

A 3D-printable carrier, shaped like a 35 mm slide mount, that holds a
double-edge safety razor blade across the scan aperture at a slight angle.
Scanning it gives a knife edge from which the scanner's line spread function
and MTF can be measured.

## Why a carrier and not a printed edge

The thing being measured is a transition about 2 px wide at 3600 dpi — some
14 µm. For the target not to dominate the result, its edge has to be straight
to well under that at spatial periods shorter than roughly a millimetre.
Slower wander than that the edge tracker follows out; faster roughness it
cannot, and that roughness lands directly in the measurement.

| source of edge | roughness | at 3600 dpi |
|---|---|---|
| FDM print | ~100 µm | ~14 px |
| Resin print | 20–35 µm | 3–5 px |
| Double-edge razor blade | < 1 µm | < 0.15 px |

So nothing printed is used as an optical edge. The print supplies the
outline, the height and a repeatable angle; the blade supplies the edge.

A hand-cut piece of flat plastic was tried first and measured a 7.71 px rise
where the holder's own mask edge in the same scan measured 2.98 px. Edge
roughness and height error together, not the scanner.

## Why a double-edge blade

43 × 22 mm, **0.1 mm thick**, dead flat, no spine, two ground edges, sold
everywhere for very little. At 0.1 mm it is thinner than film (0.14 mm), so
whatever height error remains is smaller than real film's own.

Its central slot sits 11 mm from the cutting edge. Measurement only ever uses
a window of about ±0.3 mm around the edge, so the slot never enters it.

## Printing

```
python3 make_target.py --base 1.0          # both orientations
python3 make_target.py --base 0.7 --tilt 7 # variants
```

Six STLs are checked in, at blade heights 0.7, 1.0 and 1.3 mm. **Print all
three heights.** A slide mount holds its film somewhere near mid-depth, but
how the scanner's holder references it is not documented anywhere we can
check — so scan all three and keep whichever gives the sharpest edge. That
measurement is itself worth having: it says where the focal plane is.

- **Black** PLA or PETG. Not translucent, not white.
- 0.2 mm layers. The base is 5 layers, the ridges 5 more.
- No supports. Every feature is additive from a flat bottom face.
- 4 perimeters, or just set 100% infill — the part is thin anyway.
- Overall 50.8 × 50.8 × 2.0 mm, within the 1.3–3.2 mm a slide mount may be.

Nothing is thinner than 1.0 mm except the pocket clearance, so a 0.4 mm
nozzle has no trouble with it.

## Use

1. Drop a double-edge blade into the pocket, cutting edge toward the open
   half of the aperture. **Handle it by the spine.**
2. If it rattles, a scrap of tape across the spine end holds it. Keep tape
   out of the aperture.
3. Load into the slide holder and scan at 3600 dpi, no infrared:
   `nv-cli scan <device> --dpi 3600 -o target.pnm`
4. `psf-target-v-*` has the edge near vertical and measures the **sensor
   axis**. `psf-target-h-*` has it near horizontal and measures the **feed
   axis**.

The tilt matters and is not cosmetic. A rigid edge aligned with the pixel
grid gives every scan line the same sub-pixel phase, so there is nothing to
interleave and the measurement collapses. Film only gets away with it because
it bows. 5° is comfortable; 2–15° all work.

## Checking the output

```
python3 check_target.py psf-target-v-base1.stl 5 1.0 plan-v.png
```

Prints the bounding box and whether the part stays inside the slide outline,
and draws a plan view with the aperture in red and the blade's cutting edge
in orange. `plan-v.png` and `plan-h.png` are the two committed views.
