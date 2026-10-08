//! Fine straightening (F§7, F§8): the picture turned by a small angle about
//! its centre and enlarged just enough that the turned picture fills the
//! frame, so no empty corners appear. The size stays that of the oriented
//! scan, which keeps every position on the canvas a fraction of the same
//! picture.
//!
//! A positive angle turns the picture clockwise.

/// The widest turn offered, in degrees.
pub const MAX_ANGLE: f32 = 15.0;

/// How much the picture is enlarged: the sampled area is `k` times the
/// picture (`k ≤ 1`), the largest that still lies wholly inside the scan.
pub fn fill_scale(angle_deg: f32, w: usize, h: usize) -> f32 {
    let t = angle_deg.abs().min(89.0).to_radians();
    let (c, s) = (t.cos(), t.sin());
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    (w / (w * c + h * s)).min(h / (h * c + w * s)).min(1.0)
}

/// Where output pixel centre `(ox + 0.5, oy + 0.5)` of a `w × h` picture comes
/// from in the scan, in continuous scan coordinates (pixel `i` spans `i..i+1`).
#[inline]
pub fn source_point(angle_deg: f32, k: f32, w: usize, h: usize, ox: f32, oy: f32) -> (f32, f32) {
    let t = angle_deg.to_radians();
    let (c, s) = (t.cos(), t.sin());
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let (dx, dy) = (ox + 0.5 - cx, oy + 0.5 - cy);
    (cx + k * (dx * c + dy * s), cy + k * (-dx * s + dy * c))
}

/// A point of the straightened picture as a point of the oriented scan, both
/// as fractions `(0..1)` of the `w × h` picture.
pub fn display_to_picture(angle_deg: f32, (w, h): (f64, f64), fx: f64, fy: f64) -> (f64, f64) {
    if angle_deg.abs() < 0.005 || w < 1.0 || h < 1.0 {
        return (fx, fy);
    }
    let k = fill_scale(angle_deg, w as usize, h as usize) as f64;
    let t = (angle_deg as f64).to_radians();
    let (c, s) = (t.cos(), t.sin());
    let (dx, dy) = ((fx - 0.5) * w, (fy - 0.5) * h);
    (0.5 + k * (dx * c + dy * s) / w, 0.5 + k * (-dx * s + dy * c) / h)
}

/// The inverse of [`display_to_picture`].
pub fn picture_to_display(angle_deg: f32, (w, h): (f64, f64), px: f64, py: f64) -> (f64, f64) {
    if angle_deg.abs() < 0.005 || w < 1.0 || h < 1.0 {
        return (px, py);
    }
    let k = fill_scale(angle_deg, w as usize, h as usize) as f64;
    let t = (angle_deg as f64).to_radians();
    let (c, s) = (t.cos(), t.sin());
    let (dx, dy) = ((px - 0.5) * w / k, (py - 0.5) * h / k);
    (0.5 + (dx * c - dy * s) / w, 0.5 + (dx * s + dy * c) / h)
}

/// Bilinear sample of a `w × h`, `ch`-channel picture at a continuous position.
#[inline]
fn sample<T: Copy + Into<f32>>(src: &[T], w: usize, h: usize, ch: usize, x: f32, y: f32, out: &mut [f32]) {
    let (fx, fy) = ((x - 0.5).clamp(0.0, (w - 1) as f32), (y - 0.5).clamp(0.0, (h - 1) as f32));
    let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    for c in 0..ch {
        let v = |xx: usize, yy: usize| -> f32 { src[(yy * w + xx) * ch + c].into() };
        out[c] = v(x0, y0) * (1.0 - tx) * (1.0 - ty) + v(x1, y0) * tx * (1.0 - ty) + v(x0, y1) * (1.0 - tx) * ty + v(x1, y1) * tx * ty;
    }
}

/// The `bw × bh` block at `(x0, y0)` of the straightened picture, made from
/// `src`, a `(sx0, sy0)`-offset block of `sw × sh` of the whole `w × h`
/// picture. `src` must cover [`source_bounds`] of the block.
#[allow(clippy::too_many_arguments)]
pub fn straighten_block(
    src: &[f32],
    (sx0, sy0, sw, sh): (usize, usize, usize, usize),
    (w, h): (usize, usize),
    angle_deg: f32,
    (x0, y0, bw, bh): (usize, usize, usize, usize),
    ch: usize,
) -> Vec<f32> {
    let k = fill_scale(angle_deg, w, h);
    let mut out = vec![0f32; bw * bh * ch];
    let mut px = [0f32; 4];
    for by in 0..bh {
        for bx in 0..bw {
            let (x, y) = source_point(angle_deg, k, w, h, (x0 + bx) as f32, (y0 + by) as f32);
            // Position inside the block we were given; the clamp is only for rounding at its edges.
            sample(src, sw, sh, ch, x - sx0 as f32, y - sy0 as f32, &mut px);
            out[(by * bw + bx) * ch..][..ch].copy_from_slice(&px[..ch]);
        }
    }
    out
}

/// The part of the `w × h` picture (as `(x0, y0, w, h)`) that the block
/// `(x0, y0, bw, bh)` of the straightened picture is made from, with a pixel
/// to spare for the interpolation.
pub fn source_bounds(angle_deg: f32, (w, h): (usize, usize), (x0, y0, bw, bh): (usize, usize, usize, usize)) -> (usize, usize, usize, usize) {
    let k = fill_scale(angle_deg, w, h);
    let corners = [(x0, y0), (x0 + bw, y0), (x0, y0 + bh), (x0 + bw, y0 + bh)].map(|(x, y)| source_point(angle_deg, k, w, h, x as f32 - 0.5, y as f32 - 0.5));
    let min = |f: fn(&(f32, f32)) -> f32| corners.iter().map(f).fold(f32::MAX, f32::min);
    let max = |f: fn(&(f32, f32)) -> f32| corners.iter().map(f).fold(f32::MIN, f32::max);
    let (bx0, bx1) = ((min(|c| c.0) - 2.0).floor().max(0.0) as usize, ((max(|c| c.0) + 2.0).ceil() as usize).min(w));
    let (by0, by1) = ((min(|c| c.1) - 2.0).floor().max(0.0) as usize, ((max(|c| c.1) + 2.0).ceil() as usize).min(h));
    (bx0, by0, bx1.saturating_sub(bx0).max(1), by1.saturating_sub(by0).max(1))
}

/// The whole picture straightened: `w × h` pixels of `ch` values (8-bit
/// preview pictures use this; the values are interpolated as they are).
pub fn straighten_u8(src: &[u8], w: usize, h: usize, ch: usize, angle_deg: f32) -> Vec<u8> {
    if angle_deg.abs() < 0.005 || w < 2 || h < 2 || src.len() < w * h * ch {
        return src.to_vec();
    }
    let k = fill_scale(angle_deg, w, h);
    let mut out = vec![0u8; w * h * ch];
    let mut px = [0f32; 4];
    for y in 0..h {
        for x in 0..w {
            let (sx, sy) = source_point(angle_deg, k, w, h, x as f32, y as f32);
            sample(src, w, h, ch, sx, sy, &mut px);
            for c in 0..ch {
                out[(y * w + x) * ch + c] = (px[c] + 0.5) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scale_keeps_every_output_corner_inside_the_scan() {
        for (w, h) in [(600usize, 400usize), (400, 600), (500, 500), (3000, 2000)] {
            for a in [-15.0f32, -3.0, -0.2, 0.0, 0.7, 2.0, 8.0, 15.0] {
                let k = fill_scale(a, w, h);
                assert!(k <= 1.0 && k > 0.5, "{w}x{h} at {a}: {k}");
                for (x, y) in [(0.0, 0.0), (w as f32 - 1.0, 0.0), (0.0, h as f32 - 1.0), (w as f32 - 1.0, h as f32 - 1.0)] {
                    let (sx, sy) = source_point(a, k, w, h, x, y);
                    assert!(sx >= -0.01 && sx <= w as f32 + 0.01 && sy >= -0.01 && sy <= h as f32 + 0.01, "{w}x{h} at {a}: corner ({x},{y}) came from ({sx},{sy})");
                }
            }
        }
        assert_eq!(fill_scale(0.0, 600, 400), 1.0);
        // The scale is tight: some corner reaches the border.
        let (w, h) = (600usize, 400usize);
        let k = fill_scale(5.0, w, h);
        let touches = [(0.0, 0.0), (w as f32 - 1.0, 0.0), (0.0, h as f32 - 1.0), (w as f32 - 1.0, h as f32 - 1.0)].iter().any(|&(x, y)| {
            let (sx, sy) = source_point(5.0, k, w, h, x, y);
            sx < 1.5 || sy < 1.5 || sx > w as f32 - 1.5 || sy > h as f32 - 1.5
        });
        assert!(touches, "the picture is enlarged more than it needs to be");
    }

    #[test]
    fn a_positive_angle_turns_the_picture_clockwise() {
        // A horizontal line across the middle; after a clockwise turn its right end is lower.
        let (w, h) = (200usize, 100usize);
        let mut img = vec![0u8; w * h];
        for x in 0..w {
            img[50 * w + x] = 255;
            img[49 * w + x] = 255;
        }
        let out = straighten_u8(&img, w, h, 1, 5.0);
        let row_of_peak = |col: usize| (0..h).max_by_key(|y| out[y * w + col]).unwrap();
        let (left, right) = (row_of_peak(40), row_of_peak(160));
        assert!(right > left + 8, "right end at row {right}, left at row {left}");
        // About tan(5°) · 120 px.
        let expect = (5f32.to_radians().tan() * 120.0) as usize;
        assert!((right - left).abs_diff(expect) <= 4, "{} vs {expect}", right - left);
    }

    #[test]
    fn points_map_there_and_back_and_agree_with_the_sampler() {
        let (w, h) = (300.0f64, 200.0f64);
        for a in [-7.5f32, 0.0, 1.3, 12.0] {
            for (fx, fy) in [(0.5, 0.5), (0.1, 0.2), (0.9, 0.85)] {
                let (px, py) = display_to_picture(a, (w, h), fx, fy);
                let (bx, by) = picture_to_display(a, (w, h), px, py);
                assert!((bx - fx).abs() < 1e-9 && (by - fy).abs() < 1e-9, "{a}: {fx},{fy} -> {bx},{by}");
                // The same place the sampler reads for that output pixel.
                let k = fill_scale(a, 300, 200);
                let (sx, sy) = source_point(a, k, 300, 200, (fx * w) as f32 - 0.5, (fy * h) as f32 - 0.5);
                assert!((sx as f64 / w - px).abs() < 1e-3 && (sy as f64 / h - py).abs() < 1e-3, "{a}: {sx},{sy} vs {px},{py}");
            }
        }
        // The centre does not move.
        assert_eq!(display_to_picture(9.0, (300.0, 200.0), 0.5, 0.5), (0.5, 0.5));
    }

    #[test]
    fn no_angle_leaves_the_picture_alone_and_flat_colour_stays_flat() {
        let img: Vec<u8> = (0..30 * 20 * 3).map(|i| (i * 7 % 251) as u8).collect();
        assert_eq!(straighten_u8(&img, 30, 20, 3, 0.0), img);
        let flat = vec![90u8; 30 * 20 * 3];
        assert!(straighten_u8(&flat, 30, 20, 3, 7.3).iter().all(|v| *v == 90));
    }

    #[test]
    fn a_block_made_from_its_bounds_equals_that_part_of_the_whole() {
        let (w, h) = (120usize, 80usize);
        let src: Vec<f32> = (0..w * h * 3).map(|i| ((i * 31 % 997) as f32) / 997.0).collect();
        let angle = -3.4;
        let whole = straighten_block(&src, (0, 0, w, h), (w, h), angle, (0, 0, w, h), 3);
        for blk in [(10usize, 5usize, 40usize, 30usize), (80, 50, 40, 30), (0, 60, 120, 20)] {
            let (bx, by, bw, bh) = source_bounds(angle, (w, h), blk);
            let part: Vec<f32> = (by..by + bh).flat_map(|y| src[(y * w + bx) * 3..(y * w + bx + bw) * 3].to_vec()).collect();
            let got = straighten_block(&part, (bx, by, bw, bh), (w, h), angle, blk, 3);
            for y in 0..blk.3 {
                for x in 0..blk.2 {
                    for c in 0..3 {
                        let a = got[(y * blk.2 + x) * 3 + c];
                        let b = whole[((blk.1 + y) * w + blk.0 + x) * 3 + c];
                        assert!((a - b).abs() < 1e-5, "block {blk:?} at ({x},{y}): {a} vs {b}");
                    }
                }
            }
        }
    }
}
