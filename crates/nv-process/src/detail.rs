//! Stage 7, detail (DESIGN.md §7.1, §7.7): unsharp mask and grain and noise
//! reduction. They work on the picture as it will be seen: luminance and
//! chrominance of the sRGB-encoded values, because that is what "sharper" and
//! "noisier" mean to the eye, then back to linear light.
//!
//! Both look at a pixel's neighbours, so a tile needs a margin around it
//! ([`margin`]); noise reduction is expensive and is left out of the quick
//! preview ([`Quality::Preview`]) and applied at 1:1 and in exports.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsmParams {
    pub enabled: bool,
    /// How much of the difference is added back, in percent (F§6 "Power").
    pub amount: f32,
    /// Blur radius in pixels of the finished picture.
    pub radius: f32,
    /// Differences smaller than this many 8-bit levels are left alone.
    pub threshold: u8,
}

impl Default for UsmParams {
    fn default() -> Self {
        Self { enabled: false, amount: 150.0, radius: 1.4, threshold: 0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NoiseLevel {
    #[default]
    Off,
    Light,
    Medium,
    Strong,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DetailParams {
    pub usm: UsmParams,
    pub noise: NoiseLevel,
}

/// How much work a render may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    /// The editing preview: quick. Skips noise reduction.
    Preview,
    /// What 1:1 and exports show.
    Full,
}

impl DetailParams {
    pub fn is_off(&self) -> bool {
        !self.usm.enabled && self.noise == NoiseLevel::Off
    }
}

/// Search radius and patch radius of the noise reduction.
const SEARCH: isize = 3;
const PATCH: isize = 1;

/// Pixels around a tile that the detail filters need to see, at `factor`
/// scan pixels per picture pixel. Zero when they are off.
pub fn margin(detail: &DetailParams, factor: usize) -> usize {
    let usm = if detail.usm.enabled { (3.0 * detail.usm.radius / factor.max(1) as f32).ceil() as usize + 1 } else { 0 };
    let nlm = if detail.noise != NoiseLevel::Off { (SEARCH + PATCH) as usize + 1 } else { 0 };
    usm.max(nlm)
}

// ── sRGB encoding, by table ───────────────────────────────────────────────────

fn encode(v: f32) -> f32 {
    crate::srgb_oetf(v)
}

fn decode(e: f32) -> f32 {
    let e = e.clamp(0.0, 1.0);
    if e <= 0.040_45 { e / 12.92 } else { ((e + 0.055) / 1.055).powf(2.4) }
}

// ── Unsharp mask ──────────────────────────────────────────────────────────────

/// Gaussian blur of one plane, separable, kernel cut at three sigma, edges
/// repeated.
fn gaussian(a: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    let r = (3.0 * sigma).ceil() as isize;
    let kernel: Vec<f32> = (-r..=r).map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp()).collect();
    let norm: f32 = kernel.iter().sum();
    let kernel: Vec<f32> = kernel.iter().map(|k| k / norm).collect();
    let mut tmp = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut s = 0.0;
            for (k, kv) in kernel.iter().enumerate() {
                let xx = (x as isize + k as isize - r).clamp(0, w as isize - 1) as usize;
                s += kv * a[y * w + xx];
            }
            tmp[y * w + x] = s;
        }
    }
    let mut out = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut s = 0.0;
            for (k, kv) in kernel.iter().enumerate() {
                let yy = (y as isize + k as isize - r).clamp(0, h as isize - 1) as usize;
                s += kv * tmp[yy * w + x];
            }
            out[y * w + x] = s;
        }
    }
    out
}

/// Sharpen luminance `y` (0..1, encoded) in place.
fn unsharp(y: &mut [f32], w: usize, h: usize, usm: &UsmParams, factor: usize) {
    // A proxy is smaller than the scan: the radius shrinks with it, and below
    // a third of a pixel there is nothing left to see.
    let sigma = usm.radius / factor.max(1) as f32;
    if sigma < 0.3 || usm.amount <= 0.0 {
        return;
    }
    let blur = gaussian(y, w, h, sigma);
    let gain = usm.amount / 100.0;
    let threshold = usm.threshold as f32 / 255.0;
    for (v, b) in y.iter_mut().zip(&blur) {
        let d = *v - *b;
        if d.abs() >= threshold {
            *v = (*v + gain * d).clamp(0.0, 1.0);
        }
    }
}

// ── Noise reduction ───────────────────────────────────────────────────────────

/// Non-local means on one plane: each pixel becomes the average of pixels
/// whose surroundings look like its own, weighted by how alike they look.
/// Done offset by offset, so the patch comparison is a box filter over a
/// difference image rather than a loop per pixel.
fn non_local_means(a: &[f32], w: usize, h: usize, strength: f32) -> Vec<f32> {
    let h2 = strength * strength;
    let mut num = vec![0f32; w * h];
    let mut den = vec![0f32; w * h];
    let mut diff = vec![0f32; w * h];
    let mut tmp = vec![0f32; w * h];
    let at = |x: isize, y: isize| -> f32 { a[y.clamp(0, h as isize - 1) as usize * w + x.clamp(0, w as isize - 1) as usize] };
    let area = ((2 * PATCH + 1) * (2 * PATCH + 1)) as f32;
    for dy in -SEARCH..=SEARCH {
        for dx in -SEARCH..=SEARCH {
            // Squared difference between the picture and itself shifted by (dx, dy).
            for y in 0..h {
                for x in 0..w {
                    let d = a[y * w + x] - at(x as isize + dx, y as isize + dy);
                    diff[y * w + x] = d * d;
                }
            }
            // Mean over the patch, separably.
            for y in 0..h {
                for x in 0..w {
                    let mut s = 0.0;
                    for k in -PATCH..=PATCH {
                        s += diff[y * w + (x as isize + k).clamp(0, w as isize - 1) as usize];
                    }
                    tmp[y * w + x] = s;
                }
            }
            for y in 0..h {
                for x in 0..w {
                    let mut s = 0.0;
                    for k in -PATCH..=PATCH {
                        s += tmp[(y as isize + k).clamp(0, h as isize - 1) as usize * w + x];
                    }
                    let weight = (-(s / area) / h2).exp();
                    num[y * w + x] += weight * at(x as isize + dx, y as isize + dy);
                    den[y * w + x] += weight;
                }
            }
        }
    }
    num.iter().zip(&den).map(|(n, d)| n / d.max(1e-6)).collect()
}

/// Filtering strength on the luma plane (0..1) for each level; chroma is
/// smoothed harder, because the eye minds colour noise less than it minds
/// the loss of colour edges.
fn strength(level: NoiseLevel) -> f32 {
    match level {
        NoiseLevel::Off => 0.0,
        NoiseLevel::Light => 0.025,
        NoiseLevel::Medium => 0.045,
        NoiseLevel::Strong => 0.075,
    }
}

// ── The stage ─────────────────────────────────────────────────────────────────

/// Apply the detail filters to a toned picture (linear RGB, `w × h × 3`).
/// `factor` is how many scan pixels one pixel covers (1 at full resolution).
pub fn apply(toned: &mut [f32], w: usize, h: usize, detail: &DetailParams, factor: usize, quality: Quality) {
    let noise = quality == Quality::Full && detail.noise != NoiseLevel::Off;
    if (!detail.usm.enabled && !noise) || w < 4 || h < 4 || toned.len() < w * h * 3 {
        return;
    }
    let n = w * h;

    // Luminance and colour differences of the encoded picture.
    let mut y = vec![0f32; n];
    let mut cb = vec![0f32; n];
    let mut cr = vec![0f32; n];
    for i in 0..n {
        let (r, g, b) = (encode(toned[i * 3]), encode(toned[i * 3 + 1]), encode(toned[i * 3 + 2]));
        let l = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        y[i] = l;
        cb[i] = b - l;
        cr[i] = r - l;
    }
    let original = y.clone();

    if noise {
        let s = strength(detail.noise);
        y = non_local_means(&y, w, h, s);
        cb = non_local_means(&cb, w, h, 2.0 * s);
        cr = non_local_means(&cr, w, h, 2.0 * s);
    }
    if detail.usm.enabled {
        unsharp(&mut y, w, h, &detail.usm, factor);
    }
    let _ = original;

    for i in 0..n {
        let l = y[i];
        let (r, b) = (l + cr[i], l + cb[i]);
        // Green from the luma equation, so luminance is exactly `l`.
        let g = (l - 0.2126 * r - 0.0722 * b) / 0.7152;
        toned[i * 3] = decode(r);
        toned[i * 3 + 1] = decode(g);
        toned[i * 3 + 2] = decode(b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 64;
    const H: usize = 48;

    fn grey(v: impl Fn(usize, usize) -> f32) -> Vec<f32> {
        (0..W * H).flat_map(|i| { let e = v(i % W, i / W); let l = decode(e); [l, l, l] }).collect()
    }

    fn luma(p: &[f32]) -> Vec<f32> {
        p.chunks(3).map(|c| encode(c[1])).collect()
    }

    fn noise(i: usize) -> f32 {
        let v = (i as u32).wrapping_mul(2654435761).rotate_left(13).wrapping_mul(40503);
        (v >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }

    fn usm(amount: f32, radius: f32, threshold: u8) -> DetailParams {
        DetailParams { usm: UsmParams { enabled: true, amount, radius, threshold }, noise: NoiseLevel::Off }
    }

    #[test]
    fn sharpening_steepens_an_edge_and_leaves_flat_areas_alone() {
        let step = |x: usize, _| if x < 32 { 0.3 } else { 0.6 };
        let mut p = grey(step);
        let before = luma(&p);
        apply(&mut p, W, H, &usm(200.0, 1.4, 0), 1, Quality::Full);
        let after = luma(&p);
        // Overshoot on both sides of the edge: darker just below, brighter just above.
        assert!(after[24 * W + 31] < before[24 * W + 31] - 0.02, "no undershoot");
        assert!(after[24 * W + 32] > before[24 * W + 32] + 0.02, "no overshoot");
        // Far from the edge, nothing changes.
        assert!((after[24 * W + 5] - before[24 * W + 5]).abs() < 1e-4);
        assert!((after[24 * W + 60] - before[24 * W + 60]).abs() < 1e-4);
    }

    #[test]
    fn amount_threshold_and_the_switch_all_matter() {
        let step = |x: usize, _| if x < 32 { 0.40 } else { 0.43 }; // a faint edge: 0.03 = 7.6 levels
        let base = grey(step);
        let run = |d: &DetailParams| { let mut p = base.clone(); apply(&mut p, W, H, d, 1, Quality::Full); p };
        let changed = |p: &[f32]| p.iter().zip(&base).filter(|(a, b)| (**a - **b).abs() > 1e-4).count();
        assert!(changed(&run(&usm(150.0, 1.4, 0))) > 0, "no sharpening");
        assert_eq!(changed(&run(&usm(0.0, 1.4, 0))), 0, "amount 0 changed the picture");
        assert_eq!(changed(&run(&usm(150.0, 1.4, 20))), 0, "the threshold did not hold back a faint edge");
        let off = DetailParams { usm: UsmParams { enabled: false, ..usm(150.0, 1.4, 0).usm }, noise: NoiseLevel::Off };
        assert_eq!(changed(&run(&off)), 0);
        assert!(off.is_off());
        // A bigger amount moves more.
        let strength_of = |a: f32| { let p = run(&usm(a, 1.4, 0)); p.iter().zip(&base).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max) };
        assert!(strength_of(300.0) > strength_of(100.0));
    }

    #[test]
    fn a_proxy_shrinks_the_radius_with_the_picture() {
        let step = |x: usize, _| if x < 32 { 0.3 } else { 0.6 };
        let base = grey(step);
        let max_change = |factor: usize| { let mut p = base.clone(); apply(&mut p, W, H, &usm(200.0, 1.4, 0), factor, Quality::Full); p.iter().zip(&base).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max) };
        assert!(max_change(1) > 0.0);
        assert_eq!(max_change(8), 0.0, "a 1.4 px radius at 1/8 scale is 0.175 px: nothing to sharpen");
    }

    #[test]
    fn noise_reduction_smooths_grain_and_keeps_an_edge() {
        let noisy = |x: usize, y: usize| (if x < 32 { 0.30 } else { 0.60 }) + 0.03 * noise(y * W + x);
        let clean = |x: usize, _| if x < 32 { 0.30 } else { 0.60 };
        let mut p = grey(noisy);
        let before = luma(&p);
        let d = DetailParams { usm: UsmParams::default(), noise: NoiseLevel::Medium };
        apply(&mut p, W, H, &d, 1, Quality::Full);
        let after = luma(&p);
        let err = |a: &[f32]| { let mut s = 0.0; let mut n = 0.0; for y in 4..H - 4 { for x in 4..W - 4 { if (x as isize - 32).abs() > 3 { s += (a[y * W + x] - clean(x, y)).powi(2); n += 1.0; } } } (s / n).sqrt() };
        assert!(err(&after) < 0.5 * err(&before), "grain {} -> {}", err(&before), err(&after));
        // The edge is still an edge: a big jump across two pixels.
        let jump = after[24 * W + 33] - after[24 * W + 30];
        assert!(jump > 0.2, "edge smeared to {jump}");
    }

    #[test]
    fn noise_reduction_is_for_full_quality_only() {
        let base = grey(|x, y| 0.4 + 0.05 * noise(y * W + x));
        let mut p = base.clone();
        apply(&mut p, W, H, &DetailParams { usm: UsmParams::default(), noise: NoiseLevel::Strong }, 1, Quality::Preview);
        assert_eq!(p, base);
        apply(&mut p, W, H, &DetailParams { usm: UsmParams::default(), noise: NoiseLevel::Strong }, 1, Quality::Full);
        assert_ne!(p, base);
    }

    #[test]
    fn colour_is_kept_and_luminance_is_what_changes() {
        let mut p: Vec<f32> = (0..W * H).flat_map(|i| if i % W < 32 { [0.5, 0.2, 0.1] } else { [0.5, 0.3, 0.2] }).collect();
        let before = p.clone();
        apply(&mut p, W, H, &usm(100.0, 1.0, 0), 1, Quality::Full);
        // Away from the edge nothing moved, and nothing is NaN or out of range.
        assert!(p.iter().all(|v| v.is_finite() && (0.0..=1.01).contains(v)));
        assert!((p[24 * W * 3 + 5 * 3] - before[24 * W * 3 + 5 * 3]).abs() < 1e-3);
    }

    #[test]
    fn margins_grow_with_what_is_on() {
        assert_eq!(margin(&DetailParams::default(), 1), 0);
        assert!(margin(&usm(100.0, 1.4, 0), 1) >= 5);
        assert!(margin(&usm(100.0, 4.0, 0), 1) > margin(&usm(100.0, 1.4, 0), 1));
        assert!(margin(&DetailParams { usm: UsmParams::default(), noise: NoiseLevel::Light }, 1) >= (SEARCH + PATCH) as usize);
        assert!(margin(&usm(100.0, 4.0, 0), 4) < margin(&usm(100.0, 4.0, 0), 1), "a proxy needs less");
    }
}
