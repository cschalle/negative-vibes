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
    /// Sensitivity, 1 to 20 (F§6 "Detection", default 12): higher finds more.
    pub detection: u8,
    /// The largest mark, in pixels of the full scan, that is still dust. Set
    /// from the scan's size when `None`.
    #[serde(default)]
    pub max_area: Option<usize>,
}

fn default_mark() -> [f32; 3] {
    [1.0, 0.0, 1.0]
}

impl Default for DustParams {
    fn default() -> Self {
        Self { mode: DustMode::Correct, method: DustMethod::Infrared, defect: DefectType::Dark, mark: default_mark(), detection: 12, max_area: None }
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

/// Fill the masked pixels from the ones around them, in log transmission,
/// working inwards from the edge of each mark so every pixel is made from
/// pixels that are already good or already filled.
pub fn repair(pixels: &mut [f32], mask: &[u8], w: usize, h: usize) {
    const R: isize = 3;
    let mut known: Vec<bool> = mask.iter().map(|m| *m == 0).collect();
    if !known.iter().any(|k| *k) {
        return; // nothing to learn from
    }
    let touches_known = |known: &[bool], p: usize| -> bool {
        let (px, py) = ((p % w) as isize, (p / w) as isize);
        (-1..=1).any(|dy| (-1..=1).any(|dx| {
            let (x, y) = (px + dx, py + dy);
            x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && known[y as usize * w + x as usize]
        }))
    };
    let mut frontier: Vec<usize> = (0..w * h).filter(|p| !known[*p] && touches_known(&known, *p)).collect();
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
        frontier = (0..w * h).filter(|p| !known[*p] && touches_known(&known, *p)).collect();
        if filled.is_empty() {
            break;
        }
    }
}

/// Detect and, unless the mode is Off, repair. Returns the (possibly new)
/// pixels and what was found. `Mark` leaves the pixels alone: painting the
/// marks is done on the finished picture. Infrared detection needs `ir`;
/// without it there is nothing to find and nothing changes.
pub fn apply(pixels: &[f32], ir: Option<&[f32]>, w: usize, h: usize, params: &DustParams, factor: usize) -> (Vec<f32>, Option<DustReport>) {
    if params.mode == DustMode::Off {
        return (pixels.to_vec(), None);
    }
    let report = match (params.method, ir) {
        (DustMethod::Software, _) => detect_software(pixels, w, h, params, factor),
        (DustMethod::Infrared, Some(ir)) => detect(pixels, ir, w, h, params, factor),
        (DustMethod::Infrared, None) => return (pixels.to_vec(), None),
    };
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
        let worst = spots.iter().flat_map(|i| (0..3).map(move |c| (*i, c))).map(|(i, c)| ((fixed[i * 3 + c] - clean_px[i * 3 + c]) / clean_px[i * 3 + c]).abs()).fold(0.0f32, f32::max);
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
        let worst = (0..W * H).filter(|i| mask[*i] == 1).flat_map(|i| (0..3).map(move |c| (i, c))).map(|(i, c)| ((broken[i * 3 + c] - px[i * 3 + c]) / px[i * 3 + c]).abs()).fold(0.0f32, f32::max);
        assert!(worst < 0.03, "{worst}");
        let mut all = px.clone();
        repair(&mut all, &vec![1u8; W * H], W, H); // must not hang or panic
        assert_eq!(all, px);
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
        let worst = specks.iter().flat_map(|i| (0..3).map(move |c| (*i, c))).map(|(i, c)| ((fixed[i * 3 + c] - clean_px[i * 3 + c]) / clean_px[i * 3 + c]).abs()).fold(0.0f32, f32::max);
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
}
