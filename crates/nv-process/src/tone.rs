use crate::params::ToneParams;

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

    for i in 0..n {
        for c in 0..3 {
            let v = pixels[i * 3 + c];
            // Black/white point normalisation.
            let v = ((v - bp) / range).clamp(0.0, 1.0);
            // Midtone gamma.
            let v = v.powf(mid_exp);
            // Contrast: simple sigmoid-like boost around 0.5.
            let v = apply_contrast(v, contrast);
            out[i * 3 + c] = v.clamp(0.0, 1.0);
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
/// converted image's luminance, and the midtone that puts its median at
/// middle grey. `inset` is the fraction of each side ignored, so rebate and
/// holder do not set the points.
pub fn auto_tone(converted: &[f32], w: usize, h: usize, inset: f32, low: f32, high: f32) -> ToneParams {
    let (bw, bh) = ((w as f32 * inset) as usize, (h as f32 * inset) as usize);
    let (x0, x1, y0, y1) = if w > 2 * bw + 2 && h > 2 * bh + 2 { (bw, w - bw, bh, h - bh) } else { (0, w, 0, h) };

    const BINS: usize = 2048;
    let mut hist = vec![0u32; BINS];
    for y in y0..y1 {
        for x in x0..x1 {
            let i = (y * w + x) * 3;
            let l = 0.2126 * converted[i] + 0.7152 * converted[i + 1] + 0.0722 * converted[i + 2];
            hist[((l.clamp(0.0, 1.0) * (BINS - 1) as f32) as usize).min(BINS - 1)] += 1;
        }
    }
    let total: u32 = hist.iter().sum();
    if total == 0 {
        return ToneParams::default();
    }
    let at = |frac: f32| -> f32 {
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
    let (bp, wp, med) = (at(low), at(high), at(0.5));
    if wp - bp < 0.02 {
        return ToneParams::default(); // flat image: nothing to stretch
    }

    // Median after the black/white stretch, moved to 18% linear (middle grey).
    let v = ((med - bp) / (wp - bp)).clamp(0.01, 0.99);
    let midtone = (v.ln() / 0.18f32.ln()).clamp(0.4, 2.5);
    ToneParams { black_point: bp, white_point: wp, midtone, contrast: 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ramp from 0.2 to 0.6 is stretched to the full range.
    #[test]
    fn auto_levels_stretch_a_narrow_ramp() {
        let w = 400;
        let px: Vec<f32> = (0..w * 40).flat_map(|i| { let v = 0.2 + 0.4 * (i % w) as f32 / w as f32; [v, v, v] }).collect();
        let t = auto_tone(&px, w, 40, 0.0, 0.02, 0.98);
        assert!((t.black_point - 0.208).abs() < 0.01, "{t:?}");
        assert!((t.white_point - 0.592).abs() < 0.01, "{t:?}");
        let out = apply_tone(&px, &t);
        let lo = out.iter().cloned().fold(1.0f32, f32::min);
        let hi = out.iter().cloned().fold(0.0f32, f32::max);
        assert!(lo < 0.02 && hi > 0.98, "{lo} {hi}");
    }

    #[test]
    fn auto_levels_leave_a_flat_image_alone() {
        let px = vec![0.4f32; 30 * 30 * 3];
        let t = auto_tone(&px, 30, 30, 0.0, 0.02, 0.98);
        assert_eq!((t.black_point, t.white_point, t.midtone), (0.0, 1.0, 1.0));
    }

    /// The median lands on middle grey (18% linear).
    #[test]
    fn auto_midtone_puts_the_median_at_middle_grey() {
        let w = 400;
        let px: Vec<f32> = (0..w * 40).flat_map(|i| { let v = (i % w) as f32 / w as f32; [v * v, v * v, v * v] }).collect();
        let t = auto_tone(&px, w, 40, 0.0, 0.0, 1.0);
        let out = apply_tone(&px, &t);
        let mut luma: Vec<f32> = out.chunks(3).map(|p| p[1]).collect();
        luma.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let med = luma[luma.len() / 2];
        assert!((med - 0.18).abs() < 0.03, "median {med}, {t:?}");
    }
}
