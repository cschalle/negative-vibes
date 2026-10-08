use crate::params::ProcessParams;

/// What a picture says about its own conversion: the range and the colour
/// axis. Roll analysis collects these over a roll and fixes their median.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measured {
    pub d_max: [f32; 3],
    /// `(offset, slope)` per channel; green is `(0, 1)`.
    pub cast_fit: [[f32; 2]; 3],
}

/// Density above base, profile gamma and cast alignment: stage 4 up to the
/// point where the range is measured. Also returns the cast fit used.
fn density_stage(pixels: &[f32], w: usize, h: usize, params: &ProcessParams) -> (Vec<f32>, [[f32; 2]; 3]) {
    let p = &params.conversion;
    let profile = &params.profile;

    // Film base: the override (set from the rebate, a pipette or roll
    // analysis), or the profile's.
    let base = p.film_base.unwrap_or(profile.base_rgb);
    let base = [base[0].max(0.001), base[1].max(0.001), base[2].max(0.001)];

    let n = pixels.len() / 3;
    let mut density = vec![0.0f32; n * 3];

    // Step 1: density above base D_c = log10(b_c / T_c), clamped >= 0.
    for i in 0..n {
        for c in 0..3 {
            let t = pixels[i * 3 + c].max(0.0001);
            density[i * 3 + c] = (base[c] / t).log10().max(0.0);
        }
    }

    // Step 2: dye-gamma alignment from the film profile.
    if p.auto_ccr {
        let g = profile.gamma;
        for i in 0..n {
            for c in 0..3 {
                density[i * 3 + c] = density[i * 3 + c].powf(g[c]);
            }
        }
    }

    // Step 2b: colour cast removal. Line red and blue density up with green
    // along the picture's own neutral axis (or the roll's), so neutral things
    // come out neutral through the midtones as well as at the ends.
    let fit = p.cast_fit.unwrap_or_else(|| {
        let f = cast_fit(&density, w, h, p.region);
        [[f[0].0, f[0].1], [f[1].0, f[1].1], [f[2].0, f[2].1]]
    });
    if p.cast_removal > 0.0 {
        let k = p.cast_removal.clamp(0.0, 1.0);
        for i in 0..n {
            for c in [0usize, 2] {
                let d = density[i * 3 + c];
                let aligned = (d - fit[c][0]) / fit[c][1];
                density[i * 3 + c] = (d + (aligned - d) * k).max(0.0);
            }
        }
    }
    (density, fit)
}

/// Measure a picture's own range and colour axis, ignoring any roll-wide
/// values in `params`.
pub fn measure_negative(pixels: &[f32], w: usize, h: usize, params: &ProcessParams) -> Measured {
    let mut own = params.clone();
    own.conversion.d_max = None;
    own.conversion.cast_fit = None;
    freeze_negative(pixels, w, h, &own)
}

/// The range and colour axis `params` convert with: the values it fixes, and
/// for the rest, those measured on this picture.
pub fn freeze_negative(pixels: &[f32], w: usize, h: usize, params: &ProcessParams) -> Measured {
    let mut p = params.clone();
    p.conversion.d_max = None;
    let (density, cast_fit) = density_stage(pixels, w, h, &p);
    let d_max = params
        .conversion
        .d_max
        .unwrap_or_else(|| interior_percentile(&density, w, h, 1.0 - params.conversion.tolerance, params.conversion.region));
    Measured { d_max, cast_fit }
}

/// Stage 4 — colour negative inversion (§7.4 of DESIGN.md).
///
/// Input:  linear f32 [0,1] transmission values (T_c).
/// Output: linear f32 [0,1] positive light values ready for tone stage.
pub fn convert_negative(pixels: &[f32], w: usize, h: usize, params: &ProcessParams) -> Vec<f32> {
    let p = &params.conversion;
    let profile = &params.profile;
    let (density, _) = density_stage(pixels, w, h, params);
    let n = pixels.len() / 3;

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
    let d_max = p.d_max.unwrap_or_else(|| interior_percentile(&density, w, h, 1.0 - tol, p.region));

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

/// The `(min, max)` per channel a positive is scaled by: the fixed range, or
/// the one found in `pixels`.
pub fn positive_range(pixels: &[f32], params: &ProcessParams) -> ([f32; 3], [f32; 3]) {
    if let Some(r) = params.conversion.positive_range {
        return ([r[0][0], r[1][0], r[2][0]], [r[0][1], r[1][1], r[2][1]]);
    }
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for px in pixels.chunks_exact(3) {
        for c in 0..3 {
            min[c] = min[c].min(px[c]);
            max[c] = max[c].max(px[c]);
        }
    }
    (min, max)
}

/// Stage 4 — positive normalisation (slides / direct positives).
/// Scales each channel to [0,1] using per-channel min/max.
pub fn normalise_positive(pixels: &[f32], params: &ProcessParams) -> Vec<f32> {
    let n = pixels.len() / 3;
    let (min, max) = positive_range(pixels, params);
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

/// Fit `D_c = a_c + s_c · D_g` for red and blue over the interior of the
/// picture, using the middle 80% of green density (the ends are clipped
/// highlights and shadows, which are not on the neutral axis). Returns
/// `(a, s)` per channel; green is the reference and the identity.
///
/// Least squares on a natural scene follows the dominant axis, which is
/// brightness: most of the picture is closer to neutral than to any one
/// colour. Slopes outside 0.5–2 are not film, they are a failed fit, and
/// are not applied.
fn cast_fit(density: &[f32], w: usize, h: usize, region: Option<[f32; 4]>) -> [(f32, f32); 3] {
    let identity = [(0.0, 1.0); 3];
    let (x0, x1, y0, y1) = interior(w, h, region);

    // Green's 10th and 90th percentiles, from a histogram.
    let scale = DENSITY_BINS as f32 / DENSITY_RANGE;
    let mut hist = vec![0u32; DENSITY_BINS];
    for y in y0..y1 {
        for x in x0..x1 {
            hist[((density[(y * w + x) * 3 + 1] * scale) as usize).min(DENSITY_BINS - 1)] += 1;
        }
    }
    let total: u32 = hist.iter().sum();
    if total < 100 {
        return identity;
    }
    let at = |frac: f32| -> f32 {
        let target = (total as f32 * frac) as u32;
        let mut seen = 0;
        for (i, n) in hist.iter().enumerate() {
            seen += n;
            if seen >= target.max(1) {
                return i as f32 / scale;
            }
        }
        DENSITY_RANGE
    };
    let (lo, hi) = (at(0.10), at(0.90));
    if hi - lo < 0.05 {
        return identity;
    }

    // Sums in f64: millions of values near 1 lose digits in f32.
    let (mut n, mut sg, mut sgg) = (0f64, 0f64, 0f64);
    let (mut sc, mut sgc) = ([0f64; 3], [0f64; 3]);
    for y in y0..y1 {
        for x in x0..x1 {
            let i = (y * w + x) * 3;
            let g = density[i + 1];
            if g < lo || g > hi {
                continue;
            }
            let g = g as f64;
            n += 1.0;
            sg += g;
            sgg += g * g;
            for c in [0usize, 2] {
                let v = density[i + c] as f64;
                sc[c] += v;
                sgc[c] += g * v;
            }
        }
    }
    let var = sgg / n - (sg / n) * (sg / n);
    if n < 100.0 || var < 1e-6 {
        return identity;
    }
    let mut fit = identity;
    for c in [0usize, 2] {
        let slope = (sgc[c] / n - (sg / n) * (sc[c] / n)) / var;
        if (0.5..=2.0).contains(&slope) {
            fit[c] = ((sc[c] / n - slope * sg / n) as f32, slope as f32);
        }
    }
    fit
}

/// Pixel bounds `(x0, x1, y0, y1)` of the part of the image to measure: the
/// frame's region with a tenth of it trimmed off every side, or, with no
/// region, the whole image less its outer twelfth (rebate and holder).
pub(crate) fn interior(w: usize, h: usize, region: Option<[f32; 4]>) -> (usize, usize, usize, usize) {
    let (x0, x1, y0, y1) = match region {
        Some([a, b, c, d]) => {
            let (rw, rh) = ((c - a).max(0.0), (d - b).max(0.0));
            (a + rw * 0.1, c - rw * 0.1, b + rh * 0.1, d - rh * 0.1)
        }
        None => (1.0 / 12.0, 11.0 / 12.0, 1.0 / 12.0, 11.0 / 12.0),
    };
    let px = |f: f32, n: usize| ((f.clamp(0.0, 1.0) * n as f32).round() as usize).min(n);
    let (ix0, ix1, iy0, iy1) = (px(x0, w), px(x1, w), px(y0, h), px(y1, h));
    // Too small to measure (a tiny proxy or a sliver): use everything.
    if ix1 < ix0 + 4 || iy1 < iy0 + 4 { (0, w, 0, h) } else { (ix0, ix1, iy0, iy1) }
}

/// Largest density the histogram resolves; transmission is clamped at 1e-4,
/// which is density 4.
const DENSITY_RANGE: f32 = 4.5;
const DENSITY_BINS: usize = 4096;

/// Per-channel `p`-th percentile (p in [0,1]) over the interior of the frame,
/// excluding the outer twelfth on each side -- the film rebate and the black
/// surround beyond it, which are not picture.
///
/// Uses a histogram rather than a sort: this runs on every parameter change,
/// and the answer is only needed to a thousandth of a density unit.
fn interior_percentile(pixels: &[f32], w: usize, h: usize, p: f32, region: Option<[f32; 4]>) -> [f32; 3] {
    let (x0, x1, y0, y1) = interior(w, h, region);

    let mut hist = vec![[0u32; 3]; DENSITY_BINS];
    let scale = DENSITY_BINS as f32 / DENSITY_RANGE;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = (y * w + x) * 3;
            for c in 0..3 {
                let bin = ((pixels[i + c] * scale) as usize).min(DENSITY_BINS - 1);
                hist[bin][c] += 1;
            }
        }
    }

    let total = ((x1 - x0) * (y1 - y0)) as f32;
    let target = (total * p).ceil() as u32;
    let mut result = [0.0f32; 3];
    for c in 0..3 {
        let mut seen = 0u32;
        for (bin, counts) in hist.iter().enumerate() {
            seen += counts[c];
            if seen >= target.max(1) {
                result[c] = (bin as f32 + 1.0) / scale;
                break;
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::scurve;
    use crate::params::{ConversionParams, ImageType, ProcessParams, ToneParams};
    use nv_film::FilmProfile;

    /// A grey ramp in green density, with red and blue sitting on lines of
    /// their own: the cast of a scan whose film base was a little off.
    fn cast_ramp(w: usize, h: usize) -> (Vec<f32>, [f32; 3]) {
        let base = [0.6f32, 0.3, 0.18];
        let mut px = Vec::new();
        for _y in 0..h {
            for x in 0..w {
                let dg = 0.1 + 1.6 * x as f32 / w as f32;
                let d = [0.30 + 1.35 * dg, dg, 0.10 + 0.70 * dg];
                for c in 0..3 {
                    px.push(base[c] * 10f32.powf(-d[c].max(0.0)));
                }
            }
        }
        (px, base)
    }

    fn params(base: [f32; 3], cast_removal: f32) -> ProcessParams {
        ProcessParams {
            image_type: ImageType::ColourNegative,
            profile: FilmProfile::identity(),
            conversion: ConversionParams { film_base: Some(base), exposure: 0.0, tolerance: 0.0016, auto_ccr: false, cast_removal, d_max: None, cast_fit: None, region: None, positive_range: None },
            tone: ToneParams::default(),
        }
    }

    fn spread(out: &[f32], w: usize, h: usize) -> f32 {
        // Largest channel difference over the middle of the ramp.
        let mut worst = 0f32;
        for y in h / 4..h * 3 / 4 {
            for x in w / 4..w * 3 / 4 {
                let p = &out[(y * w + x) * 3..][..3];
                worst = worst.max((p[0] - p[1]).abs()).max((p[2] - p[1]).abs());
            }
        }
        worst
    }

    #[test]
    fn cast_removal_makes_a_biased_ramp_neutral() {
        let (w, h) = (240, 60);
        let (px, base) = cast_ramp(w, h);
        let off = super::convert_negative(&px, w, h, &params(base, 0.0));
        let on = super::convert_negative(&px, w, h, &params(base, 1.0));
        assert!(spread(&off, w, h) > 0.08, "test image has no cast: {}", spread(&off, w, h));
        assert!(spread(&on, w, h) < 0.02, "cast left: {}", spread(&on, w, h));
    }

    #[test]
    fn cast_removal_is_continuous_in_its_strength() {
        let (w, h) = (240, 60);
        let (px, base) = cast_ramp(w, h);
        let s = |k| spread(&super::convert_negative(&px, w, h, &params(base, k)), w, h);
        assert!(s(0.0) > s(0.5) && s(0.5) > s(1.0), "{} {} {}", s(0.0), s(0.5), s(1.0));
    }

    #[test]
    fn a_flat_picture_is_left_alone() {
        let (w, h) = (60, 60);
        let px = vec![0.3f32; w * h * 3];
        let a = super::convert_negative(&px, w, h, &params([0.6, 0.3, 0.18], 0.0));
        let b = super::convert_negative(&px, w, h, &params([0.6, 0.3, 0.18], 1.0));
        assert_eq!(a, b);
    }

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
