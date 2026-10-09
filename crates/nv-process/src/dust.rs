//! Infrared dust and scratch removal (DESIGN.md §7.5, F§6 iSRD).
//!
//! Film is transparent to infrared, so dust, hairs and scratches, which are
//! not, show up in the infrared plane and in nothing else. The steps:
//!
//! 1. **Normalise** the infrared by a smooth estimate of its illumination, so
//!    a lamp that is brighter in the middle does not look like a defect.
//! 2. **Remove crosstalk.** Cyan dye absorbs a little infrared, so the picture
//!    leaves a faint ghost in the plane. Fit infrared against red density over
//!    the picture, robustly, and look at what the fit does not explain.
//! 3. **Detect** what is darker than the rest by a margin set from the plane's
//!    own noise (the median absolute deviation), keep marks of a plausible
//!    size, and grow them a little to cover their soft edges.
//! 4. **Repair** the marked pixels from their surroundings, in log density,
//!    working in from the edge of each mark.
//!
//! Everything is local except the statistics (illumination, fit, noise), so a
//! full-resolution tile can be repaired from the tile and a margin around it.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DustMode {
    /// Leave the picture as scanned.
    Off,
    /// Repair what was found.
    #[default]
    Correct,
    /// Show what was found, painted over the picture.
    Mark,
}

/// A brush stroke, kept as fractions of the whole scan so it means the same
/// thing at any resolution, in a tile, in the loupe, and in an export.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    /// Points along the stroke, `[x, y]` as fractions of the scan's width and height.
    pub points: Vec<[f32; 2]>,
    /// Brush radius as a fraction of the scan's longer side.
    pub radius: f32,
}

/// Paint the strokes onto a mask for a proxy of `w × h` pixels placed at
/// `origin` in a scan of `full` pixels, each proxy pixel covering `factor`
/// scan pixels. A pixel is covered when its centre is within a brush radius
/// of the stroke.
pub fn rasterise(strokes: &[Stroke], w: usize, h: usize, factor: usize, origin: (usize, usize), full: (usize, usize)) -> Vec<u8> {
    let mut mask = vec![0u8; w * h];
    let long = full.0.max(full.1).max(1) as f32;
    let f = factor.max(1) as f32;
    for stroke in strokes {
        // Brush centre and radius in this proxy's pixels.
        let to_px = |p: &[f32; 2]| ((p[0] * full.0 as f32 - origin.0 as f32) / f, (p[1] * full.1 as f32 - origin.1 as f32) / f);
        let r = (stroke.radius * long / f).max(0.5);
        let pts: Vec<(f32, f32)> = stroke.points.iter().map(to_px).collect();
        let mut stamp = |cx: f32, cy: f32| {
            let (x0, x1) = (((cx - r).floor().max(0.0)) as usize, ((cx + r).ceil().max(0.0) as usize).min(w.saturating_sub(1)));
            let (y0, y1) = (((cy - r).floor().max(0.0)) as usize, ((cy + r).ceil().max(0.0) as usize).min(h.saturating_sub(1)));
            if cx + r < 0.0 || cy + r < 0.0 || cx - r > w as f32 || cy - r > h as f32 {
                return;
            }
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                    if dx * dx + dy * dy <= r * r {
                        mask[y * w + x] = 1;
                    }
                }
            }
        };
        match pts.as_slice() {
            [] => {}
            [only] => stamp(only.0, only.1),
            _ => {
                for pair in pts.windows(2) {
                    let (a, b) = (pair[0], pair[1]);
                    // Discs at half-radius steps leave no gaps.
                    let steps = (((b.0 - a.0).hypot(b.1 - a.1)) / (r * 0.5).max(0.5)).ceil().max(1.0) as usize;
                    for k in 0..=steps {
                        let t = k as f32 / steps as f32;
                        stamp(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                    }
                }
            }
        }
    }
    mask
}

/// Distance from point `p` to the segment `a`–`b`.
fn point_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    (p[0] - (a[0] + t * dx)).hypot(p[1] - (a[1] + t * dy))
}

/// Distance between the segments `a`–`b` and `c`–`d`: zero if they cross.
fn segment_segment(a: [f32; 2], b: [f32; 2], c: [f32; 2], d: [f32; 2]) -> f32 {
    let side = |p: [f32; 2], q: [f32; 2], r: [f32; 2]| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
    if side(a, b, c) * side(a, b, d) < 0.0 && side(c, d, a) * side(c, d, b) < 0.0 {
        return 0.0;
    }
    point_segment(a, c, d).min(point_segment(b, c, d)).min(point_segment(c, a, b)).min(point_segment(d, a, b))
}

/// The shortest distance between two strokes' paths (a lone point is a path of one point).
fn path_distance(a: &[[f32; 2]], b: &[[f32; 2]]) -> f32 {
    let segs = |p: &[[f32; 2]]| -> Vec<([f32; 2], [f32; 2])> {
        match p {
            [] => vec![],
            [one] => vec![(*one, *one)],
            _ => p.windows(2).map(|w| (w[0], w[1])).collect(),
        }
    };
    let (sa, sb) = (segs(a), segs(b));
    sa.iter().flat_map(|(p, q)| sb.iter().map(move |(r, s)| segment_segment(*p, *q, *r, *s))).fold(f32::MAX, f32::min)
}

/// Remove every stroke that the eraser (points and radius, as for a stroke)
/// touches: whose brush and the eraser's overlap anywhere along their paths.
pub fn erase_touching(strokes: &mut Vec<Stroke>, eraser: &Stroke) {
    strokes.retain(|s| path_distance(&s.points, &eraser.points) > s.radius + eraser.radius);
}

/// Where dust is looked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DustMethod {
    /// In the infrared plane (F§6 iSRD): exact, but needs the plane, and
    /// silver black and white film blocks infrared.
    #[default]
    Infrared,
    /// In the picture itself (F§6 SRDx): small specks that stand out from
    /// their surroundings. Works on any scan, and can mistake fine detail
    /// for dust.
    Software,
}

/// Which specks software detection looks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DefectType {
    /// Darker than the surroundings in the scan, which is dust on the film.
    #[default]
    Dark,
    Bright,
    Both,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DustParams {
    pub mode: DustMode,
    #[serde(default)]
    pub method: DustMethod,
    #[serde(default)]
    pub defect: DefectType,
    /// Colour of Mark mode, as linear light (F§6 "mark colour").
    #[serde(default = "default_mark")]
    pub mark: [f32; 3],
    /// Where dust removal must leave the picture alone (F§6 mask: fine detail
    /// such as stars or eyelashes).
    /// Line the infrared up with the colour before looking for dust, when it
    /// is offset (F§6: a scanner's passes need not register; up to 20 pixels).
    #[serde(default = "default_true")]
    pub align: bool,
    /// The offset to apply, in scan pixels, once measured on the whole picture
    /// (so every tile uses the same one). `None` measures it.
    #[serde(default)]
    pub ir_shift: Option<[f32; 2]>,
    /// The farthest the infrared may be out of line, in scan pixels (a
    /// preference, F§14; not part of a frame's look).
    #[serde(default = "default_align_limit")]
    pub align_limit: u32,
    #[serde(default)]
    pub exclude: Vec<Stroke>,
    /// Where it must repair, whether or not anything was detected.
    #[serde(default)]
    pub include: Vec<Stroke>,
    /// Sensitivity, 1 to 20 (F§6 "Detection", default 12): higher finds more.
    pub detection: u8,
    /// The largest mark, in pixels of the full scan, that is still dust. Set
    /// from the scan's size when `None`.
    #[serde(default)]
    pub max_area: Option<usize>,
}

fn default_align_limit() -> u32 {
    MAX_SHIFT as u32
}

fn default_true() -> bool {
    true
}

fn default_mark() -> [f32; 3] {
    [1.0, 0.0, 1.0]
}

impl Default for DustParams {
    fn default() -> Self {
        Self {
            mode: DustMode::Correct,
            method: DustMethod::Infrared,
            defect: DefectType::Dark,
            mark: default_mark(),
            align: true,
            ir_shift: None,
            align_limit: MAX_SHIFT as u32,
            exclude: Vec::new(),
            include: Vec::new(),
            detection: 12,
            max_area: None,
        }
    }
}

/// What detection found.
#[derive(Debug, Clone, PartialEq)]
pub struct DustReport {
    /// One byte per pixel: 1 where the pixel is repaired.
    pub mask: Vec<u8>,
    /// Separate marks.
    pub marks: usize,
    /// Fraction of the picture the mask covers.
    pub covered: f32,
}

/// Margin to render around a tile so marks and repairs at its edge see their
/// surroundings, in full-resolution pixels.
pub const TILE_MARGIN: usize = 24;

/// Largest radius an infrared mark is grown by, at full resolution.
const GROW_PX: f32 = 2.0;

/// The detection threshold in noise units for a sensitivity.
fn threshold_mads(detection: u8) -> f32 {
    (120.0 / detection.clamp(1, 20) as f32).clamp(5.0, 120.0)
}

// ── Statistics ────────────────────────────────────────────────────────────────

fn median(v: &mut [f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let mid = v.len() / 2;
    *v.select_nth_unstable_by(mid, |a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)).1
}

/// Median and median absolute deviation.
fn median_mad(v: &[f32]) -> (f32, f32) {
    let mut a = v.to_vec();
    let m = median(&mut a);
    let mut d: Vec<f32> = v.iter().map(|x| (x - m).abs()).collect();
    (m, median(&mut d))
}

// ── Registration ──────────────────────────────────────────────────────────────

/// The farthest the infrared may be out of line with the colour, in scan pixels.
pub const MAX_SHIFT: usize = 20;

/// Gradient magnitude of a plane.
fn edges(a: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut out = vec![0f32; w * h];
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            let gx = a[y * w + x + 1] - a[y * w + x - 1];
            let gy = a[(y + 1) * w + x] - a[(y - 1) * w + x];
            out[y * w + x] = gx.hypot(gy);
        }
    }
    out
}

/// Average `f × f` blocks.
fn reduce(a: &[f32], w: usize, h: usize, f: usize) -> (Vec<f32>, usize, usize) {
    if f <= 1 {
        return (a.to_vec(), w, h);
    }
    let (rw, rh) = (w / f, h / f);
    let mut out = vec![0f32; rw * rh];
    for y in 0..rh {
        for x in 0..rw {
            let mut s = 0.0;
            for dy in 0..f {
                for dx in 0..f {
                    s += a[(y * f + dy) * w + x * f + dx];
                }
            }
            out[y * rw + x] = s / (f * f) as f32;
        }
    }
    (out, rw, rh)
}

/// Normalised correlation of two edge maps with `b` shifted by `(dx, dy)`,
/// over the part of `a` that stays inside both and away from the border.
fn shifted_ncc(a: &[f32], b: &[f32], w: usize, h: usize, dx: isize, dy: isize, margin: usize) -> f32 {
    let (mut sab, mut saa, mut sbb) = (0f64, 0f64, 0f64);
    for y in margin..h.saturating_sub(margin) {
        let yy = y as isize + dy;
        if yy < 0 || yy >= h as isize {
            continue;
        }
        for x in margin..w.saturating_sub(margin) {
            let xx = x as isize + dx;
            if xx < 0 || xx >= w as isize {
                continue;
            }
            let (va, vb) = (a[y * w + x] as f64, b[yy as usize * w + xx as usize] as f64);
            sab += va * vb;
            saa += va * va;
            sbb += vb * vb;
        }
    }
    if saa < 1e-12 || sbb < 1e-12 { 0.0 } else { (sab / (saa * sbb).sqrt()) as f32 }
}

/// How far the infrared sits from the colour: the `(dx, dy)`, in pixels of
/// this image, such that `ir(x + dx, y + dy)` lines up with the colour at
/// `(x, y)`. Found by correlating the edges of the two, coarsely over the
/// whole range and then finely, and to a fraction of a pixel by a parabola
/// through the best offset and its neighbours. `None` when there is too
/// little structure to tell, or nothing correlates.
pub fn estimate_shift(pixels: &[f32], ir: &[f32], w: usize, h: usize, factor: usize) -> Option<[f32; 2]> {
    estimate_shift_within(pixels, ir, w, h, factor, MAX_SHIFT)
}

/// [`estimate_shift`] looking no farther than `limit` scan pixels.
pub fn estimate_shift_within(pixels: &[f32], ir: &[f32], w: usize, h: usize, factor: usize, limit: usize) -> Option<[f32; 2]> {
    if w < 48 || h < 48 || pixels.len() < w * h * 3 || ir.len() < w * h {
        return None;
    }
    let max = (limit / factor.max(1)).max(2);
    let ln_c: Vec<f32> = (0..w * h).map(|i| pixels[i * 3].max(1e-4).ln()).collect();
    let ln_i: Vec<f32> = ir.iter().map(|v| v.max(1e-4).ln()).collect();
    let (ec, ei) = (edges(&ln_c, w, h), edges(&ln_i, w, h));

    // Coarse: reduce until the search is small.
    let f = (max / 6).max(1);
    let (cc, cw, ch) = reduce(&ec, w, h, f);
    let (ci, _, _) = reduce(&ei, w, h, f);
    let r = (max / f + 1) as isize;
    let margin = (r as usize + 2).min(cw / 4);
    let mut best = (f32::MIN, 0isize, 0isize);
    for dy in -r..=r {
        for dx in -r..=r {
            let n = shifted_ncc(&cc, &ci, cw, ch, dx, dy, margin);
            if n > best.0 {
                best = (n, dx, dy);
            }
        }
    }
    // Fine: around the coarse answer, at this image's own resolution.
    let (cx, cy) = (best.1 * f as isize, best.2 * f as isize);
    let span = f as isize + 1;
    let fine_margin = (max + 4).min(w.min(h) / 4);
    let mut fine = (f32::MIN, cx, cy);
    for dy in cy - span..=cy + span {
        for dx in cx - span..=cx + span {
            let n = shifted_ncc(&ec, &ei, w, h, dx, dy, fine_margin);
            if n > fine.0 {
                fine = (n, dx, dy);
            }
        }
    }
    if fine.0 < 0.12 {
        return None; // the two share no structure worth trusting
    }
    // A parabola through the peak and its two neighbours, on each axis.
    let at = |dx: isize, dy: isize| shifted_ncc(&ec, &ei, w, h, dx, dy, fine_margin);
    let sub = |m1: f32, c: f32, p1: f32| -> f32 {
        let d = m1 - 2.0 * c + p1;
        if d.abs() < 1e-9 { 0.0 } else { (0.5 * (m1 - p1) / d).clamp(-0.5, 0.5) }
    };
    let (sx, sy) = (sub(at(fine.1 - 1, fine.2), fine.0, at(fine.1 + 1, fine.2)), sub(at(fine.1, fine.2 - 1), fine.0, at(fine.1, fine.2 + 1)));
    let found = [fine.1 as f32 + sx, fine.2 as f32 + sy];
    // The search is a little wider than asked, to find the peak's flank; the answer is not.
    let reach = (limit / factor.max(1)) as f32 + 0.5;
    (found[0].abs() <= reach && found[1].abs() <= reach).then_some(found)
}

/// The infrared resampled so it lines up with the colour: `out(x, y) =
/// ir(x + dx, y + dy)`, bilinear, edges repeated.
pub fn shift_plane(ir: &[f32], w: usize, h: usize, dx: f32, dy: f32) -> Vec<f32> {
    let mut out = vec![0f32; w * h];
    let at = |x: isize, y: isize| ir[y.clamp(0, h as isize - 1) as usize * w + x.clamp(0, w as isize - 1) as usize];
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 + dx, y as f32 + dy);
            let (x0, y0) = (fx.floor(), fy.floor());
            let (tx, ty) = (fx - x0, fy - y0);
            let (x0, y0) = (x0 as isize, y0 as isize);
            out[y * w + x] = at(x0, y0) * (1.0 - tx) * (1.0 - ty) + at(x0 + 1, y0) * tx * (1.0 - ty) + at(x0, y0 + 1) * (1.0 - tx) * ty + at(x0 + 1, y0 + 1) * tx * ty;
        }
    }
    out
}

/// What to shift the infrared by, in this image's pixels, if anything: the
/// frozen offset when the picture has been measured as a whole, else the
/// measurement. Offsets under a quarter of a pixel are noise and are left alone.
fn registration(pixels: &[f32], ir: &[f32], w: usize, h: usize, params: &DustParams, factor: usize) -> Option<[f32; 2]> {
    if !params.align {
        return None;
    }
    let s = match params.ir_shift {
        Some(s) => [s[0] / factor as f32, s[1] / factor as f32],
        None => estimate_shift_within(pixels, ir, w, h, factor, params.align_limit as usize)?,
    };
    (s[0].abs() >= 0.25 || s[1].abs() >= 0.25).then_some(s)
}

// ── Detection ─────────────────────────────────────────────────────────────────

/// Terms of a polynomial in `(u, v)` up to this total degree: the lamp's
/// falloff, which is smooth but bends hard near the edges of the film window.
const DEGREE: usize = 4;

fn terms(u: f64, v: f64, out: &mut Vec<f64>) {
    out.clear();
    for i in 0..=DEGREE {
        for j in 0..=DEGREE - i {
            out.push(u.powi(i as i32) * v.powi(j as i32));
        }
    }
}

/// Solve `A x = b` by Gaussian elimination with partial pivoting.
fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for col in 0..n {
        let piv = (col..n).max_by(|&i, &j| a[i][col].abs().partial_cmp(&a[j][col].abs()).unwrap_or(std::cmp::Ordering::Equal))?;
        if a[piv][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, piv);
        b.swap(col, piv);
        for r in col + 1..n {
            let f = a[r][col] / a[col][col];
            for c in col..n {
                a[r][c] -= f * a[col][c];
            }
            b[r] -= f * b[col];
        }
    }
    let mut x = vec![0.0; n];
    for r in (0..n).rev() {
        let s: f64 = (r + 1..n).map(|c| a[r][c] * x[c]).sum();
        x[r] = (b[r] - s) / a[r][r];
    }
    Some(x)
}

/// Find the dust in a scan. `pixels` is linear RGB (`w × h × 3`), `ir` the
/// infrared plane, `factor` how many scan pixels one of these covers (1 for
/// the scan itself, more for a proxy), which scales what counts as a mark.
pub fn detect(pixels: &[f32], ir: &[f32], w: usize, h: usize, params: &DustParams, factor: usize) -> DustReport {
    let none = || DustReport { mask: vec![0; w * h], marks: 0, covered: 0.0 };
    if w < 16 || h < 16 || pixels.len() < w * h * 3 || ir.len() < w * h {
        return none();
    }

    // The film window: where there is light at all. The dark surround of the
    // holder is not film and has no say in what the film looks like.
    let level = {
        let mut v: Vec<f32> = ir.iter().step_by(((w * h) / 20_000).max(1)).copied().collect();
        median(&mut v)
    };
    let film: Vec<bool> = ir.iter().map(|v| *v > 0.35 * level).collect();
    let ln_ir: Vec<f32> = ir.iter().map(|v| v.max(1e-4).ln()).collect();
    let ln_red: Vec<f32> = (0..w * h).map(|i| pixels[i * 3].max(1e-4).ln()).collect();
    let coord = |i: usize| ((2 * (i % w) + 1) as f64 / w as f64 - 1.0, (2 * (i / w) + 1) as f64 / h as f64 - 1.0);

    // Fit infrared = smooth lamp surface + ghost of the picture, over the film,
    // three times, each time leaving out what sits well below the fit (dust).
    let stride = (film.iter().filter(|f| **f).count() / 12_000).max(1);
    let samples: Vec<usize> = (0..w * h).filter(|i| film[*i] && i % stride == 0).collect();
    let n_terms = (DEGREE + 1) * (DEGREE + 2) / 2 + 1;
    if samples.len() < 4 * n_terms {
        return none();
    }
    let mut keep = vec![true; samples.len()];
    let mut fit = vec![0.0f64; n_terms];
    let mut row = Vec::new();
    for round in 0..3 {
        let mut ata = vec![vec![0.0f64; n_terms]; n_terms];
        let mut atb = vec![0.0f64; n_terms];
        for (k, &i) in samples.iter().enumerate() {
            if !keep[k] {
                continue;
            }
            let (u, v) = coord(i);
            terms(u, v, &mut row);
            row.push(ln_red[i] as f64);
            let y = ln_ir[i] as f64;
            for a in 0..n_terms {
                atb[a] += row[a] * y;
                for c in a..n_terms {
                    ata[a][c] += row[a] * row[c];
                }
            }
        }
        for a in 0..n_terms {
            for c in 0..a {
                ata[a][c] = ata[c][a];
            }
            ata[a][a] += 1e-9;
        }
        let Some(x) = solve(ata, atb) else { return none() };
        fit = x;
        let res: Vec<f32> = samples
            .iter()
            .map(|&i| {
                let (u, v) = coord(i);
                terms(u, v, &mut row);
                row.push(ln_red[i] as f64);
                (ln_ir[i] as f64 - row.iter().zip(&fit).map(|(a, b)| a * b).sum::<f64>()) as f32
            })
            .collect();
        let (m, mad) = median_mad(&res);
        let limit = 3.0 * 1.4826 * mad.max(1e-4);
        for k in 0..samples.len() {
            keep[k] = (res[k] - m).abs() <= limit;
        }
        if round == 2 {
            break;
        }
    }

    // What the lamp and the picture's ghost do not explain, everywhere.
    let resid: Vec<f32> = (0..w * h)
        .map(|i| {
            let (u, v) = coord(i);
            terms(u, v, &mut row);
            row.push(ln_red[i] as f64);
            (ln_ir[i] as f64 - row.iter().zip(&fit).map(|(a, b)| a * b).sum::<f64>()) as f32
        })
        .collect();
    let sample: Vec<f32> = samples.iter().map(|&i| resid[i]).collect();
    let (med, mad) = median_mad(&sample);
    // Never below a few percent of the infrared level: a clean, noiseless
    // plane has no spread, and everything would look like a defect.
    let thr = med - (threshold_mads(params.detection) * mad).max(0.04);

    // Dust is dark in the infrared and may be darker than the film window's
    // own cut-off, so candidates are not limited to the window: what lies
    // outside it is one enormous mark, and the size limit throws that out.
    // Dust and fibres are dark in the infrared; a scratch in the emulsion
    // scatters light and can be bright. The same margin serves both sides.
    let (dark, bright) = match params.defect {
        DefectType::Dark => (true, false),
        DefectType::Bright => (false, true),
        DefectType::Both => (true, true),
    };
    let score: Vec<f32> = resid
        .iter()
        .map(|r| {
            let d = if dark { med - *r } else { f32::MIN };
            let b = if bright { *r - med } else { f32::MIN };
            d.max(b)
        })
        .collect();
    marks_from(&score, med - thr, w, h, params, factor, false)
}

/// From a score per pixel (higher is more dust-like) and the score at which a
/// pixel becomes a candidate: connected candidates that are of a plausible
/// size, grown to cover their soft edges.
fn marks_from(score: &[f32], margin: f32, w: usize, h: usize, params: &DustParams, factor: usize, compact: bool) -> DustReport {
    let candidate: Vec<bool> = score.iter().map(|s| *s > margin).collect();
    let max_area = params.max_area.unwrap_or_else(|| ((0.004 * (w * h) as f64) as usize).max(50));
    let max_area_here = (max_area / (factor * factor).max(1)).max(8);
    let mut label = vec![0u32; w * h];
    let mut mask = vec![0u8; w * h];
    let mut marks = 0usize;
    let mut stack = Vec::new();
    for start in 0..w * h {
        if !candidate[start] || label[start] != 0 {
            continue;
        }
        marks += 1;
        let id = marks as u32;
        let mut members = vec![start];
        label[start] = id;
        stack.push(start);
        let mut strongest = score[start];
        let (mut bx0, mut bx1, mut by0, mut by1) = (start % w, start % w, start / w, start / w);
        while let Some(p) = stack.pop() {
            let (px, py) = (p % w, p / w);
            // Eight neighbours: a hair is a thin diagonal line, and its pixels
            // touch at the corners.
            for dy in [-1isize, 0, 1] {
                for dx in [-1isize, 0, 1] {
                    let (nx, ny) = (px as isize + dx, py as isize + dy);
                    if nx < 0 || ny < 0 || nx as usize >= w || ny as usize >= h {
                        continue;
                    }
                    let q = ny as usize * w + nx as usize;
                    if candidate[q] && label[q] == 0 {
                        label[q] = id;
                        strongest = strongest.max(score[q]);
                        let (qx, qy) = (q % w, q / w);
                        (bx0, bx1, by0, by1) = (bx0.min(qx), bx1.max(qx), by0.min(qy), by1.max(qy));
                        members.push(q);
                        stack.push(q);
                    }
                }
            }
        }
        // A lone pixel is noise unless it is far past the threshold.
        let strong = strongest > 2.0 * margin;
        // Software detection cannot tell a speck from the picture's own fine
        // lines, so it keeps only what is compact: a speck is a few pixels
        // across, a rim or a hair of a glass is long.
        let side = (bx1 - bx0).max(by1 - by0) + 1;
        let too_long = compact && (side > (10 / factor).max(4) || members.len() > (60 / (factor * factor)).max(6));
        if members.len() > max_area_here || too_long || (members.len() < 2 && !strong) {
            marks -= 1;
            continue;
        }
        for q in members {
            mask[q] = 1;
        }
    }

    // Grow to cover the soft edge of each mark.
    let radius = ((GROW_PX / factor as f32).round() as usize).max(1);
    let mask = dilate(&mask, w, h, radius);
    let covered = mask.iter().map(|m| *m as usize).sum::<usize>() as f32 / (w * h) as f32;
    DustReport { mask, marks, covered }
}

// ── Software detection (SRDx) ─────────────────────────────────────────────────

/// Grey-scale erosion (`min`) or dilation (`max`) over a square of radius `r`,
/// separably.
fn morph(a: &[f32], w: usize, h: usize, r: usize, max: bool) -> Vec<f32> {
    let pick = |x: f32, y: f32| if max { x.max(y) } else { x.min(y) };
    let init = if max { f32::MIN } else { f32::MAX };
    let mut tmp = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut v = init;
            for xx in x.saturating_sub(r)..(x + r + 1).min(w) {
                v = pick(v, a[y * w + xx]);
            }
            tmp[y * w + x] = v;
        }
    }
    let mut out = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut v = init;
            for yy in y.saturating_sub(r)..(y + r + 1).min(h) {
                v = pick(v, tmp[yy * w + x]);
            }
            out[y * w + x] = v;
        }
    }
    out
}

/// Find specks in the picture itself: what a morphological top-hat lifts out,
/// the part of each small dark (or bright) feature that rises above its
/// surroundings. The picture's own detail narrower than a few pixels looks
/// like dust to this, which is why it is for films that infrared cannot see
/// through, and why masks exist.
pub fn detect_software(pixels: &[f32], w: usize, h: usize, params: &DustParams, factor: usize) -> DustReport {
    let none = || DustReport { mask: vec![0; w * h], marks: 0, covered: 0.0 };
    if w < 16 || h < 16 || pixels.len() < w * h * 3 {
        return none();
    }
    // Log transmission of the mean of the channels: density-like, so a speck
    // of a given opacity is the same step whatever it sits on.
    let l: Vec<f32> = pixels.chunks_exact(3).map(|p| ((p[0] + p[1] + p[2]) / 3.0).max(1e-4).ln()).collect();
    let r = ((3.0 / factor as f32).round() as usize).max(1);

    let score_of = |dark: bool| -> (Vec<f32>, f32) {
        // Dark specks: closing minus the picture. Bright: the picture minus opening.
        let t: Vec<f32> = if dark {
            let closed = morph(&morph(&l, w, h, r, true), w, h, r, false);
            closed.iter().zip(&l).map(|(c, v)| c - v).collect()
        } else {
            let opened = morph(&morph(&l, w, h, r, false), w, h, r, true);
            l.iter().zip(&opened).map(|(v, o)| v - o).collect()
        };
        let (bx, by) = (w / 12, h / 12);
        let inner: Vec<f32> = (by..h - by).flat_map(|y| (bx..w - bx).map(move |x| (y, x))).map(|(y, x)| t[y * w + x]).collect();
        let (med, mad) = median_mad(&inner);
        // At least a few percent: a smooth picture has no spread to measure.
        let margin = (threshold_mads(params.detection) * mad).max(0.10);
        (t.iter().map(|v| v - med).collect(), margin)
    };
    let (dark, bright) = match params.defect {
        DefectType::Dark => (true, false),
        DefectType::Bright => (false, true),
        DefectType::Both => (true, true),
    };
    let mut best: Option<(Vec<f32>, f32)> = None;
    for kind in [dark.then_some(true), bright.then_some(false)].into_iter().flatten() {
        let (score, margin) = score_of(kind);
        best = Some(match best {
            None => (score, margin),
            // Both kinds: a pixel counts for whichever is stronger, against the larger margin.
            Some((a, ma)) => {
                let m = ma.max(margin);
                (a.iter().zip(&score).map(|(x, y)| x.max(*y)).collect(), m)
            }
        });
    }
    match best {
        Some((score, margin)) => marks_from(&score, margin, w, h, params, factor, true),
        None => none(),
    }
}

/// Grow a mask by `r` pixels in every direction (a disk, not a square: a
/// round speck stays round).
fn dilate(mask: &[u8], w: usize, h: usize, r: usize) -> Vec<u8> {
    let mut out = mask.to_vec();
    let ri = r as isize;
    let limit = ri * ri + ri / 2;
    for y in 0..h {
        for x in 0..w {
            if mask[y * w + x] == 0 {
                continue;
            }
            for dy in -ri..=ri {
                for dx in -ri..=ri {
                    if dx * dx + dy * dy <= limit {
                        let (nx, ny) = (x as isize + dx, y as isize + dy);
                        if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                            out[ny as usize * w + nx as usize] = 1;
                        }
                    }
                }
            }
        }
    }
    out
}

// ── Repair ────────────────────────────────────────────────────────────────────

/// Marks larger than this many pixels are filled from matching patches of the
/// picture instead of being smoothed over: a smooth fill is invisible in a
/// speck and a smear in a scratch.
const LARGE_MARK: usize = 30;
/// Marks larger than this are smoothed over, whatever their size.
const PATCH_MAX: usize = 6000;

/// Connected components (four-way) of the mask that are larger than `limit`.
fn large_marks(mask: &[u8], w: usize, h: usize, limit: usize) -> Vec<bool> {
    let mut label = vec![false; w * h];
    let mut large = vec![false; w * h];
    let mut stack = Vec::new();
    for start in 0..w * h {
        if mask[start] == 0 || label[start] {
            continue;
        }
        let mut members = vec![start];
        label[start] = true;
        stack.push(start);
        while let Some(p) = stack.pop() {
            let (px, py) = (p % w, p / w);
            for (nx, ny) in [(px.wrapping_sub(1), py), (px + 1, py), (px, py.wrapping_sub(1)), (px, py + 1)] {
                if nx < w && ny < h {
                    let q = ny * w + nx;
                    if mask[q] != 0 && !label[q] {
                        label[q] = true;
                        members.push(q);
                        stack.push(q);
                    }
                }
            }
        }
        if members.len() > limit {
            for q in members {
                large[q] = true;
            }
        }
    }
    large
}

fn touches_known(known: &[bool], p: usize, w: usize, h: usize) -> bool {
    let (px, py) = ((p % w) as isize, (p / w) as isize);
    (-1..=1).any(|dy| {
        (-1..=1).any(|dx| {
            let (x, y) = (px + dx, py + dy);
            x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && known[y as usize * w + x as usize]
        })
    })
}

/// Fill the pixels in `fill` from the known ones around them, in log
/// transmission, working in from the edge so every pixel is made from pixels
/// that are already good or already filled.
fn onion(pixels: &mut [f32], known: &mut [bool], fill: &[bool], w: usize, h: usize) {
    const R: isize = 3;
    let mut frontier: Vec<usize> = (0..w * h).filter(|p| fill[*p] && !known[*p] && touches_known(known, *p, w, h)).collect();
    while !frontier.is_empty() {
        let mut filled = Vec::with_capacity(frontier.len());
        for &p in &frontier {
            let (px, py) = ((p % w) as isize, (p / w) as isize);
            let mut acc = [0f64; 3];
            let mut wsum = 0f64;
            for dy in -R..=R {
                for dx in -R..=R {
                    let (x, y) = (px + dx, py + dy);
                    if x < 0 || y < 0 || x as usize >= w || y as usize >= h {
                        continue;
                    }
                    let q = y as usize * w + x as usize;
                    if known[q] {
                        let wt = 1.0 / (1.0 + (dx * dx + dy * dy) as f64);
                        for c in 0..3 {
                            acc[c] += wt * (pixels[q * 3 + c].max(1e-5) as f64).ln();
                        }
                        wsum += wt;
                    }
                }
            }
            if wsum > 0.0 {
                filled.push((p, [(acc[0] / wsum).exp() as f32, (acc[1] / wsum).exp() as f32, (acc[2] / wsum).exp() as f32]));
            }
        }
        // Commit the layer together, so a pixel does not lean on a neighbour
        // that was filled a moment ago in the same pass.
        for (p, v) in &filled {
            pixels[p * 3..p * 3 + 3].copy_from_slice(v);
            known[*p] = true;
        }
        if filled.is_empty() {
            break;
        }
        frontier = (0..w * h).filter(|p| fill[*p] && !known[*p] && touches_known(known, *p, w, h)).collect();
    }
}

/// Fill the pixels in `fill` by copying, for each, the pixel whose
/// surroundings best match what is already known around it, found within a
/// window, and brought to this place's brightness and colour. Unlike
/// [`onion`] this keeps the grain and the lines of the picture.
fn patch_fill(pixels: &mut [f32], known: &mut [bool], fill: &[bool], w: usize, h: usize) {
    const SEARCH: isize = 14;
    const P: isize = 2; // compare 5 × 5 neighbourhoods
    let ln = |p: &[f32], i: usize, c: usize| (p[i * 3 + c].max(1e-5) as f64).ln();
    let mut frontier: Vec<usize> = (0..w * h).filter(|p| fill[*p] && !known[*p] && touches_known(known, *p, w, h)).collect();
    while !frontier.is_empty() {
        let mut progressed = false;
        for &p in &frontier {
            let (px, py) = ((p % w) as isize, (p / w) as isize);
            // What is known around this pixel, as offsets.
            let around: Vec<(isize, isize)> = (-P..=P)
                .flat_map(|dy| (-P..=P).map(move |dx| (dx, dy)))
                .filter(|(dx, dy)| {
                    (*dx, *dy) != (0, 0) && {
                        let (x, y) = (px + dx, py + dy);
                        x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && known[y as usize * w + x as usize]
                    }
                })
                .collect();
            if around.len() < 4 {
                continue; // not enough to match on yet: a later layer
            }
            let mut best: Option<(f64, usize)> = None;
            for sy in (py - SEARCH).max(P)..=(py + SEARCH).min(h as isize - 1 - P) {
                for sx in (px - SEARCH).max(P)..=(px + SEARCH).min(w as isize - 1 - P) {
                    let q = sy as usize * w + sx as usize;
                    if !known[q] || fill[q] && !known[q] {
                        continue;
                    }
                    let mut ssd = 0.0;
                    let mut ok = true;
                    for (dx, dy) in &around {
                        let (qx, qy) = (sx + dx, sy + dy);
                        let qi = qy as usize * w + qx as usize;
                        if qx < 0 || qy < 0 || qx as usize >= w || qy as usize >= h || !known[qi] {
                            ok = false;
                            break;
                        }
                        let pi = (py + dy) as usize * w + (px + dx) as usize;
                        for c in 0..3 {
                            let d = ln(pixels, pi, c) - ln(pixels, qi, c);
                            ssd += d * d;
                        }
                    }
                    if ok && best.is_none_or(|(b, _)| ssd < b) {
                        best = Some((ssd, q));
                    }
                }
            }
            let Some((_, q)) = best else { continue };
            // Brightness and colour of this place: the mean difference over the matched neighbourhood.
            let (qx, qy) = ((q % w) as isize, (q / w) as isize);
            let mut off = [0f64; 3];
            for (dx, dy) in &around {
                let pi = (py + dy) as usize * w + (px + dx) as usize;
                let qi = (qy + dy) as usize * w + (qx + dx) as usize;
                for c in 0..3 {
                    off[c] += ln(pixels, pi, c) - ln(pixels, qi, c);
                }
            }
            for c in 0..3 {
                let v = ln(pixels, q, c) + off[c] / around.len() as f64;
                pixels[p * 3 + c] = v.exp() as f32;
            }
            known[p] = true;
            progressed = true;
        }
        if !progressed {
            break;
        }
        frontier = (0..w * h).filter(|p| fill[*p] && !known[*p] && touches_known(known, *p, w, h)).collect();
    }
    // Anything the patches could not reach is smoothed over, so nothing is left unfilled.
    onion(pixels, known, fill, w, h);
}

/// Fill the masked pixels: small marks by smoothing from their surroundings,
/// large ones (scratches, hairs) from matching patches, so the grain of the
/// film runs through the repair.
pub fn repair(pixels: &mut [f32], mask: &[u8], w: usize, h: usize) {
    let mut known: Vec<bool> = mask.iter().map(|m| *m == 0).collect();
    if !known.iter().any(|k| *k) {
        return; // nothing to learn from
    }
    // Patch search costs about a few hundred thousand operations per pixel
    // filled: right for a hair or a scratch, far too much for a blotch, which
    // is smoothed over instead.
    let large = large_marks(mask, w, h, LARGE_MARK);
    let huge = large_marks(mask, w, h, PATCH_MAX);
    let patch: Vec<bool> = (0..w * h).map(|i| large[i] && !huge[i]).collect();
    let smooth: Vec<bool> = (0..w * h).map(|i| mask[i] != 0 && !patch[i]).collect();
    onion(pixels, &mut known, &smooth, w, h);
    if patch.iter().any(|l| *l) {
        patch_fill(pixels, &mut known, &patch, w, h);
    }
}

/// Detect and, unless the mode is Off, repair. Returns the (possibly new)
/// pixels and what was found. `Mark` leaves the pixels alone: painting the
/// marks is done on the finished picture. Infrared detection needs `ir`;
/// without it there is nothing to find and nothing changes.
pub fn apply(pixels: &[f32], ir: Option<&[f32]>, w: usize, h: usize, params: &DustParams, factor: usize) -> (Vec<f32>, Option<DustReport>) {
    apply_placed(pixels, ir, w, h, params, factor, (0, 0), (w * factor, h * factor))
}

/// [`apply`] for a block that sits at `origin` in a scan of `full` pixels,
/// which is where the brush strokes are measured from.
#[allow(clippy::too_many_arguments)]
pub fn apply_placed(
    pixels: &[f32],
    ir: Option<&[f32]>,
    w: usize,
    h: usize,
    params: &DustParams,
    factor: usize,
    origin: (usize, usize),
    full: (usize, usize),
) -> (Vec<f32>, Option<DustReport>) {
    if params.mode == DustMode::Off {
        return (pixels.to_vec(), None);
    }
    let detected = match (params.method, ir) {
        (DustMethod::Software, _) => Some(detect_software(pixels, w, h, params, factor)),
        (DustMethod::Infrared, Some(ir)) => match registration(pixels, ir, w, h, params, factor) {
            Some(s) => Some(detect(pixels, &shift_plane(ir, w, h, s[0], s[1]), w, h, params, factor)),
            None => Some(detect(pixels, ir, w, h, params, factor)),
        },
        (DustMethod::Infrared, None) => None,
    };
    // Without a plane there is nothing to detect, but a stroke can still ask for a repair.
    if detected.is_none() && params.include.is_empty() {
        return (pixels.to_vec(), None);
    }
    let mut report = detected.unwrap_or(DustReport { mask: vec![0; w * h], marks: 0, covered: 0.0 });

    // The person's brush wins over the detector: protected pixels are left
    // alone, painted-in pixels are repaired.
    if !params.exclude.is_empty() {
        let keep = rasterise(&params.exclude, w, h, factor, origin, full);
        for (m, k) in report.mask.iter_mut().zip(&keep) {
            if *k != 0 {
                *m = 0;
            }
        }
    }
    if !params.include.is_empty() {
        let add = rasterise(&params.include, w, h, factor, origin, full);
        for (m, a) in report.mask.iter_mut().zip(&add) {
            if *a != 0 {
                *m = 1;
            }
        }
    }
    report.covered = report.mask.iter().map(|m| *m as usize).sum::<usize>() as f32 / (w * h).max(1) as f32;

    let mut out = pixels.to_vec();
    if params.mode == DustMode::Correct {
        repair(&mut out, &report.mask, w, h);
    }
    (out, Some(report))
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 160;
    const H: usize = 120;

    /// A smooth "picture" of linear transmission, and a flat clean infrared
    /// plane with a slow lamp falloff.
    fn clean() -> (Vec<f32>, Vec<f32>) {
        let mut px = Vec::new();
        let mut ir = Vec::new();
        for y in 0..H {
            for x in 0..W {
                let (fx, fy) = (x as f32 / W as f32, y as f32 / H as f32);
                px.extend_from_slice(&[0.30 + 0.25 * fx, 0.18 + 0.12 * fy, 0.10 + 0.05 * (fx + fy)]);
                let lamp = 0.78 + 0.12 * (1.0 - ((fx - 0.5).powi(2) + (fy - 0.5).powi(2))).max(0.0);
                ir.push(lamp);
            }
        }
        (px, ir)
    }

    /// A tiny deterministic noise source in [-1, 1].
    fn noise(i: usize) -> f32 {
        let v = (i as u32).wrapping_mul(2654435761).rotate_left(13).wrapping_mul(40503);
        (v >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }

    /// Dust: opaque to infrared (the plane drops) and to colour (the picture
    /// darkens), over a few spots and a hair.
    fn dirty() -> (Vec<f32>, Vec<f32>, Vec<f32>, Vec<usize>) {
        let (clean_px, clean_ir) = clean();
        let (mut px, mut ir) = (clean_px.clone(), clean_ir);
        let mut spots = Vec::new();
        let mut hit = |x: usize, y: usize| {
            let i = y * W + x;
            spots.push(i);
            ir[i] *= 0.25;
            for c in 0..3 {
                px[i * 3 + c] *= 0.3;
            }
        };
        for (cx, cy) in [(40usize, 40usize), (100, 70), (70, 95)] {
            for dy in 0..3 {
                for dx in 0..3 {
                    hit(cx + dx, cy + dy);
                }
            }
        }
        for t in 0..40 {
            hit(20 + t, 20 + t / 3); // a hair
        }
        (px, ir, clean_px, spots)
    }

    #[test]
    fn dust_is_found_and_repaired_and_nothing_else_changes() {
        let (px, ir, clean_px, spots) = dirty();
        let report = detect(&px, &ir, W, H, &DustParams::default(), 1);
        assert!(spots.iter().all(|i| report.mask[*i] == 1), "missed dust");
        assert!(report.covered < 0.06, "mask covers {:.1}%", report.covered * 100.0);
        assert!((4..=8).contains(&report.marks), "{} marks", report.marks);

        let (fixed, _) = apply(&px, Some(&ir), W, H, &DustParams::default(), 1);
        // Away from the marks the picture is exactly as it was.
        for i in 0..W * H {
            if report.mask[i] == 0 {
                assert_eq!(&fixed[i * 3..i * 3 + 3], &px[i * 3..i * 3 + 3], "pixel {i} changed outside the mask");
            }
        }
        // Inside, the repair is close to the picture that was there.
        let worst =
            spots.iter().flat_map(|i| (0..3).map(move |c| (*i, c))).map(|(i, c)| ((fixed[i * 3 + c] - clean_px[i * 3 + c]) / clean_px[i * 3 + c]).abs()).fold(0.0f32, f32::max);
        assert!(worst < 0.05, "repair is {:.1}% off", worst * 100.0);
    }

    #[test]
    fn a_clean_plane_with_noise_and_a_ghost_of_the_picture_finds_nothing() {
        let (px, mut ir) = clean();
        for (i, v) in ir.iter_mut().enumerate() {
            *v *= 1.0 + 0.01 * noise(i); // 1% noise
            // Cyan dye ghost: the picture leaks into the infrared.
            *v *= px[i * 3].powf(0.35) * 1.5;
        }
        let report = detect(&px, &ir, W, H, &DustParams::default(), 1);
        assert!(report.covered < 0.001, "noise and ghost flagged {:.2}%", report.covered * 100.0);
    }

    #[test]
    fn a_large_dark_region_is_not_dust() {
        let (mut px, mut ir) = clean();
        for y in 30..90 {
            for x in 40..120 {
                ir[y * W + x] *= 0.3;
                px[(y * W + x) * 3] *= 0.3;
            }
        }
        let report = detect(&px, &ir, W, H, &DustParams::default(), 1);
        assert_eq!(report.mask[60 * W + 80], 0, "a big dark patch was treated as dust");
    }

    #[test]
    fn a_bright_scratch_in_the_infrared_is_found_only_when_asked() {
        let (px, mut ir, _, _) = dirty();
        let mut scratch = Vec::new();
        for t in 0..30 {
            let i = (100 + t / 6) * W + 15 + t;
            scratch.push(i);
            ir[i] *= 1.8; // emulsion scratch: brighter
        }
        let found = |defect| detect(&px, &ir, W, H, &DustParams { defect, ..Default::default() }, 1).mask;
        let (dark, bright, both) = (found(DefectType::Dark), found(DefectType::Bright), found(DefectType::Both));
        assert!(scratch.iter().all(|i| dark[*i] == 0), "dark mode took a bright scratch");
        assert!(scratch.iter().all(|i| bright[*i] == 1), "bright mode missed it");
        assert!(scratch.iter().all(|i| both[*i] == 1));
        // And the dust is still dust in "both".
        let spot = 40 * W + 40;
        assert!(dark[spot] == 1 && both[spot] == 1 && bright[spot] == 0);
    }

    #[test]
    fn higher_detection_finds_more() {
        let (px, mut ir, _, _) = dirty();
        for (i, v) in ir.iter_mut().enumerate() {
            *v *= 1.0 + 0.02 * noise(i + 7);
            // A faint speck too, between noise and a real mark.
            if i == 55 * W + 60 || i == 55 * W + 61 {
                *v *= 0.9;
            }
        }
        let covered = |d: u8| detect(&px, &ir, W, H, &DustParams { detection: d, ..Default::default() }, 1).covered;
        let (low, mid, high) = (covered(2), covered(12), covered(20));
        assert!(low <= mid && mid <= high, "{low} {mid} {high}");
        assert!(high > low, "sensitivity does nothing");
    }

    #[test]
    fn off_changes_nothing_and_mark_does_not_repair() {
        let (px, ir, _, _) = dirty();
        let (off, r) = apply(&px, Some(&ir), W, H, &DustParams { mode: DustMode::Off, ..Default::default() }, 1);
        assert!(r.is_none() && off == px);
        let (mark, r) = apply(&px, Some(&ir), W, H, &DustParams { mode: DustMode::Mark, ..Default::default() }, 1);
        assert!(r.is_some_and(|r| r.marks > 0));
        assert_eq!(mark, px, "mark mode leaves the pixels for the renderer to paint over");
    }

    #[test]
    fn repair_fills_a_hole_smoothly_and_survives_an_all_masked_picture() {
        let (px, _) = clean();
        let mut broken = px.clone();
        let mut mask = vec![0u8; W * H];
        for y in 50..58 {
            for x in 70..78 {
                mask[y * W + x] = 1;
                for c in 0..3 {
                    broken[(y * W + x) * 3 + c] = 5.0;
                }
            }
        }
        repair(&mut broken, &mask, W, H);
        let worst = (0..W * H)
            .filter(|i| mask[*i] == 1)
            .flat_map(|i| (0..3).map(move |c| (i, c)))
            .map(|(i, c)| ((broken[i * 3 + c] - px[i * 3 + c]) / px[i * 3 + c]).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 0.03, "{worst}");
        let mut all = px.clone();
        repair(&mut all, &vec![1u8; W * H], W, H); // must not hang or panic
        assert_eq!(all, px);
    }

    /// Standard deviation of one channel over the pixels in `at`.
    fn spread(p: &[f32], at: impl Iterator<Item = usize>) -> f32 {
        let v: Vec<f32> = at.map(|i| p[i * 3]).collect();
        let m = v.iter().sum::<f32>() / v.len() as f32;
        (v.iter().map(|x| (x - m).powi(2)).sum::<f32>() / v.len() as f32).sqrt()
    }

    /// A scratch is repaired with the grain of its surroundings, not smoothed
    /// into a smear, and the colour still matches.
    #[test]
    fn a_large_mark_is_filled_with_grain_and_a_small_one_with_a_smooth_fill() {
        let (clean_px, _) = clean();
        let grainy: Vec<f32> = clean_px.iter().enumerate().map(|(i, v)| v * (1.0 + 0.06 * noise(i * 7 + 3))).collect();
        let hole = |x0: usize, y0: usize, n: usize| -> Vec<usize> { (y0..y0 + n).flat_map(|y| (x0..x0 + n).map(move |x| y * W + x)).collect() };

        // 14 × 14 = 196 pixels: a large mark.
        let big = hole(70, 50, 14);
        let mut mask = vec![0u8; W * H];
        let mut broken = grainy.clone();
        for i in &big {
            mask[*i] = 1;
            for c in 0..3 {
                broken[i * 3 + c] = 9.0;
            }
        }
        repair(&mut broken, &mask, W, H);
        let (orig, filled) = (spread(&grainy, big.iter().copied()), spread(&broken, big.iter().copied()));
        assert!(filled > 0.5 * orig, "grain lost: {filled} against {orig}");
        let mean = |p: &[f32]| big.iter().map(|i| p[i * 3]).sum::<f32>() / big.len() as f32;
        assert!((mean(&broken) - mean(&grainy)).abs() / mean(&grainy) < 0.08, "colour off");

        // 4 × 4 = 16 pixels: small, smoothed (and invisible at that size).
        let tiny = hole(30, 80, 4);
        let mut mask = vec![0u8; W * H];
        let mut broken = grainy.clone();
        for i in &tiny {
            mask[*i] = 1;
            for c in 0..3 {
                broken[i * 3 + c] = 9.0;
            }
        }
        repair(&mut broken, &mask, W, H);
        assert!(spread(&broken, tiny.iter().copied()) < 0.5 * spread(&grainy, tiny.iter().copied()) + 0.02, "small marks use the smooth fill");
    }

    /// An edge through a large mark is carried across it by the patches.
    #[test]
    fn a_line_through_a_large_mark_continues_through_it() {
        let (mut px, _) = clean();
        for y in 0..H {
            for x in W / 2..W {
                for c in 0..3 {
                    px[(y * W + x) * 3 + c] *= 0.4;
                }
            }
        }
        let mut mask = vec![0u8; W * H];
        let mut broken = px.clone();
        for y in 50..64 {
            for x in W / 2 - 7..W / 2 + 7 {
                mask[y * W + x] = 1;
                for c in 0..3 {
                    broken[(y * W + x) * 3 + c] = 9.0;
                }
            }
        }
        repair(&mut broken, &mask, W, H);
        // The step from the left of the hole to the right of it, along its middle row.
        let row = 57 * W;
        let (left, right) = (broken[(row + W / 2 - 8) * 3], broken[(row + W / 2 + 7) * 3]);
        let (a, b) = (broken[(row + W / 2 - 4) * 3], broken[(row + W / 2 + 3) * 3]);
        let true_step = (left - right).abs();
        assert!((a - b).abs() > 0.45 * true_step, "the edge was smeared flat: {} of a true {true_step}", (a - b).abs());
    }

    #[test]
    fn tiny_or_mismatched_input_is_left_alone() {
        let r = detect(&[0.5; 12], &[0.5; 4], 2, 2, &DustParams::default(), 1);
        assert_eq!((r.marks, r.covered), (0, 0.0));
        let (px, ir) = clean();
        assert_eq!(detect(&px, &ir[..10], W, H, &DustParams::default(), 1).marks, 0);
    }

    // ── Software detection ──

    /// A smooth picture with a mild texture, and dark specks of 2–3 pixels.
    fn speckled() -> (Vec<f32>, Vec<f32>, Vec<usize>) {
        let (mut px, _) = clean();
        let clean_px = px.clone();
        for (i, v) in px.iter_mut().enumerate() {
            *v *= 1.0 + 0.004 * noise(i);
        }
        let mut specks = Vec::new();
        for (cx, cy) in [(30usize, 30usize), (90, 50), (60, 90), (120, 20)] {
            for dy in 0..2 {
                for dx in 0..3 {
                    let i = (cy + dy) * W + cx + dx;
                    specks.push(i);
                    for c in 0..3 {
                        px[i * 3 + c] *= 0.45;
                    }
                }
            }
        }
        (px, clean_px, specks)
    }

    #[test]
    fn software_detection_finds_specks_without_any_infrared() {
        let (px, clean_px, specks) = speckled();
        let params = DustParams { method: DustMethod::Software, ..Default::default() };
        let r = detect_software(&px, W, H, &params, 1);
        assert!(specks.iter().all(|i| r.mask[*i] == 1), "missed a speck");
        assert!(r.covered < 0.05, "{:.1}%", r.covered * 100.0);
        let (fixed, _) = apply(&px, None, W, H, &params, 1);
        let worst =
            specks.iter().flat_map(|i| (0..3).map(move |c| (*i, c))).map(|(i, c)| ((fixed[i * 3 + c] - clean_px[i * 3 + c]) / clean_px[i * 3 + c]).abs()).fold(0.0f32, f32::max);
        assert!(worst < 0.06, "repair {:.1}% off", worst * 100.0);
    }

    #[test]
    fn software_detection_leaves_edges_and_smooth_gradients_alone() {
        let (mut px, _) = clean();
        // A hard edge down the middle of the picture: not a speck.
        for y in 0..H {
            for x in W / 2..W {
                for c in 0..3 {
                    px[(y * W + x) * 3 + c] *= 0.5;
                }
            }
        }
        let r = detect_software(&px, W, H, &DustParams { method: DustMethod::Software, defect: DefectType::Both, ..Default::default() }, 1);
        assert_eq!(r.marks, 0, "an edge or a gradient was taken for dust ({:.2}% covered)", r.covered * 100.0);
    }

    #[test]
    fn defect_type_chooses_dark_or_bright() {
        let (mut px, _, specks) = speckled();
        // Add a bright speck.
        let bright = 100 * W + 20;
        for c in 0..3 {
            px[bright * 3 + c] *= 2.6;
            px[(bright + 1) * 3 + c] *= 2.6;
        }
        let find = |defect| detect_software(&px, W, H, &DustParams { method: DustMethod::Software, defect, ..Default::default() }, 1).mask;
        let (dark, bright_only, both) = (find(DefectType::Dark), find(DefectType::Bright), find(DefectType::Both));
        assert!(dark[specks[0]] == 1 && dark[bright] == 0, "dark mode");
        assert!(bright_only[bright] == 1 && bright_only[specks[0]] == 0, "bright mode");
        assert!(both[specks[0]] == 1 && both[bright] == 1, "both");
    }

    #[test]
    fn infrared_method_without_a_plane_changes_nothing() {
        let (px, _) = clean();
        let (out, report) = apply(&px, None, W, H, &DustParams::default(), 1);
        assert!(report.is_none() && out == px);
    }

    // ── Brush strokes ──

    fn dot(x: f32, y: f32, r: f32) -> Stroke {
        Stroke { points: vec![[x, y]], radius: r }
    }

    #[test]
    fn a_stroke_paints_a_disc_where_its_fractions_say() {
        // A 200 × 100 scan, brush radius 5% of the long side = 10 px.
        let m = rasterise(&[dot(0.5, 0.5, 0.05)], 200, 100, 1, (0, 0), (200, 100));
        let area = m.iter().map(|v| *v as f32).sum::<f32>();
        assert!((area - std::f32::consts::PI * 100.0).abs() < 12.0, "area {area}");
        assert_eq!(m[50 * 200 + 100], 1);
        assert_eq!(m[50 * 200 + 130], 0);
        // A stroke is a line of discs with no gaps.
        let line = rasterise(&[Stroke { points: vec![[0.1, 0.5], [0.9, 0.5]], radius: 0.01 }], 200, 100, 1, (0, 0), (200, 100));
        assert!((20..=180).all(|x| line[50 * 200 + x] == 1), "gaps in a stroke");
        // One that is off the picture paints nothing and does not panic.
        assert!(rasterise(&[dot(3.0, 3.0, 0.05)], 200, 100, 1, (0, 0), (200, 100)).iter().all(|v| *v == 0));
        assert!(rasterise(&[Stroke { points: vec![], radius: 0.1 }], 20, 10, 1, (0, 0), (20, 10)).iter().all(|v| *v == 0));
    }

    /// The same stroke covers the same part of the scan whatever the proxy's
    /// size, and a block cut from the scan finds it where it belongs.
    #[test]
    fn a_stroke_means_the_same_thing_at_any_size_and_place() {
        let stroke = [Stroke { points: vec![[0.3, 0.4], [0.6, 0.45]], radius: 0.04 }];
        let full = (400usize, 300usize);
        let whole = rasterise(&stroke, 400, 300, 1, (0, 0), full);
        let half = rasterise(&stroke, 200, 150, 2, (0, 0), full);
        let (a, b) = (whole.iter().map(|v| *v as f32).sum::<f32>() / (400.0 * 300.0), half.iter().map(|v| *v as f32).sum::<f32>() / (200.0 * 150.0));
        assert!((a - b).abs() / a < 0.06, "{a} vs {b}");
        // A block at (100, 50): pixel (x, y) of it is (x + 100, y + 50) of the scan.
        let block = rasterise(&stroke, 150, 120, 1, (100, 50), full);
        for (x, y) in [(30usize, 80usize), (60, 85), (140, 10), (5, 5)] {
            assert_eq!(block[y * 150 + x], whole[(y + 50) * 400 + x + 100], "block pixel {x},{y}");
        }
    }

    #[test]
    fn the_eraser_removes_what_it_touches() {
        let mut strokes = vec![dot(0.2, 0.2, 0.02), dot(0.8, 0.8, 0.02), Stroke { points: vec![[0.2, 0.7], [0.4, 0.7]], radius: 0.02 }];
        erase_touching(&mut strokes, &dot(0.21, 0.21, 0.02));
        assert_eq!(strokes.len(), 2);
        erase_touching(&mut strokes, &Stroke { points: vec![[0.3, 0.71]], radius: 0.02 });
        assert_eq!(strokes.len(), 1);
        assert_eq!(strokes[0].points[0], [0.8, 0.8]);
    }

    #[test]
    fn a_protect_stroke_keeps_dust_removal_off_the_picture_and_a_heal_stroke_forces_it() {
        let (px, ir, _, spots) = dirty();
        // Protect the first 3 × 3 spot (around pixel (40, 40) of 160 × 120).
        let protect = Stroke { points: vec![[41.5 / W as f32, 41.5 / H as f32]], radius: 5.0 / W as f32 };
        let params = DustParams { exclude: vec![protect], ..Default::default() };
        let (fixed, report) = apply(&px, Some(&ir), W, H, &params, 1);
        let spot = 41 * W + 41;
        assert!(spots.contains(&spot) && report.unwrap().mask[spot] == 0, "the protected spot was marked");
        assert_eq!(&fixed[spot * 3..spot * 3 + 3], &px[spot * 3..spot * 3 + 3], "the protected spot was repaired");
        // The other dust is still found.
        let (_, r2) = apply(&px, Some(&ir), W, H, &params, 1);
        assert!(r2.unwrap().mask[spots[spots.len() - 1]] == 1);

        // Heal: paint over a clean place, with no infrared at all.
        let (clean_px, _) = clean();
        let mut marked = clean_px.clone();
        let (hx, hy) = (80usize, 60usize);
        for c in 0..3 {
            marked[(hy * W + hx) * 3 + c] *= 0.2;
        } // a blemish nothing detects
        let heal = DustParams { include: vec![Stroke { points: vec![[hx as f32 / W as f32, hy as f32 / H as f32]], radius: 3.0 / W as f32 }], ..Default::default() };
        let (healed, rep) = apply(&marked, None, W, H, &heal, 1);
        assert!(rep.is_some_and(|r| r.mask[hy * W + hx] == 1));
        let want = clean_px[(hy * W + hx) * 3];
        assert!((healed[(hy * W + hx) * 3] - want).abs() / want < 0.05, "healed {} want {want}", healed[(hy * W + hx) * 3]);
        // And with no strokes and no plane there is still nothing to do.
        assert!(apply(&marked, None, W, H, &DustParams::default(), 1).1.is_none());
    }

    #[test]
    fn strokes_survive_json_and_old_settings_have_none() {
        let p = DustParams { exclude: vec![dot(0.1, 0.2, 0.03)], include: vec![Stroke { points: vec![[0.5, 0.5], [0.6, 0.6]], radius: 0.01 }], ..Default::default() };
        let back: DustParams = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
        let old: DustParams = serde_json::from_str(r#"{"mode":"correct","detection":12}"#).unwrap();
        assert!(old.exclude.is_empty() && old.include.is_empty());
    }

    // ── Registration ──

    /// A textured picture with edges, and an infrared plane that carries its
    /// ghost: the same structure, shifted by `(sx, sy)`.
    fn textured(sx: f32, sy: f32) -> (Vec<f32>, Vec<f32>) {
        let (w, h) = (192usize, 144usize);
        // Value noise on a grid of 9-pixel cells: structure at every place and
        // no repeat to confuse the search (a checkerboard matches itself half a
        // period away, since edges have no polarity).
        let hash = |ix: i32, iy: i32| -> f32 {
            let v = (ix.wrapping_mul(73856093) ^ iy.wrapping_mul(19349663)) as u32;
            let v = v.wrapping_mul(2654435761).rotate_left(11).wrapping_mul(40503);
            (v >> 8) as f32 / (1u32 << 24) as f32
        };
        let pic = |x: f32, y: f32| -> f32 {
            let (gx, gy) = (x / 9.0, y / 9.0);
            let (ix, iy) = (gx.floor() as i32, gy.floor() as i32);
            let (tx, ty) = (gx - ix as f32, gy - iy as f32);
            let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
            let top = hash(ix, iy) * (1.0 - sx) + hash(ix + 1, iy) * sx;
            let bottom = hash(ix, iy + 1) * (1.0 - sx) + hash(ix + 1, iy + 1) * sx;
            (0.15 + 0.8 * (top * (1.0 - sy) + bottom * sy)).clamp(0.05, 1.0)
        };
        let mut px = Vec::new();
        let mut ir = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let c = pic(x as f32, y as f32);
                px.extend_from_slice(&[c, c * 0.6, c * 0.4]);
                // The plane at (x, y) is the picture at (x - sx, y - sy): ir(x + sx, y + sy) = colour(x, y).
                ir.push(0.8 * pic(x as f32 - sx, y as f32 - sy).powf(0.3));
            }
        }
        (px, ir)
    }

    #[test]
    fn the_offset_between_infrared_and_colour_is_measured_to_a_fraction_of_a_pixel() {
        for (sx, sy) in [(0.0f32, 0.0f32), (4.0, -3.0), (-7.0, 5.0), (2.4, 1.3), (12.0, 9.0)] {
            let (px, ir) = textured(sx, sy);
            let got = estimate_shift(&px, &ir, 192, 144, 1).expect("a shift");
            assert!((got[0] - sx).abs() < 0.4 && (got[1] - sy).abs() < 0.4, "wanted ({sx}, {sy}) got {got:?}");
        }
    }

    #[test]
    fn the_alignment_limit_is_respected() {
        let (px, ir) = textured(12.0, 9.0);
        let near = estimate_shift_within(&px, &ir, 192, 144, 1, 20).expect("within 20");
        assert!((near[0] - 12.0).abs() < 0.5 && (near[1] - 9.0).abs() < 0.5, "{near:?}");
        // With a 5-pixel limit the answer cannot be 12 pixels away.
        assert!(estimate_shift_within(&px, &ir, 192, 144, 1, 5).is_none(), "an offset beyond the limit must not be reported");
        assert_eq!(DustParams::default().align_limit, 20);
        let mut v = serde_json::to_value(DustParams::default()).unwrap();
        v.as_object_mut().unwrap().remove("align_limit");
        let old: DustParams = serde_json::from_value(v).unwrap();
        assert_eq!(old.align_limit, 20);
    }

    #[test]
    fn a_featureless_plane_gives_no_offset_and_small_ones_are_ignored() {
        let (px, _) = textured(0.0, 0.0);
        assert!(estimate_shift(&px, &vec![0.7; 192 * 144], 192, 144, 1).is_none());
        let p = DustParams::default();
        let (px, ir) = textured(0.1, -0.1);
        assert!(registration(&px, &ir, 192, 144, &p, 1).is_none(), "noise-level offsets must not be applied");
        let off = DustParams { align: false, ..Default::default() };
        let (px, ir) = textured(5.0, 0.0);
        assert!(registration(&px, &ir, 192, 144, &off, 1).is_none());
        // A frozen offset is used as given, scaled to this image.
        let frozen = DustParams { ir_shift: Some([8.0, -4.0]), ..Default::default() };
        assert_eq!(registration(&px, &ir, 192, 144, &frozen, 2), Some([4.0, -2.0]));
    }

    #[test]
    fn shifting_a_plane_moves_it_and_a_whole_shift_is_exact() {
        let (w, h) = (20usize, 10usize);
        let a: Vec<f32> = (0..w * h).map(|i| (i % w) as f32).collect();
        let s = shift_plane(&a, w, h, 3.0, 0.0);
        assert_eq!(&s[..6], &[3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
        let half = shift_plane(&a, w, h, 0.5, 0.0);
        assert!((half[5] - 5.5).abs() < 1e-6);
    }

    /// Dust is found where the colour has it, even when the infrared that shows it is displaced.
    #[test]
    fn dust_is_found_in_the_right_place_when_the_infrared_is_out_of_line() {
        let (mut px, mut ir) = textured(5.0, -4.0);
        let spot = [(60usize, 50usize), (140, 90)];
        for (cx, cy) in spot {
            for dy in 0..3 {
                for dx in 0..3 {
                    // The speck on the film: dark in colour here, dark in the plane at (+5, -4).
                    let i = (cy + dy) * 192 + cx + dx;
                    for c in 0..3 {
                        px[i * 3 + c] *= 0.4;
                    }
                    let j = ((cy + dy) as isize - 4) as usize * 192 + cx + dx + 5;
                    ir[j] *= 0.25;
                }
            }
        }
        let hit = |p: &DustParams| {
            let (_, r) = apply(&px, Some(&ir), 192, 144, p, 1);
            let m = r.unwrap().mask;
            spot.iter().filter(|(cx, cy)| m[(cy + 1) * 192 + cx + 1] == 1).count()
        };
        assert_eq!(hit(&DustParams::default()), 2, "aligned detection missed the dust");
        assert!(hit(&DustParams { align: false, ..Default::default() }) < 2, "misaligned detection should lose it");
    }
}
