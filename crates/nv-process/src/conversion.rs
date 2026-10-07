use crate::params::ProcessParams;

/// Stage 4 — colour negative inversion (§7.4 of DESIGN.md).
///
/// Input:  linear f32 [0,1] transmission values (T_c).
/// Output: linear f32 [0,1] positive light values ready for tone stage.
pub fn convert_negative(pixels: &[f32], params: &ProcessParams) -> Vec<f32> {
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

    // Step 3: D_max per channel from high percentile (clamp white point).
    let tol = p.tolerance;
    let d_max = channel_percentile(&density, 1.0 - tol);

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
        t * 0.5 / toe
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

/// Per-channel `p`-th percentile value (p in [0,1]).
fn channel_percentile(pixels: &[f32], p: f32) -> [f32; 3] {
    let n = pixels.len() / 3;
    let mut ch: [Vec<f32>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for i in 0..n {
        for c in 0..3 {
            ch[c].push(pixels[i * 3 + c]);
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
