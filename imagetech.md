# Image technologies for Negative Vibes

Computational imaging research offers several ways to improve negative scans.
The most useful distinction is whether a method recovers information captured
across multiple measurements or estimates plausible detail from a single scan.

This file surveys the candidates and records what measurement says about each.
Everything below was measured on the OpticFilm 8200i on 2026-10-31, on one
frame; where that matters it is called out.

## What the hardware allows

Two measurements decide most of what follows.

### The scanner is optics-limited at 3600 dpi, not sampling-limited

Horizontal power spectrum of a 7200 dpi scan, read across the sensor line so
no resampling is involved, on a physical frequency axis. It decays to a
minimum at **72 cyc/mm** and then rises again toward Nyquist. Real optical
detail decays monotonically, so that rise is noise, not information.

A 72 cyc/mm cutoff is fully sampled by 3675 dpi. 3600 dpi is therefore already
critically sampled (Nyquist 70.9 cyc/mm), and 7200 dpi is empty magnification:
8.7x the data for no additional detail.

Relative power, each scan normalised at 5 cyc/mm:

| cyc/mm | 10 | 20 | 30 | 40 | 50 | 60 | 70 | 100 | 140 |
|---|---|---|---|---|---|---|---|---|---|
| 3600 dpi | 0.222 | 0.106 | 0.067 | 0.042 | 0.026 | 0.019 | 0.017 | — | — |
| 7200 dpi | 0.144 | 0.059 | 0.037 | 0.022 | 0.014 | 0.010 | 0.009 | 0.018 | 0.098 |

This is the premise behind every "combine multiple measurements" technique, and
it does not hold here.

### What looks like noise is film grain

Four averaged passes would reduce purely temporal noise by 2.00x. Measured on
the blue channel -- the weakest through an orange mask -- by negative density:

| band | 1 pass | 4 samples | 2x exposure | 4-sample gain |
|---|---|---|---|---|
| thin (scene shadow) | 20.24 | 16.80 | 15.79 | 1.20x |
| band 2 | 27.79 | 24.60 | 22.92 | 1.13x |
| band 3 | 35.62 | 32.46 | 30.13 | 1.10x |
| dense (scene highlight) | 50.52 | 47.59 | 43.69 | **1.06x** |

(high-frequency SD x 1e4, 1200 dpi, same frame throughout)

About 96% of the high-frequency variation is common between separate
acquisitions, so it is fixed grain, and the stacking gain is worst exactly
where help is most wanted. It also collides with a decision already taken:
`grain_kept` in `nv-process/src/detail.rs` exists to *preserve* that signal.

Note that 2x exposure beat 4 samples at half the time cost.

## Assessment

| idea | viable | worthwhile |
|---|---|---|
| Multi-frame / burst super-resolution | no | — |
| Spectral fusion: IR detail, hyperspectral | no | — |
| Spectral fusion: film profiles | yes, needs data | **highest** |
| RAW capture, minimal processing | already done | done |
| Multi-exposure for D_max | yes | moderate, untested |
| Reference-based super-resolution | technically | **no, harmful** |
| AI scratch repair, general | yes | no |
| AI repair, wide defects only | yes | small but real |
| Deconvolution with a measured PSF | yes | **best available** |

### Multi-frame or burst super-resolution

Capture the same negative several times with tiny subpixel shifts, register and
combine. Not viable here, on three independent grounds:

- It recovers detail lost to *undersampling*. At 3600 dpi there is none to
  recover -- the optics band-limit the image inside Nyquist.
- Subpixel shifts are possible vertically, since the motor feed is under our
  control, but impossible horizontally: the CCD is fixed to the optical axis
  and there is no lateral actuator. Vertical is the axis where the sensor
  already oversamples ~2.6x.
- The noise benefit is the 1.06-1.20x measured above, not 2x.

At 300-1200 dpi the scan genuinely is undersampled, so the method could work --
but the fix there is to scan at 3600 and downsample, which already exists and
costs nothing. Redundant where it would work, impossible where it would matter.

Reference: [Deep Burst Super-Resolution, CVPR 2021](https://openaccess.thecvf.com/content/CVPR2021/html/Bhat_Deep_Burst_Super-Resolution_CVPR_2021_paper.html)

### Multi-sensor or spectral fusion

For colour film, infrared locates dust and scratches but does not add fine
image detail to the RGB scan. Hyperspectral capture can analyse film dyes and
recover colour, but research systems capturing hundreds of bands are
specialised equipment, not consumer film scanners. Both conclusions stand.

What this misses is that **the largest measured defect in the project is
spectral**. A film profile encodes dye density against the scanner's RGB
response. The seven builtin profiles are uncorrelated with the stocks they
name: magnitude-weighted cosine between our move-from-generic and SilverFast's
is +0.079 for Gold 200 and +0.023 for Superia 400, against a metric that reads
+1.000 on self, +0.858 across cast-removal settings, and +0.332 between two
genuinely different real stocks. Negating gives -0.079 and -0.023, so it is not
a sign error; swapping the stocks gives +0.004 and +0.084, so they are not
mislabelled. They are simply unrelated.

That is a data problem, not an algorithm problem, and it is blocked on
reference material. Nothing else on this list is a comparably large measured
error.

A related latent defect: `cast_removal` defaults to 1.0 and its neutral-axis
fit runs *after* the profile's gamma and crosstalk, measuring and then removing
roughly half of what the profile did (pairwise deltaE 1.79-1.94 at 1.0 against
2.53-4.00 at 0.0). Worth fixing only once profiles are real -- while the data
is noise, the suppression is accidentally protective.

References: [Hyperspectral scanning of photographic negatives](https://pmc.ncbi.nlm.nih.gov/articles/PMC10098514/); [spectral methods for restoring faded color film](https://pmc.ncbi.nlm.nih.gov/articles/PMC9767916/)

### RAW-aware denoising, calibration, and reconstruction

Processing minimally altered scanner data preserves subtle tones and avoids
baking in sharpening or noise reduction too early. This is already the design:
16-bit raw throughout, against SilverFast SE Plus's 8-bit export, with shading
calibration in the backend.

The open part is multiple exposures, and the reason is **D_max headroom** --
capturing density a single exposure clips -- not noise, which the table above
shows stacking barely touches. Whether a single exposure actually clips at
D_max is untested and cheap to test. That measurement, not more stacking, is
the next step here.

### Reference-based super-resolution (Ref-SR)

Ref-SR needs a genuinely similar, higher-resolution reference that can be
aligned to the scan. A different photograph of the same scene may contribute
texture, but can also transfer detail that was never in the negative, and
research identifies incorrect reference use as a source of blur and artifacts.

Beyond availability, it fails this project's working standard: there is no
ground truth to measure it against. For an archival tool, importing texture
from another photograph is a correctness bug.

Reference: [Task-Decoupled Framework for Reference-Based Super-Resolution, CVPR 2022](https://openaccess.thecvf.com/content/CVPR2022/papers/Huang_Task_Decoupled_Framework_for_Reference-Based_Super-Resolution_CVPR_2022_paper.pdf)

### AI scratch and damage repair

A 2023 study of scanned analogue film found the tested restoration methods
could distort image content or smooth away film grain, and were not competitive
with professional hand restoration.

Our own data agrees. IR dust removal already achieves -68% defects against
SilverFast's -27%, so the headroom is small; and the paper's complaint about
smoothed grain is exactly what `grain_kept` was built to prevent.

One exception is worth keeping open. `plustek8200i_ir_repair_row` deliberately
refuses runs wider than 1/32 of the frame, because interpolating across them
produced visible coloured streaks. That skipped class is genuine unrepaired
damage, it is *already detected*, and it is bounded -- a defensible place for
inpainting, off by default and reversible.

Reference: [Ivanova et al., 2023, Simulating Analogue Film Damage to Analyse and Improve Artefact Restoration](https://diglib.eg.org/items/0c456dbb-e409-4979-bd49-2a0dadafd7e8)

### Deconvolution with a measured PSF

Not in the original survey, and the best remaining opportunity.

Being optics-limited means frequencies below 72 cyc/mm were *captured but
attenuated*. Restoring them recovers real information rather than inventing it,
which is the distinction this file opens with. Measured SNR at 3600 dpi sets
the honest bounds:

| cyc/mm | 10 | 20 | 30 | 40 | 50 | 60 | 70 |
|---|---|---|---|---|---|---|---|
| above noise floor | 12.3x | 5.9x | 3.8x | 2.4x | 1.5x | 1.1x | 1.0x |

So a regularised deconvolution, with a PSF measured from the film rebate edge,
has real headroom in the **10-40 cyc/mm** band -- where perceived sharpness
lives -- and must be SNR-gated above ~50 cyc/mm or it will only amplify grain.

It fits the project: measurable before and after by MTF, bounded, and it
invents nothing. It is also the only remaining lever on the ~4.5% sharpness gap
against SilverFast that the Lanczos-3 resampler did not explain.

## Priorities

1. **Film profiles.** The largest measured error. Decide how to obtain
   reference data, or relabel the named stocks honestly.
2. **Deconvolution** with a measured PSF, SNR-gated.
3. **De-emphasise 7200 dpi.** The cheapest user-facing win here: it costs 8.7x
   the data and carries no detail the 3600 dpi scan does not.
4. **Test whether a single exposure clips at D_max**, then decide on
   multi-exposure.
5. **Inpainting for wide IR-detected defects**, off by default and reversible.
6. Drop burst super-resolution and Ref-SR.

Items 1 and 2 are the only ones that would visibly change an image.

## Caveat

Every measurement here comes from one frame. The 72 cyc/mm cutoff assumes that
frame carries detail out to the optical limit; if it does not, the true cutoff
is higher and the conclusions about 7200 dpi and burst super-resolution would
need revisiting. A second frame with finer detail should confirm it before the
figure is used to build a deconvolution PSF.
