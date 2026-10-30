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
    /// Local contrast (F§6 AACO), the effect in percent: 0 is off.
    #[serde(default)]
    pub local_contrast: f32,
    /// The equalisation grid, measured once on the whole picture so that a
    /// tile, the loupe and the preview all use the same one. Not saved: it is
    /// measured again when needed.
    #[serde(skip)]
    pub aaco: Option<AacoGrid>,
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
        !self.usm.enabled && self.noise == NoiseLevel::Off && self.local_contrast <= 0.0
    }
}

/// Where a block sits in its scan, for things measured over the whole scan.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub origin: (usize, usize),
    pub full: (usize, usize),
}

/// Cells across and down, and bins per cell histogram.
const GRID: usize = 8;
const BINS: usize = 64;
/// Histograms are clipped at this many times the flat height, which is what
/// keeps adaptive equalisation from turning noise into texture.
const CLIP: f32 = 3.0;

/// A lookup table for each cell of an 8 × 8 grid over the picture: the
/// cumulative histogram of its brightness, clipped.
#[derive(Debug, Clone, PartialEq)]
pub struct AacoGrid {
    luts: Vec<f32>,
}

impl AacoGrid {
    /// Measure `y` (encoded luminance, `w × h`).
    pub fn measure(y: &[f32], w: usize, h: usize) -> Self {
        let mut luts = vec![0f32; GRID * GRID * BINS];
        for gy in 0..GRID {
            for gx in 0..GRID {
                let (x0, x1) = (gx * w / GRID, (gx + 1) * w / GRID);
                let (y0, y1) = (gy * h / GRID, (gy + 1) * h / GRID);
                let mut hist = [0f32; BINS];
                let mut n = 0f32;
                for yy in y0..y1.max(y0 + 1).min(h) {
                    for xx in x0..x1.max(x0 + 1).min(w) {
                        hist[((y[yy * w + xx].clamp(0.0, 1.0) * (BINS - 1) as f32) + 0.5) as usize] += 1.0;
                        n += 1.0;
                    }
                }
                let lut = &mut luts[(gy * GRID + gx) * BINS..][..BINS];
                if n == 0.0 {
                    for (b, v) in lut.iter_mut().enumerate() {
                        *v = b as f32 / (BINS - 1) as f32;
                    }
                    continue;
                }
                // Clip, and give what was cut back to every bin.
                let limit = CLIP * n / BINS as f32;
                let excess: f32 = hist.iter().map(|c| (c - limit).max(0.0)).sum();
                let share = excess / BINS as f32;
                let mut acc = 0f32;
                for b in 0..BINS {
                    acc += hist[b].min(limit) + share;
                    lut[b] = acc / n;
                }
            }
        }
        Self { luts }
    }

    /// The equalised value of brightness `v` at fractional grid position
    /// `(gx, gy)` (cell centres at whole numbers), interpolated between the
    /// four cells around it and between bins.
    fn map(&self, v: f32, gx: f32, gy: f32) -> f32 {
        let (gx, gy) = (gx.clamp(0.0, (GRID - 1) as f32), gy.clamp(0.0, (GRID - 1) as f32));
        let (x0, y0) = (gx.floor() as usize, gy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(GRID - 1), (y0 + 1).min(GRID - 1));
        let (tx, ty) = (gx - x0 as f32, gy - y0 as f32);
        let f = v.clamp(0.0, 1.0) * (BINS - 1) as f32;
        let (b0, tb) = ((f.floor() as usize).min(BINS - 1), f - f.floor());
        let b1 = (b0 + 1).min(BINS - 1);
        let cell = |cx: usize, cy: usize| {
            let l = &self.luts[(cy * GRID + cx) * BINS..][..BINS];
            l[b0] * (1.0 - tb) + l[b1] * tb
        };
        let top = cell(x0, y0) * (1.0 - tx) + cell(x1, y0) * tx;
        let bottom = cell(x0, y1) * (1.0 - tx) + cell(x1, y1) * tx;
        top * (1.0 - ty) + bottom * ty
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

/// How much of the grain each level puts back after the filter has run.
///
/// Non-local means on flat film grain collapses toward a plain window
/// average: inside the 7x7 search every 3x3 patch looks like every other
/// one, so the weights flatten and nothing is left to tell grain from
/// grain. Measured at 3600 dpi on a flat wall, `Light` alone took 87% of
/// the high-frequency energy, against 11% for SilverFast's Light GANE and
/// 17% for its Strong -- our lightest setting was past their heaviest, and
/// the wall came out waxy with the scratches smeared.
///
/// Lowering the strength does not fix that. At 0.025 it is already 0.76x
/// the grain sigma measured on the same patch, which is conservative for
/// non-local means; the collapse is in the flatness of the region, not in
/// the constant. So run the filter and then put a measured share of the
/// grain back, which is what keeps the picture looking like film.
fn grain_kept(level: NoiseLevel) -> f32 {
    match level {
        NoiseLevel::Off => 1.0,
        NoiseLevel::Light => 0.92,
        NoiseLevel::Medium => 0.74,
        NoiseLevel::Strong => 0.51,
    }
}

/// Measure the grid from a toned picture (linear RGB, `w × h × 3`).
pub fn measure_grid(toned: &[f32], w: usize, h: usize) -> AacoGrid {
    let y: Vec<f32> = toned.chunks_exact(3).map(|p| 0.2126 * encode(p[0]) + 0.7152 * encode(p[1]) + 0.0722 * encode(p[2])).collect();
    AacoGrid::measure(&y, w, h)
}

// ── The stage ─────────────────────────────────────────────────────────────────

/// Apply the detail filters to a toned picture (linear RGB, `w × h × 3`).
/// `factor` is how many scan pixels one pixel covers (1 at full resolution).
pub fn apply(toned: &mut [f32], w: usize, h: usize, detail: &DetailParams, factor: usize, quality: Quality, place: Placement) {
    let noise = quality == Quality::Full && detail.noise != NoiseLevel::Off;
    let local = detail.local_contrast > 0.0;
    if (!detail.usm.enabled && !noise && !local) || w < 4 || h < 4 || toned.len() < w * h * 3 {
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
        let kept = grain_kept(detail.noise);
        let filtered = non_local_means(&y, w, h, s);
        for i in 0..n {
            y[i] = filtered[i] + kept * (original[i] - filtered[i]);
        }
        // Colour keeps none of it: chroma speckle is the one kind of noise
        // the eye does mind, and there is no texture in it worth saving.
        cb = non_local_means(&cb, w, h, 2.0 * s);
        cr = non_local_means(&cr, w, h, 2.0 * s);
    }
    if local {
        // Measured on this picture unless a whole-picture grid was given.
        let own;
        let grid = match &detail.aaco {
            Some(g) => g,
            None => {
                own = AacoGrid::measure(&y, w, h);
                &own
            }
        };
        let effect = (detail.local_contrast / 100.0).clamp(0.0, 1.0);
        let f = factor.max(1) as f32;
        for yy in 0..h {
            for xx in 0..w {
                // The cell centres are at whole numbers of a grid laid over the whole scan.
                let gx = (place.origin.0 as f32 + (xx as f32 + 0.5) * f) / place.full.0.max(1) as f32 * GRID as f32 - 0.5;
                let gy = (place.origin.1 as f32 + (yy as f32 + 0.5) * f) / place.full.1.max(1) as f32 * GRID as f32 - 0.5;
                let v = y[yy * w + xx];
                y[yy * w + xx] = v + effect * (grid.map(v, gx, gy) - v);
            }
        }
    }
    if detail.usm.enabled {
        unsharp(&mut y, w, h, &detail.usm, factor);
    }

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
    const WHOLE: Placement = Placement { origin: (0, 0), full: (W, H) };

    fn grey(v: impl Fn(usize, usize) -> f32) -> Vec<f32> {
        (0..W * H)
            .flat_map(|i| {
                let e = v(i % W, i / W);
                let l = decode(e);
                [l, l, l]
            })
            .collect()
    }

    fn luma(p: &[f32]) -> Vec<f32> {
        p.chunks(3).map(|c| encode(c[1])).collect()
    }

    fn noise(i: usize) -> f32 {
        let v = (i as u32).wrapping_mul(2654435761).rotate_left(13).wrapping_mul(40503);
        (v >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }

    fn usm(amount: f32, radius: f32, threshold: u8) -> DetailParams {
        DetailParams { usm: UsmParams { enabled: true, amount, radius, threshold }, ..Default::default() }
    }

    #[test]
    fn sharpening_steepens_an_edge_and_leaves_flat_areas_alone() {
        let step = |x: usize, _| if x < 32 { 0.3 } else { 0.6 };
        let mut p = grey(step);
        let before = luma(&p);
        apply(&mut p, W, H, &usm(200.0, 1.4, 0), 1, Quality::Full, WHOLE);
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
        let run = |d: &DetailParams| {
            let mut p = base.clone();
            apply(&mut p, W, H, d, 1, Quality::Full, WHOLE);
            p
        };
        let changed = |p: &[f32]| p.iter().zip(&base).filter(|(a, b)| (**a - **b).abs() > 1e-4).count();
        assert!(changed(&run(&usm(150.0, 1.4, 0))) > 0, "no sharpening");
        assert_eq!(changed(&run(&usm(0.0, 1.4, 0))), 0, "amount 0 changed the picture");
        assert_eq!(changed(&run(&usm(150.0, 1.4, 20))), 0, "the threshold did not hold back a faint edge");
        let off = DetailParams { usm: UsmParams { enabled: false, ..usm(150.0, 1.4, 0).usm }, ..Default::default() };
        assert_eq!(changed(&run(&off)), 0);
        assert!(off.is_off());
        // A bigger amount moves more.
        let strength_of = |a: f32| {
            let p = run(&usm(a, 1.4, 0));
            p.iter().zip(&base).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max)
        };
        assert!(strength_of(300.0) > strength_of(100.0));
    }

    #[test]
    fn a_proxy_shrinks_the_radius_with_the_picture() {
        let step = |x: usize, _| if x < 32 { 0.3 } else { 0.6 };
        let base = grey(step);
        let max_change = |factor: usize| {
            let mut p = base.clone();
            apply(&mut p, W, H, &usm(200.0, 1.4, 0), factor, Quality::Full, WHOLE);
            p.iter().zip(&base).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max)
        };
        assert!(max_change(1) > 0.0);
        assert_eq!(max_change(8), 0.0, "a 1.4 px radius at 1/8 scale is 0.175 px: nothing to sharpen");
    }

    #[test]
    fn noise_reduction_smooths_grain_and_keeps_an_edge() {
        let noisy = |x: usize, y: usize| (if x < 32 { 0.30 } else { 0.60 }) + 0.03 * noise(y * W + x);
        let clean = |x: usize, _| if x < 32 { 0.30 } else { 0.60 };
        let err = |a: &[f32]| {
            let (mut s, mut n) = (0.0, 0.0);
            for y in 4..H - 4 {
                for x in 4..W - 4 {
                    if (x as isize - 32).abs() > 3 {
                        s += (a[y * W + x] - clean(x, y)).powi(2);
                        n += 1.0;
                    }
                }
            }
            (s / n).sqrt()
        };
        let run = |level| {
            let mut p = grey(noisy);
            apply(&mut p, W, H, &DetailParams { noise: level, ..Default::default() }, 1, Quality::Full, WHOLE);
            let a = luma(&p);
            (err(&a), a[24 * W + 33] - a[24 * W + 30])
        };

        let before = err(&luma(&grey(noisy)));
        let (light, _) = run(NoiseLevel::Light);
        let (medium, _) = run(NoiseLevel::Medium);
        let (strong, jump) = run(NoiseLevel::Strong);

        // A ladder, and each rung does something.
        assert!(strong < medium && medium < light && light < before, "not a ladder: {before} {light} {medium} {strong}");
        // Light is light. It used to take 87% of the high-frequency energy off
        // a flat wall at 3600 dpi -- five times what SilverFast's *Strong* GANE
        // takes -- and left the grain looking like wax. It keeps most of it now.
        assert!(light > 0.85 * before, "light is not light: {before} -> {light}");
        // Strong still earns its name.
        assert!(strong < 0.7 * before, "strong does too little: {before} -> {strong}");
        // The edge is still an edge: a big jump across two pixels.
        assert!(jump > 0.2, "edge smeared to {jump}");
    }

    #[test]
    fn noise_reduction_is_for_full_quality_only() {
        let base = grey(|x, y| 0.4 + 0.05 * noise(y * W + x));
        let mut p = base.clone();
        apply(&mut p, W, H, &DetailParams { noise: NoiseLevel::Strong, ..Default::default() }, 1, Quality::Preview, WHOLE);
        assert_eq!(p, base);
        apply(&mut p, W, H, &DetailParams { noise: NoiseLevel::Strong, ..Default::default() }, 1, Quality::Full, WHOLE);
        assert_ne!(p, base);
    }

    #[test]
    fn colour_is_kept_and_luminance_is_what_changes() {
        let mut p: Vec<f32> = (0..W * H).flat_map(|i| if i % W < 32 { [0.5, 0.2, 0.1] } else { [0.5, 0.3, 0.2] }).collect();
        let before = p.clone();
        apply(&mut p, W, H, &usm(100.0, 1.0, 0), 1, Quality::Full, WHOLE);
        // Away from the edge nothing moved, and nothing is NaN or out of range.
        assert!(p.iter().all(|v| v.is_finite() && (0.0..=1.01).contains(v)));
        assert!((p[24 * W * 3 + 5 * 3] - before[24 * W * 3 + 5 * 3]).abs() < 1e-3);
    }

    #[test]
    fn margins_grow_with_what_is_on() {
        assert_eq!(margin(&DetailParams::default(), 1), 0);
        assert!(margin(&usm(100.0, 1.4, 0), 1) >= 5);
        assert!(margin(&usm(100.0, 4.0, 0), 1) > margin(&usm(100.0, 1.4, 0), 1));
        assert!(margin(&DetailParams { noise: NoiseLevel::Light, ..Default::default() }, 1) >= (SEARCH + PATCH) as usize);
        assert!(margin(&usm(100.0, 4.0, 0), 4) < margin(&usm(100.0, 4.0, 0), 1), "a proxy needs less");
    }

    // ── Local contrast ──

    fn local(effect: f32) -> DetailParams {
        DetailParams { local_contrast: effect, ..Default::default() }
    }

    /// A dim, flat left half with faint texture and a bright right half.
    fn dim_and_bright() -> Vec<f32> {
        grey(|x, y| if x < 32 { 0.22 + 0.02 * noise(y * W + x) } else { 0.70 + 0.02 * noise(y * W + x + 5) })
    }

    #[test]
    fn local_contrast_lifts_texture_in_a_flat_region_and_zero_does_nothing() {
        let base = dim_and_bright();
        let mut off = base.clone();
        apply(&mut off, W, H, &local(0.0), 1, Quality::Full, WHOLE);
        assert_eq!(off, base);

        let mut on = base.clone();
        apply(&mut on, W, H, &local(100.0), 1, Quality::Full, WHOLE);
        let spread_of = |p: &[f32]| {
            let l = luma(p);
            let v: Vec<f32> = (8..40).flat_map(|y| (4..24).map(move |x| (y, x))).map(|(y, x)| l[y * W + x]).collect();
            let m = v.iter().sum::<f32>() / v.len() as f32;
            (v.iter().map(|a| (a - m).powi(2)).sum::<f32>() / v.len() as f32).sqrt()
        };
        assert!(spread_of(&on) > 1.5 * spread_of(&base), "texture {} -> {}", spread_of(&base), spread_of(&on));
        // Half the effect is between.
        let mut half = base.clone();
        apply(&mut half, W, H, &local(50.0), 1, Quality::Full, WHOLE);
        assert!(spread_of(&half) > spread_of(&base) && spread_of(&half) < spread_of(&on));
    }

    #[test]
    fn local_contrast_keeps_a_ramp_going_the_same_way() {
        let ramp = grey(|x, _| 0.1 + 0.8 * x as f32 / (W - 1) as f32);
        let mut p = ramp.clone();
        apply(&mut p, W, H, &local(100.0), 1, Quality::Full, WHOLE);
        let l = luma(&p);
        for y in [5usize, 24, 40] {
            // Interpolating between cells whose tables differ can dip a little at a
            // cell boundary (a known property of adaptive equalisation); a real
            // reversal would be a large step the wrong way.
            let worst = l[y * W..(y + 1) * W].windows(2).map(|w| w[0] - w[1]).fold(0.0f32, f32::max);
            assert!(worst < 0.04, "a ramp stepped back by {worst} on row {y}");
            assert!(l[y * W + W - 1] > l[y * W] + 0.5, "the ramp lost its slope");
        }
    }

    /// A block rendered with the whole picture's grid and its place in the
    /// scan equals the same block of the whole picture.
    #[test]
    fn a_block_with_the_whole_grid_matches_the_whole_picture() {
        let whole_src = grey(|x, y| 0.15 + 0.7 * ((x as f32 / 20.0).sin() * (y as f32 / 17.0).cos() * 0.5 + 0.5) + 0.02 * noise(y * W + x));
        let grid = measure_grid(&whole_src, W, H);
        let params = DetailParams { local_contrast: 80.0, aaco: Some(grid), ..Default::default() };
        let mut whole = whole_src.clone();
        apply(&mut whole, W, H, &params, 1, Quality::Full, WHOLE);

        let (bx, by, bw, bh) = (20usize, 10usize, 30usize, 25usize);
        let mut block: Vec<f32> = (by..by + bh).flat_map(|y| whole_src[(y * W + bx) * 3..(y * W + bx + bw) * 3].to_vec()).collect();
        apply(&mut block, bw, bh, &params, 1, Quality::Full, Placement { origin: (bx, by), full: (W, H) });
        for y in 0..bh {
            for x in 0..bw {
                let (a, b) = (block[(y * bw + x) * 3 + 1], whole[((by + y) * W + bx + x) * 3 + 1]);
                assert!((a - b).abs() < 1e-5, "block ({x},{y}): {a} vs {b}");
            }
        }
    }

    #[test]
    fn a_proxy_uses_the_same_grid_positions_as_the_scan() {
        // The same picture at half size, grid measured on each: the mapping at the same scan position agrees.
        let src = grey(|x, y| 0.2 + 0.6 * (x as f32 / W as f32) + 0.1 * (y as f32 / H as f32 * 6.0).sin());
        let small: Vec<f32> = (0..H / 2)
            .flat_map(|y| {
                (0..W / 2).flat_map({
                    let src = &src;
                    move |x| src[((y * 2) * W + x * 2) * 3..((y * 2) * W + x * 2) * 3 + 3].to_vec()
                })
            })
            .collect();
        let (g_full, g_small) = (measure_grid(&src, W, H), measure_grid(&small, W / 2, H / 2));
        let mut a = src.clone();
        apply(&mut a, W, H, &DetailParams { local_contrast: 100.0, aaco: Some(g_full), ..Default::default() }, 1, Quality::Full, WHOLE);
        let mut b = small.clone();
        apply(
            &mut b,
            W / 2,
            H / 2,
            &DetailParams { local_contrast: 100.0, aaco: Some(g_small), ..Default::default() },
            2,
            Quality::Full,
            Placement { origin: (0, 0), full: (W, H) },
        );
        let worst =
            (0..H / 2).flat_map(|y| (0..W / 2).map(move |x| (y, x))).map(|(y, x)| (a[((y * 2) * W + x * 2) * 3 + 1] - b[(y * (W / 2) + x) * 3 + 1]).abs()).fold(0.0f32, f32::max);
        let mean = (0..H / 2).flat_map(|y| (0..W / 2).map(move |x| (y, x))).map(|(y, x)| (a[((y * 2) * W + x * 2) * 3 + 1] - b[(y * (W / 2) + x) * 3 + 1]).abs()).sum::<f32>()
            / ((H / 2) * (W / 2)) as f32;
        assert!(worst < 0.1 && mean < 0.015, "the proxy and the scan disagree: worst {worst}, mean {mean}");
    }
}
