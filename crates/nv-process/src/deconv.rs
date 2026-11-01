//! Stage 3b, deconvolution (DESIGN.md §7.8): undo as much of the scanner's own
//! blur as the film's signal-to-noise will pay for.
//!
//! Unlike unsharp mask in [`crate::detail`], which works on the finished
//! picture because that is where "sharper" means something to the eye, this
//! runs on the linear raw before conversion. The blur being removed happened
//! in linear light, in the optics, so that is the only place it can be
//! removed without inventing structure.
//!
//! The point spread function is measured, not assumed: a razor blade in a
//! printed carrier, scanned at 3600 dpi, with the carrier's height swept until
//! the blade sat on the focal plane (`tools/psf-target/`). The two axes differ
//! enough above 40 cyc/mm to be worth separate kernels, so the filter is
//! separable with one kernel per axis rather than one round one.
//!
//! How hard to invert is not something the measurement decides. A linear
//! filter multiplies signal and noise by the same amount at every frequency,
//! so this cannot improve signal-to-noise -- it corrects rendition, and it
//! makes grain louder in the same proportion it makes detail louder. What
//! decides the shape is how much of what is up there is real. Measured on
//! reference frames at 3600 dpi, picture power over the scan's own noise
//! floor runs 8.5x at 20 cyc/mm, 4.6x at 30, 2.4x at 40, 1.1x at 50 and 0.4x
//! at 60. So the filter restores hard below 40, where the band is mostly
//! picture, and hands over to the signal-to-noise estimate above it, where it
//! is mostly grain.

use serde::{Deserialize, Serialize};

use crate::deconv_psf::{F_MAX, FEED, REFERENCE_PX_PER_MM, SAMPLES, SENSOR};

/// Taps either side of centre. The halo in the measured spread function
/// reaches past this, but what is past it is a few percent of the energy
/// spread over tens of pixels: a wide, shallow correction that a kernel this
/// size cannot make and that shows as large-area contrast rather than detail.
const RADIUS: usize = 10;
/// Length of a kernel, centre included.
pub const TAPS: usize = 2 * RADIUS + 1;

/// Regularisation at full strength. Smaller inverts harder. 0.08 was chosen by
/// measurement: at that value the net transfer predicted for each axis lands
/// within 5% of what re-measuring a filtered slanted edge actually gives,
/// while 0.04 overpromises by 13% on the sensor axis, where the filter starts
/// leaning on the noisiest part of the measurement.
const LAMBDA_FULL: f32 = 0.08;
/// Where restoring gives way to suppressing, in cycles per millimetre, and how
/// wide the handover is. Fixed, not per frame and not per axis: derived per
/// frame it makes the sharpening depend on which way the subject happens to
/// carry its detail, and per axis it sharpens one direction more than the
/// other.
const CROSSOVER: f32 = 40.0;
const HANDOVER: f32 = 20.0;

/// Picture power over the scan's own noise floor, in power, against cycles per
/// millimetre: the measurement that sets where the handover goes.
const SNR_F: [f32; 9] = [0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 200.0];
const SNR_P: [f32; 9] = [40.0, 34.2, 8.5, 4.6, 2.4, 1.1, 0.4, 0.2, 0.2];

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DeconvParams {
    /// How much of the measured blur to undo, in percent. 0 is off; 100 is the
    /// measured default; above 100 inverts harder than the measurement can
    /// vouch for.
    pub strength: f32,
    /// What the scan samples at, in pixels per millimetre, so the measured
    /// spread function can be matched to it. `None` means the resolution is
    /// not known -- a raw PNM does not record it -- and nothing is done,
    /// because a spread function applied at the wrong pitch is not a
    /// correction, it is a different filter.
    pub px_per_mm: Option<f32>,
}

impl Default for DeconvParams {
    fn default() -> Self {
        Self { strength: 100.0, px_per_mm: None }
    }
}

impl DeconvParams {
    pub fn is_off(&self) -> bool {
        self.strength <= 0.0 || self.px_per_mm.is_none()
    }
}

fn lerp_table(xs: &[f32], ys: &[f32], x: f32) -> f32 {
    if x <= xs[0] {
        return ys[0];
    }
    for i in 1..xs.len() {
        if x <= xs[i] {
            let t = (x - xs[i - 1]) / (xs[i] - xs[i - 1]);
            return ys[i - 1] + t * (ys[i] - ys[i - 1]);
        }
    }
    ys[ys.len() - 1]
}

/// Samples of a folded transfer, from 0 to Nyquist of the grid in question.
const BANDS: usize = 129;

/// The stored continuous transfer at `f` cycles per reference pixel.
fn stored(axis: &[f32; SAMPLES], f: f32) -> f32 {
    let s = f.abs() / F_MAX * (SAMPLES - 1) as f32;
    if s >= (SAMPLES - 1) as f32 {
        return 0.0;
    }
    let i = s.floor() as usize;
    let t = s - i as f32;
    axis[i] * (1.0 - t) + axis[i + 1] * t
}

/// The scanner's transfer along one axis as a picture sampled at `px_per_mm`
/// after a `factor` reduction actually carries it.
///
/// Two things happen between the stored curve and that picture. A proxy
/// averages `factor` neighbouring scan pixels, which is its own blur: not a
/// continuous box but a moving average of `factor` samples, whose transfer is
/// the Dirichlet kernel and is exactly 1 when `factor` is 1, where a `sinc`
/// would have wrongly blurred a picture nothing had been done to. Then
/// everything above the output grid's Nyquist folds back onto the band below
/// it, because that is what sampling does to a response the optics carry past
/// it -- and these optics do.
fn transfer_for(axis: &[f32; SAMPLES], px_per_mm: f32, factor: usize) -> [f32; BANDS] {
    let mut out = [0f32; BANDS];
    let scale = px_per_mm / REFERENCE_PX_PER_MM;
    let n = factor as f32;
    for i in 0..BANDS {
        let f = 0.5 * i as f32 / (BANDS - 1) as f32; // cycles per output pixel
        let mut acc = 0f32;
        for k in 0..4 {
            for g in [f + k as f32, (k + 1) as f32 - f] {
                // The same physical frequency in cycles per reference pixel.
                let ref_f = g / n / scale;
                let pi_g = std::f32::consts::PI * g;
                // Averaging one pixel is the identity, and at zero frequency
                // the Dirichlet kernel's ratio is 0/0 but its limit is 1.
                let box_t = if factor == 1 || pi_g.abs() < 1e-6 {
                    1.0
                } else {
                    let den = n * (pi_g / n).sin();
                    if den.abs() < 1e-9 { 0.0 } else { (pi_g.sin() / den).abs() }
                };
                acc += stored(axis, ref_f) * box_t;
            }
        }
        out[i] = acc;
    }
    let dc = out[0];
    if dc > 0.0 {
        for v in out.iter_mut() {
            *v /= dc;
        }
    }
    out
}

/// One axis's kernel: a regularised inverse of `transfer`, as a symmetric FIR
/// normalised to unit sum so it cannot shift the picture's brightness.
fn kernel_from(transfer: &[f32; BANDS], strength: f32, px_per_mm: f32) -> [f32; TAPS] {
    let lambda_full = LAMBDA_FULL * 100.0 / strength.max(1e-3);
    let mut gain = [0f32; BANDS];
    for i in 0..BANDS {
        let f_px = 0.5 * i as f32 / (BANDS - 1) as f32;
        let f_mm = f_px * px_per_mm;
        let snr = lerp_table(&SNR_F, &SNR_P, f_mm).max(1e-3);
        let t = ((f_mm - CROSSOVER) / HANDOVER).clamp(0.0, 1.0);
        let lambda = lambda_full + (1.0 / snr - lambda_full) * t;
        let d = transfer[i];
        gain[i] = d / (d * d + lambda);
    }
    let g0 = gain[0];
    if g0 > 0.0 {
        for v in gain.iter_mut() {
            *v /= g0;
        }
    }
    // h[k] = 2 * integral over [0, 1/2] of G(f) cos(2 pi f k) df. The transfer
    // is real and even because the spread function was symmetrised, so the
    // kernel is real and symmetric and this cosine sum is the whole inverse.
    let mut half = [0f32; RADIUS + 1];
    let df = 0.5 / (BANDS - 1) as f32;
    for (k, slot) in half.iter_mut().enumerate() {
        let mut acc = 0f32;
        for i in 0..BANDS {
            let f = 0.5 * i as f32 / (BANDS - 1) as f32;
            let w = if i == 0 || i == BANDS - 1 { 0.5 } else { 1.0 };
            acc += w * gain[i] * (std::f32::consts::TAU * f * k as f32).cos();
        }
        *slot = 2.0 * acc * df;
    }
    let mut taps = [0f32; TAPS];
    for k in 0..=RADIUS {
        taps[RADIUS + k] = half[k];
        taps[RADIUS - k] = half[k];
    }
    let sum: f32 = taps.iter().sum();
    if sum.abs() > 1e-6 {
        for v in taps.iter_mut() {
            *v /= sum;
        }
    }
    taps
}

/// The two kernels for a picture sampled at `px_per_mm` after a `factor`
/// reduction: down the scan (the feed axis) and across it (the sensor axis).
pub fn kernels(params: &DeconvParams, factor: usize) -> Option<([f32; TAPS], [f32; TAPS])> {
    let px_per_mm = params.px_per_mm?;
    if params.strength <= 0.0 || px_per_mm <= 0.0 || factor == 0 {
        return None;
    }
    // The spread function was measured at one resolution and is only honest
    // near it, so the *scan's* pitch is what gates this -- a genuine 1200 dpi
    // scan is not a 3600 dpi scan averaged down, it reads the sensor
    // differently, and correcting it with this curve would be inventing a
    // measurement rather than using one.
    let ratio = px_per_mm / REFERENCE_PX_PER_MM;
    if !(0.7..=1.4).contains(&ratio) {
        return None;
    }
    // The proxy's reduction, by contrast, is understood: it is an average of
    // `factor` neighbouring pixels and `transfer_for` accounts for it. Past
    // this much reduction the kernel is wider than the detail it is correcting
    // and the preview gains nothing a cheaper sharpening would not.
    if factor > 8 {
        return None;
    }
    let out_px_per_mm = px_per_mm / factor as f32;
    let feed = transfer_for(&FEED, px_per_mm, factor);
    let sensor = transfer_for(&SENSOR, px_per_mm, factor);
    Some((kernel_from(&feed, params.strength, out_px_per_mm), kernel_from(&sensor, params.strength, out_px_per_mm)))
}

/// How far beyond a tile's edge the kernel reaches, so a tile can be rendered
/// with a margin and trimmed. Without this a 1:1 view would show a seam at
/// every tile boundary, where the clamp invented a border that is not there.
pub fn margin(params: &DeconvParams) -> usize {
    if params.is_off() { 0 } else { RADIUS }
}

/// Filter one axis of interleaved RGB in place, reading `src` and writing
/// `dst`. `stride` is how many samples apart neighbouring pixels are along the
/// axis, `count` how many of them there are, `runs` how many such lines.
fn pass(src: &[f32], dst: &mut [f32], taps: &[f32; TAPS], w: usize, h: usize, down: bool) {
    let (count, runs) = if down { (h, w) } else { (w, h) };
    for run in 0..runs {
        for i in 0..count {
            let mut acc = [0f32; 3];
            for (t, &k) in taps.iter().enumerate() {
                if k == 0.0 {
                    continue;
                }
                let j = (i as isize + t as isize - RADIUS as isize).clamp(0, count as isize - 1) as usize;
                let off = if down { (j * w + run) * 3 } else { (run * w + j) * 3 };
                acc[0] += k * src[off];
                acc[1] += k * src[off + 1];
                acc[2] += k * src[off + 2];
            }
            let off = if down { (i * w + run) * 3 } else { (run * w + i) * 3 };
            dst[off] = acc[0];
            dst[off + 1] = acc[1];
            dst[off + 2] = acc[2];
        }
    }
}

/// Deconvolve `pixels` (interleaved linear RGB, `w × h`) in place. Does
/// nothing when the strength is zero or the sampling is unknown.
pub fn apply(pixels: &mut [f32], w: usize, h: usize, params: &DeconvParams, factor: usize) {
    if w == 0 || h == 0 || pixels.len() < w * h * 3 {
        return;
    }
    let Some((feed, sensor)) = kernels(params, factor) else { return };
    let mut tmp = vec![0f32; pixels.len()];
    pass(pixels, &mut tmp, &feed, w, h, true);
    pass(&tmp, pixels, &sensor, w, h, false);
    // Inverting a low pass overshoots at an edge: 3.9% of the step on the feed
    // axis and 5.3% on the sensor axis, measured on an edge first blurred by
    // the spread function itself, which is the only edge the scanner can
    // actually deliver. Linear light has no headroom past full scale and none
    // below zero, so that overshoot is clipped here rather than left for a
    // later stage to find.
    for v in pixels.iter_mut() {
        *v = v.clamp(0.0, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REF: f32 = REFERENCE_PX_PER_MM;

    /// The measured spread function itself, as a kernel, so a test can blur a
    /// synthetic edge the way the scanner would before asking the filter to
    /// undo it. Without this a test would only ever measure the filter against
    /// an edge the scanner could never deliver.
    fn blur_kernel(axis: &[f32; SAMPLES]) -> Vec<f32> {
        let r = 24usize;
        let bands = 513usize;
        let mut half = vec![0f32; r + 1];
        let df = 0.5 / (bands - 1) as f32;
        for k in 0..=r {
            let mut acc = 0f32;
            for i in 0..bands {
                let f = 0.5 * i as f32 / (bands - 1) as f32;
                // Folded, because the edge being built is a sampled one.
                let mut d = 0f32;
                for j in 0..4 {
                    d += stored(axis, f + j as f32) + stored(axis, (j + 1) as f32 - f);
                }
                let w = if i == 0 || i == bands - 1 { 0.5 } else { 1.0 };
                acc += w * d * (std::f32::consts::TAU * f * k as f32).cos();
            }
            half[k] = 2.0 * acc * df;
        }
        let mut full = vec![0f32; 2 * r + 1];
        for k in 0..=r {
            full[r + k] = half[k];
            full[r - k] = half[k];
        }
        let s: f32 = full.iter().sum();
        full.iter_mut().for_each(|v| *v /= s);
        full
    }

    fn convolve(sig: &[f32], k: &[f32]) -> Vec<f32> {
        let r = k.len() / 2;
        (0..sig.len())
            .map(|i| {
                k.iter()
                    .enumerate()
                    .map(|(t, &w)| {
                        let j = (i as isize + t as isize - r as isize).clamp(0, sig.len() as isize - 1) as usize;
                        w * sig[j]
                    })
                    .sum()
            })
            .collect()
    }

    fn rise_10_90(y: &[f32]) -> f32 {
        let at = |level: f32| -> f32 {
            for i in 1..y.len() {
                if y[i - 1] < level && y[i] >= level {
                    return i as f32 - 1.0 + (level - y[i - 1]) / (y[i] - y[i - 1]);
                }
            }
            f32::NAN
        };
        at(0.9) - at(0.1)
    }

    #[test]
    fn off_until_told_what_the_scan_was() {
        // A raw PNM does not record its resolution, so the default has to be
        // inert: a spread function applied at the wrong pitch is a different
        // filter, not a correction.
        assert!(DeconvParams::default().is_off());
        assert!(kernels(&DeconvParams::default(), 1).is_none());
        assert!(kernels(&DeconvParams { strength: 0.0, px_per_mm: Some(REF) }, 1).is_none());
    }

    #[test]
    fn only_at_the_resolution_it_was_measured_at() {
        for dpi in [600.0f32, 1200.0, 2400.0, 7200.0] {
            let p = DeconvParams { strength: 100.0, px_per_mm: Some(dpi / 25.4) };
            assert!(kernels(&p, 1).is_none(), "{dpi} dpi should be left alone");
        }
        let p = DeconvParams { strength: 100.0, px_per_mm: Some(3600.0 / 25.4) };
        assert!(kernels(&p, 1).is_some(), "3600 dpi is what was measured");
    }

    #[test]
    fn kernels_keep_the_brightness_they_were_given() {
        for strength in [25.0f32, 50.0, 100.0, 150.0, 200.0] {
            for factor in 1..=8 {
                let p = DeconvParams { strength, px_per_mm: Some(REF) };
                let (feed, sensor) = kernels(&p, factor).expect("kernels");
                for (name, k) in [("feed", feed), ("sensor", sensor)] {
                    let sum: f32 = k.iter().sum();
                    assert!((sum - 1.0).abs() < 1e-4, "{name} at {strength}% factor {factor}: sum {sum}");
                }
            }
        }
    }

    #[test]
    fn a_flat_field_comes_out_flat() {
        // Every tap summing to one is necessary but not sufficient: the two
        // passes must also leave the edges of the picture alone, where the
        // kernel runs off the end and the clamp repeats the border.
        let (w, h) = (24usize, 18usize);
        let mut px = vec![0.42f32; w * h * 3];
        apply(&mut px, w, h, &DeconvParams { strength: 100.0, px_per_mm: Some(REF) }, 1);
        for (i, v) in px.iter().enumerate() {
            assert!((v - 0.42).abs() < 1e-5, "sample {i} became {v}");
        }
    }

    #[test]
    fn zero_strength_changes_nothing() {
        let (w, h) = (16usize, 12usize);
        let before: Vec<f32> = (0..w * h * 3).map(|i| ((i * 37) % 251) as f32 / 251.0).collect();
        let mut after = before.clone();
        apply(&mut after, w, h, &DeconvParams { strength: 0.0, px_per_mm: Some(REF) }, 1);
        assert_eq!(before, after, "strength 0 must not touch a single sample");
    }

    #[test]
    fn taps_match_the_measurement() {
        let p = DeconvParams { strength: 100.0, px_per_mm: Some(REF) };
        let (feed, sensor) = kernels(&p, 1).expect("kernels");
        // Centre and the two neighbours, which carry nearly all of the effect.
        // Pinned so that regenerating the tables from the CSVs cannot quietly
        // change what the default does.
        let expect_feed = [0.998_62f32, 0.323_57, -0.317_31];
        let expect_sensor = [0.979_21f32, 0.269_94, -0.289_02];
        for (i, want) in expect_feed.iter().enumerate() {
            assert!((feed[RADIUS + i] - want).abs() < 2e-3, "feed tap {i}: {} vs {want}", feed[RADIUS + i]);
        }
        for (i, want) in expect_sensor.iter().enumerate() {
            assert!((sensor[RADIUS + i] - want).abs() < 2e-3, "sensor tap {i}: {} vs {want}", sensor[RADIUS + i]);
        }
        // Symmetric, so it cannot shift the picture sideways.
        for k in 1..=RADIUS {
            assert_eq!(feed[RADIUS + k], feed[RADIUS - k]);
            assert_eq!(sensor[RADIUS + k], sensor[RADIUS - k]);
        }
    }

    #[test]
    fn undoes_the_blur_it_was_measured_from() {
        let p = DeconvParams { strength: 100.0, px_per_mm: Some(REF) };
        let (feed, sensor) = kernels(&p, 1).expect("kernels");
        for (name, axis, k, want_before, want_after) in [
            ("feed", &FEED, feed, 2.895f32, 2.002f32),
            ("sensor", &SENSOR, sensor, 2.653, 1.839),
        ] {
            let mut step = vec![0f32; 120];
            step[60..].fill(1.0);
            let blurred = convolve(&step, &blur_kernel(axis));
            let out = convolve(&blurred, &k);
            let before = rise_10_90(&blurred);
            let after = rise_10_90(&out);
            assert!((before - want_before).abs() < 0.05, "{name} blurred rise {before}");
            assert!((after - want_after).abs() < 0.05, "{name} corrected rise {after}");
            // Inverting a low pass rings. It must stay small on an edge the
            // scanner could actually deliver, which is why the step is blurred
            // first: against an ideal step this same filter overshoots by 32%.
            let over = out.iter().cloned().fold(f32::MIN, f32::max) - 1.0;
            assert!(over < 0.08, "{name} overshoot {over}");
        }
    }

    #[test]
    fn more_strength_sharpens_more() {
        let mut last = f32::MAX;
        for strength in [25.0f32, 50.0, 100.0, 200.0] {
            let p = DeconvParams { strength, px_per_mm: Some(REF) };
            let (feed, _) = kernels(&p, 1).expect("kernels");
            let mut step = vec![0f32; 120];
            step[60..].fill(1.0);
            let out = convolve(&convolve(&step, &blur_kernel(&FEED)), &feed);
            let rise = rise_10_90(&out);
            assert!(rise < last, "{strength}% gave rise {rise}, not shorter than {last}");
            last = rise;
        }
    }
}
