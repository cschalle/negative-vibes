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
