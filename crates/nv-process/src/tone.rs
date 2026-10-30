use crate::params::ToneParams;

/// Entries in a curve lookup table: finer than one 8-bit step of the encoding.
const CURVE_LUT: usize = 1024;

/// The most of any one channel auto levels may throw away at each end.
///
/// The thresholds are measured on luminance but the stretch runs per channel,
/// so a channel whose tail reaches past the luminance quantile loses more than
/// the nominal fraction -- and nothing bounded how much more. Measured against
/// SilverFast, which discards 0.00-0.01%: on the tulip frame we lost up to 2.7%
/// of the red highlights, and on `bw-negative-glass-with-light` 12.2% of the
/// picture was crushed to solid black.
///
/// A budget rather than a tighter threshold, because the two cases want
/// opposite things: that B&W frame needed the black point pulled a long way
/// back, while the tulip frame's converted picture already fills the range, so
/// tightening the threshold only costs it contrast and buys nothing. This caps
/// the damage where there is a tail to lose and leaves the stretch alone where
/// there is not.
const CLIP_BUDGET: f32 = 0.005;

/// Stage 5 — tone adjustments (black/white point, midtone gamma, contrast).
///
/// Input/output: linear f32 [0,1] (before gamma-2.2 output encoding).
pub fn apply_tone(pixels: &[f32], p: &ToneParams) -> Vec<f32> {
    let n = pixels.len() / 3;
    let mut out = vec![0.0f32; n * 3];

    let bp = p.black_point;
    let wp = p.white_point.max(bp + 0.001);
    let range = wp - bp;
    // Midtone gamma: x^(1/midtone) — applied in [0,1] after black/white remapping.
    let mid_exp = if p.midtone <= 0.0 { 1.0 } else { 1.0 / p.midtone };
    // Contrast: S-curve steepness around 0.5. contrast in [-1,1].
    let contrast = p.contrast.clamp(-1.0, 1.0);

    // Curves work on the perceptual encoding, so a point at 0.5 is a mid-grey
    // as the eye sees it. Skipped, free of cost, when no curve was touched.
    let luts = (!p.curves.is_identity()).then(|| p.curves.luts(CURVE_LUT));

    for i in 0..n {
        let bal = [pixels[i * 3] * p.balance[0], pixels[i * 3 + 1] * p.balance[1], pixels[i * 3 + 2] * p.balance[2]];

        // The black and white points are luminance: `auto_tone` reads them off
        // a luminance histogram, and both pipettes compute them with `luma()`.
        // Applying them to each channel instead kept the absolute gap between
        // channels while shrinking its magnitude, so whatever cast the
        // conversion left came out stronger -- on the tulip wall b* went from
        // -8.2 without this stage to -13.1 with it, and the frame lost to
        // SilverFast on neutrality it would otherwise have won. Move the
        // luminance and carry the channels with it: the stage changes how
        // bright a pixel is, not what colour it is.
        let l_in = 0.2126 * bal[0] + 0.7152 * bal[1] + 0.0722 * bal[2];
        // Black/white point normalisation.
        let t = ((l_in - bp) / range).clamp(0.0, 1.0);
        // Midtone gamma.
        let t = t.powf(mid_exp);
        // Contrast: simple sigmoid-like boost around 0.5.
        let t = apply_contrast(t, contrast).clamp(0.0, 1.0);
        // Below this the ratios are noise, and the pixel is black either way.
        let lit = l_in > 1e-5;
        let gain = if lit { t / l_in } else { 0.0 };

        for c in 0..3 {
            let v = if lit { (bal[c] * gain).clamp(0.0, 1.0) } else { t };
            out[i * 3 + c] = match &luts {
                Some(l) => {
                    let e = v.powf(1.0 / 2.2) * (CURVE_LUT - 1) as f32;
                    let (k, t) = (e as usize, e.fract());
                    let y = l[c][k] * (1.0 - t) + l[c][(k + 1).min(CURVE_LUT - 1)] * t;
                    y.powf(2.2)
                }
                None => v,
            };
        }
    }
    out
}

/// Lift or lower contrast with a symmetric adjustment around 0.5.
/// contrast in [-1, 1]: positive increases contrast (compresses midtones),
/// negative reduces it.
fn apply_contrast(v: f32, contrast: f32) -> f32 {
    if contrast == 0.0 {
        return v;
    }
    // Map to [-0.5, 0.5], apply power, map back.
    let c = v - 0.5;
    // strength: higher contrast → steeper slope near 0.
    let strength = 1.0 + contrast.abs() * 2.0;
    let sign = c.signum();
    let result = if contrast > 0.0 {
        // Compress: push values away from 0.5.
        sign * c.abs().powf(1.0 / strength)
    } else {
        // Expand: pull values toward 0.5.
        sign * c.abs().powf(strength)
    };
    result + 0.5
}

/// Auto levels (F§5.1): black and white points from percentiles of the
/// converted image's luminance (measured through `balance`, which is kept),
/// and the midtone that puts its median at middle grey. Colour cast is not
/// guessed here: equalising channel medians turned the tulip wall blue-green,
/// because the scene is mostly red and green. Cast removal is a fit in the
/// conversion, and the grey pipette is for the rest.
pub fn auto_tone(converted: &[f32], w: usize, h: usize, region: Option<[f32; 4]>, low: f32, high: f32, balance: [f32; 3]) -> ToneParams {
    let (x0, x1, y0, y1) = crate::conversion::interior(w, h, region);

    const BINS: usize = 2048;
    let bin = |v: f32| ((v.clamp(0.0, 1.0) * (BINS - 1) as f32) as usize).min(BINS - 1);
    let quantile = |hist: &[u32], frac: f32| -> f32 {
        let total: u32 = hist.iter().sum();
        let target = ((total as f32 * frac) as u32).max(1);
        let mut seen = 0;
        for (i, n) in hist.iter().enumerate() {
            seen += n;
            if seen >= target {
                return i as f32 / (BINS - 1) as f32;
            }
        }
        1.0
    };

    let mut hist = vec![0u32; BINS];
    // Per channel as well as luminance: the points are chosen on luminance but
    // spent per channel, and only the channel histograms can say what that
    // costs.
    let mut chans = [vec![0u32; BINS], vec![0u32; BINS], vec![0u32; BINS]];
    for y in y0..y1 {
        for x in x0..x1 {
            let i = (y * w + x) * 3;
            let l = 0.2126 * converted[i] * balance[0] + 0.7152 * converted[i + 1] * balance[1] + 0.0722 * converted[i + 2] * balance[2];
            hist[bin(l)] += 1;
            for c in 0..3 {
                chans[c][bin(converted[i + c] * balance[c])] += 1;
            }
        }
    }
    let (bp, wp, med) = (quantile(&hist, low), quantile(&hist, high), quantile(&hist, 0.5));
    if wp - bp < 0.02 {
        return ToneParams { balance, ..ToneParams::default() }; // flat image: nothing to stretch
    }

    // What the budget is spent on is the picture that arrived intact. Pixels
    // already sitting on 0 or 1 were saturated upstream, in the conversion, and
    // holding the points back cannot bring them back -- it would only cost the
    // frame its contrast and return nothing. So the end bins are left out of
    // the count, and the budget bounds what auto levels itself throws away.
    let budget_point = |hist: &[u32], from_top: bool| -> f32 {
        let inner = &hist[1..BINS - 1];
        let total: u32 = inner.iter().sum();
        if total == 0 {
            return if from_top { 0.0 } else { 1.0 };
        }
        let target = ((total as f32 * CLIP_BUDGET) as u32).max(1);
        let mut seen = 0;
        let mut hit = |i: usize, n: u32| -> Option<f32> {
            seen += n;
            (seen >= target).then(|| (i + 1) as f32 / (BINS - 1) as f32)
        };
        let found = if from_top { inner.iter().enumerate().rev().find_map(|(i, n)| hit(i, *n)) } else { inner.iter().enumerate().find_map(|(i, n)| hit(i, *n)) };
        found.unwrap_or(if from_top { 0.0 } else { 1.0 })
    };

    // Pull the points back to whatever the worst channel can afford. Where a
    // channel has no tail past them this changes nothing and the stretch stands.
    let (mut bp, mut wp) = (bp, wp);
    for c in 0..3 {
        bp = bp.min(budget_point(&chans[c], false));
        wp = wp.max(budget_point(&chans[c], true));
    }
    if wp - bp < 0.02 {
        return ToneParams { balance, ..ToneParams::default() };
    }

    // F§5.1 also carries `AutoTargetBlack/White`, which would land the points
    // a little inside 0 and 1 instead of on them. Left out deliberately: the
    // budget already bounds what is thrown away, and the headroom on top of it
    // only costs contrast -- on the tulip frame it turned a stretch into a
    // compression, taking the interior 2-98 spread below where it started.

    // Median after the black/white stretch, moved to 18% linear (middle grey).
    let v = ((med - bp) / (wp - bp)).clamp(0.01, 0.99);
    let midtone = (v.ln() / 0.18f32.ln()).clamp(0.4, 2.5);
    ToneParams { black_point: bp, white_point: wp, midtone, contrast: 0.0, balance, curves: Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curves::Curves;

    /// Auto levels moves brightness, not colour. It used to apply its black
    /// and white points to each channel, which holds the gap between channels
    /// while shrinking its magnitude and so makes any cast stronger: on the
    /// tulip wall b* went from -8.2 to -13.1 just by running this stage.
    #[test]
    fn the_tone_stage_changes_brightness_and_not_hue() {
        // A picture with a deliberate blue cast, spread over the range so the
        // stretch has something to do.
        let (w, h) = (300, 60);
        let mut px = vec![0.0f32; w * h * 3];
        for i in 0..w * h {
            let t = 0.12 + 0.55 * (i % w) as f32 / w as f32;
            px[i * 3] = t * 0.92;
            px[i * 3 + 1] = t;
            px[i * 3 + 2] = t * 1.18;
        }
        let tone = auto_tone(&px, w, h, None, 0.02, 0.98, [1.0; 3]);
        assert!(tone.white_point - tone.black_point < 0.99, "the stretch did nothing to preserve");
        let out = apply_tone(&px, &tone);

        let mut checked = 0;
        for i in 0..w * h {
            let o = [out[i * 3], out[i * 3 + 1], out[i * 3 + 2]];
            // Only where nothing has hit an end: a clamp cannot keep a ratio.
            if o.iter().any(|v| *v <= 0.001 || *v >= 0.999) {
                continue;
            }
            checked += 1;
            for c in [0usize, 2] {
                let want = px[i * 3 + c] / px[i * 3 + 1];
                let got = o[c] / o[1];
                assert!((got - want).abs() < 0.01, "pixel {i} channel {c}: ratio {got:.4} against {want:.4}");
            }
        }
        assert!(checked > w * h / 4, "too few pixels left unclamped to test: {checked}");
    }

    /// The points are chosen on luminance but spent per channel, so a channel
    /// with a tail the luminance histogram barely sees must still not be
    /// crushed: on real frames this cost up to 3.5% of one channel.
    #[test]
    fn auto_levels_never_spend_more_than_the_budget_on_one_channel() {
        // Green carries the luminance, so a long dark tail in red is nearly
        // invisible to the luminance histogram that picks the black point.
        // Wide, so one column of the ramp is a tenth of the budget rather than
        // a whole step of it: the quantiles are binned and land on a column.
        let (w, h) = (1000, 60);
        let mut px = vec![0.0f32; w * h * 3];
        for i in 0..w * h {
            let t = (i % w) as f32 / w as f32;
            px[i * 3] = if i % 250 == 0 { 0.001 } else { 0.35 + 0.4 * t };
            px[i * 3 + 1] = 0.30 + 0.45 * t;
            px[i * 3 + 2] = 0.32 + 0.42 * t;
        }
        let t = auto_tone(&px, w, h, None, 0.02, 0.98, [1.0; 3]);

        // Over the region auto levels actually measures: it ignores the border,
        // where the film rebate is.
        let (x0, x1, y0, y1) = crate::conversion::interior(w, h, None);
        for c in 0..3 {
            let vals: Vec<f32> = (y0..y1).flat_map(|y| (x0..x1).map(move |x| (y, x))).map(|(y, x)| px[(y * w + x) * 3 + c]).collect();
            let below = vals.iter().filter(|v| **v < t.black_point).count();
            let above = vals.iter().filter(|v| **v > t.white_point).count();
            let n = vals.len() as f32;
            // Slack for the binning: a quantile lands on a bin edge and on a
            // column of the ramp, and both can carry pixels past it.
            let slack = CLIP_BUDGET + 1.0 / 2048.0 + 1.0 / w as f32;
            assert!(below as f32 / n <= slack, "channel {c}: {} of {n} below the black point", below);
            assert!(above as f32 / n <= slack, "channel {c}: {} of {n} above the white point", above);
        }
    }

    /// A ramp from 0.2 to 0.6 is stretched to the full range.
    #[test]
    fn auto_levels_stretch_a_narrow_ramp() {
        let w = 400;
        let px: Vec<f32> = (0..w * 40)
            .flat_map(|i| {
                let v = 0.2 + 0.4 * (i % w) as f32 / w as f32;
                [v, v, v]
            })
            .collect();
        let t = auto_tone(&px, w, 40, None, 0.02, 0.98, [1.0; 3]);
        assert!((t.black_point - 0.240).abs() < 0.01, "{t:?}");
        assert!((t.white_point - 0.560).abs() < 0.01, "{t:?}");
        let out = apply_tone(&px, &t);
        let lo = out.iter().cloned().fold(1.0f32, f32::min);
        let hi = out.iter().cloned().fold(0.0f32, f32::max);
        assert!(lo < 0.02 && hi > 0.98, "{lo} {hi}");
    }

    #[test]
    fn auto_levels_leave_a_flat_image_alone() {
        let px = vec![0.4f32; 30 * 30 * 3];
        let t = auto_tone(&px, 30, 30, None, 0.02, 0.98, [1.0; 3]);
        assert_eq!((t.black_point, t.white_point, t.midtone), (0.0, 1.0, 1.0));
    }

    /// The median lands on middle grey (18% linear).
    #[test]
    fn auto_midtone_puts_the_median_at_middle_grey() {
        let w = 400;
        let px: Vec<f32> = (0..w * 40)
            .flat_map(|i| {
                let v = (i % w) as f32 / w as f32;
                [v * v, v * v, v * v]
            })
            .collect();
        let t = auto_tone(&px, w, 40, None, 0.0, 1.0, [1.0; 3]);
        let out = apply_tone(&px, &t);
        let mut luma: Vec<f32> = out.chunks(3).map(|p| p[1]).collect();
        luma.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let med = luma[luma.len() / 2];
        assert!((med - 0.18).abs() < 0.03, "median {med}, {t:?}");
    }

    #[test]
    fn curves_change_the_midtones_and_leave_the_ends() {
        use crate::curves::Curve;
        let p = ToneParams { curves: Curves { master: Curve(vec![[0.0, 0.0], [0.5, 0.35], [1.0, 1.0]]), ..Default::default() }, ..ToneParams::default() };
        let out = apply_tone(&[0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.218, 0.218, 0.218], &p);
        assert!(out[0].abs() < 1e-3 && (out[3] - 1.0).abs() < 1e-3, "{out:?}");
        // 0.218 linear is 0.5 encoded; the curve takes it to 0.35 encoded.
        assert!((out[6] - 0.35f32.powf(2.2)).abs() < 0.01, "{}", out[6]);
    }

    #[test]
    fn identity_curves_change_nothing() {
        let px = [0.1, 0.4, 0.8];
        assert_eq!(apply_tone(&px, &ToneParams::default()), px.to_vec());
    }
}
