use crate::params::ProcessParams;

/// Stage 4 — colour negative inversion (§7.4 of DESIGN.md).
///
/// Input:  linear f32 [0,1] transmission values (T_c).
/// Output: linear f32 [0,1] positive light values ready for tone stage.
pub fn convert_negative(pixels: &[f32], w: usize, h: usize, params: &ProcessParams) -> Vec<f32> {
    let p = &params.conversion;
    let profile = &params.profile;

    // Film base: use override, or auto-estimate later injected by the caller
    // (in process() we call estimate_film_base before this stage).
    // Here the base comes through ConversionParams::film_base (already set).
    let base = p.film_base.unwrap_or(profile.base_rgb);
    let base = [base[0].max(0.001), base[1].max(0.001), base[2].max(0.001)];

    let n = pixels.len() / 3;
    let mut density = vec![0.0f32; n * 3];

    // Step 1: density above base D_c = log10(b_c / T_c), clamped >= 0.
    for i in 0..n {
        for c in 0..3 {
            let t = pixels[i * 3 + c].max(0.0001);
            let d = (base[c] / t).log10().max(0.0);
            density[i * 3 + c] = d;
        }
    }

    // Step 2: dye-gamma alignment (CCR) — scale each channel by profile.gamma
    // so all three share one neutral axis.
    if p.auto_ccr {
        let g = profile.gamma;
        for i in 0..n {
            density[i * 3]     = density[i * 3].powf(g[0]);
            density[i * 3 + 1] = density[i * 3 + 1].powf(g[1]);
            density[i * 3 + 2] = density[i * 3 + 2].powf(g[2]);
        }
    }

    // Step 3: D_max per channel from a high percentile of the IMAGE.
    //
    // The scan window is wider than the film frame, so its edges are the
    // black surround -- about 11% of the pixels on this scanner. Those are
    // the densest thing in the buffer by a wide margin (transmission near
    // zero gives density near the 1e-4 clamp, about 3.7), so taking the
    // percentile over the whole buffer measured the surround rather than
    // the picture and squashed the real image into the bottom fifth of the
    // range. Sample the interior only, the same region estimate_film_base()
    // treats as "not border".
    let tol = p.tolerance;
    let d_max = interior_percentile(&density, w, h, 1.0 - tol);

    // Step 4: exposure offset (stops).
    let exp_scale = 2.0f32.powf(p.exposure);

    // Step 5: normalise density to [0,1] and apply a simple S-curve (toe+shoulder).
    let toe = profile.curve_toe;
    let shoulder = profile.curve_shoulder;

    let mut out = vec![0.0f32; n * 3];
    for i in 0..n {
        for c in 0..3 {
            // Floor at 0.3 density units: prevents noise amplification when
            // the film base estimate is wrong or the image has very low contrast.
            let dmax = d_max[c].max(0.3);
            let dn = (density[i * 3 + c] / dmax * exp_scale).clamp(0.0, 1.0);
            out[i * 3 + c] = scurve(dn, toe, shoulder);
        }
    }
    out
}

/// Stage 4 — positive normalisation (slides / direct positives).
/// Scales each channel to [0,1] using per-channel min/max.
pub fn normalise_positive(pixels: &[f32], _params: &ProcessParams) -> Vec<f32> {
    let n = pixels.len() / 3;
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for i in 0..n {
        for c in 0..3 {
            let v = pixels[i * 3 + c];
            min[c] = min[c].min(v);
            max[c] = max[c].max(v);
        }
    }
    let mut out = vec![0.0f32; n * 3];
    for i in 0..n {
        for c in 0..3 {
            let range = (max[c] - min[c]).max(0.001);
            out[i * 3 + c] = ((pixels[i * 3 + c] - min[c]) / range).clamp(0.0, 1.0);
        }
    }
    out
}

/// S-curve with configurable toe and shoulder fractions.
/// toe/shoulder in [0, 0.5]; sharpens the midtones, compresses extremes.
fn scurve(t: f32, toe: f32, shoulder: f32) -> f32 {
    // Piecewise: linear toe, cubic mid, linear shoulder.
    if t < toe {
        // Slope 0.5: the toe compresses the shadows and meets the cubic
        // segment exactly at toe*0.5, which is where the mid branch starts.
        // This was `t * 0.5 / toe`, slope 4.17 for the default toe of 0.12 --
        // it expanded the shadows instead of compressing them, and jumped
        // from 0.50 to 0.06 across t = toe. Pixels either side of that edge
        // landed eight stops apart, which is what put the colour speckle
        // through the dark parts of every frame.
        t * 0.5
    } else if t > 1.0 - shoulder {
        let s = (t - (1.0 - shoulder)) / shoulder;
        1.0 - shoulder * 0.5 + s * shoulder * 0.5
    } else {
        // Smooth cubic between toe and shoulder.
        let lo = toe;
        let hi = 1.0 - shoulder;
        let range = hi - lo;
        let x = (t - lo) / range;
        // Hermite smoothstep: 3x²-2x³
        let smooth = x * x * (3.0 - 2.0 * x);
        // Map [0,1] → [toe*0.5, 1.0-shoulder*0.5]
        let out_lo = toe * 0.5;
        let out_hi = 1.0 - shoulder * 0.5;
        out_lo + smooth * (out_hi - out_lo)
    }
}

/// Per-channel `p`-th percentile (p in [0,1]) over the interior of the frame,
/// excluding the outer twelfth on each side -- the film rebate and the black
/// surround beyond it, which are not picture.
fn interior_percentile(pixels: &[f32], w: usize, h: usize, p: f32) -> [f32; 3] {
    let bw = (w / 12).max(4);
    let bh = (h / 12).max(4);
    let mut ch: [Vec<f32>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    if w > 2 * bw && h > 2 * bh {
        for y in bh..h - bh {
            for x in bw..w - bw {
                let i = y * w + x;
                for c in 0..3 {
                    ch[c].push(pixels[i * 3 + c]);
                }
            }
        }
    } else {
        // Frame too small to crop: fall back to every pixel.
        for i in 0..pixels.len() / 3 {
            for c in 0..3 {
                ch[c].push(pixels[i * 3 + c]);
            }
        }
    }
    let mut result = [0.0f32; 3];
    for c in 0..3 {
        ch[c].sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
        let idx = ((ch[c].len() as f32 * p) as usize).min(ch[c].len().saturating_sub(1));
        result[c] = ch[c][idx];
    }
    result
}

#[cfg(test)]
mod tests {
    use super::scurve;

    const TOE: f32 = 0.12;
    const SHOULDER: f32 = 0.18;

    /// The three segments have to meet. They did not: the toe ended at 0.5
    /// while the cubic started at 0.06, so pixels a hair either side of
    /// t = toe came out eight stops apart and every shadow was full of
    /// colour speckle.
    #[test]
    fn segments_join_without_a_step() {
        let below = scurve(TOE - 1e-4, TOE, SHOULDER);
        let above = scurve(TOE + 1e-4, TOE, SHOULDER);
        assert!((below - above).abs() < 1e-3, "toe join: {below} vs {above}");

        let below = scurve(1.0 - SHOULDER - 1e-4, TOE, SHOULDER);
        let above = scurve(1.0 - SHOULDER + 1e-4, TOE, SHOULDER);
        assert!((below - above).abs() < 1e-3, "shoulder join: {below} vs {above}");
    }

    /// A toe compresses the shadows; it must not amplify them.
    #[test]
    fn the_toe_compresses() {
        let t = TOE * 0.5;
        assert!(scurve(t, TOE, SHOULDER) < t, "toe should darken, not lift");
    }

    #[test]
    fn monotonic_over_the_whole_range() {
        let mut prev = -1.0f32;
        for i in 0..=1000 {
            let v = scurve(i as f32 / 1000.0, TOE, SHOULDER);
            assert!(v >= prev - 1e-6, "not monotonic at t={}", i as f32 / 1000.0);
            prev = v;
        }
    }

    #[test]
    fn endpoints_are_sane() {
        assert!(scurve(0.0, TOE, SHOULDER).abs() < 1e-6);
        assert!(scurve(1.0, TOE, SHOULDER) <= 1.0);
    }
}
