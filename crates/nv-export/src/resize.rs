//! Making the exported picture smaller (F§9: pixel sizes, named sizes).
//!
//! The picture is averaged down by exact area coverage: each output pixel is
//! the mean of the scan pixels it covers, fractional edges weighted by how
//! much of them it covers. That is a box filter, which is what a downscale
//! should be (nothing in it can ring or alias the way a sharper kernel does),
//! and it is done on linear light, before the output curve, so a fine grid
//! of black and white averages to the grey the eye sees.
//!
//! It streams: rows go in as the renderer produces them and finished rows
//! come out, so a 7200 dpi frame is never held whole.

/// Averages `src_w × src_h` down to `dst_w × dst_h` (neither larger than the source).
pub struct Downscaler {
    src_h: usize,
    dst_w: usize,
    channels: usize,
    /// Per output column: the first source column and the weights from there.
    columns: Vec<(usize, Vec<f32>)>,
    /// Source rows per output row.
    ratio: f64,
    /// The output row being built, and how far down the source it has got.
    acc: Vec<f32>,
    out_row: usize,
    next_src: usize,
}

impl Downscaler {
    pub fn new(src_w: usize, src_h: usize, dst_w: usize, dst_h: usize, channels: usize) -> Self {
        let (dst_w, dst_h) = (dst_w.clamp(1, src_w.max(1)), dst_h.clamp(1, src_h.max(1)));
        let rx = src_w as f64 / dst_w as f64;
        let columns = (0..dst_w)
            .map(|x| {
                let (a, b) = (x as f64 * rx, (x + 1) as f64 * rx);
                let first = a.floor() as usize;
                let last = ((b - 1e-9).ceil() as usize).min(src_w);
                let weights = (first..last)
                    .map(|sx| ((b.min(sx as f64 + 1.0) - a.max(sx as f64)) / rx) as f32)
                    .collect();
                (first, weights)
            })
            .collect();
        Self { src_h, dst_w, channels, columns, ratio: src_h as f64 / dst_h as f64, acc: vec![0.0; dst_w * channels], out_row: 0, next_src: 0 }
    }

    /// One source row averaged across to the output width.
    fn across(&self, row: &[f32], src_w: usize) -> Vec<f32> {
        let c = self.channels;
        let mut out = vec![0.0f32; self.dst_w * c];
        debug_assert_eq!(row.len(), src_w * c);
        for (x, (first, weights)) in self.columns.iter().enumerate() {
            for (k, w) in weights.iter().enumerate() {
                let s = (first + k) * c;
                for ch in 0..c {
                    out[x * c + ch] += row[s + ch] * w;
                }
            }
        }
        out
    }

    /// Feed whole source rows (`src_w × channels` values each); returns the
    /// output rows that are now complete.
    pub fn push_rows(&mut self, rows: &[f32], src_w: usize) -> Vec<f32> {
        let mut done = Vec::new();
        let row_len = src_w * self.channels;
        for row in rows.chunks_exact(row_len) {
            if self.next_src >= self.src_h {
                break;
            }
            let r = self.across(row, src_w);
            let (y0, y1) = (self.next_src as f64, self.next_src as f64 + 1.0);
            let boundary = (self.out_row + 1) as f64 * self.ratio;
            if y1 <= boundary + 1e-9 {
                // The whole source row belongs to the current output row.
                for (a, v) in self.acc.iter_mut().zip(&r) {
                    *a += v / self.ratio as f32;
                }
                if y1 >= boundary - 1e-9 {
                    self.finish_row(&mut done);
                }
            } else {
                // It straddles the boundary: split it between this row and the next.
                let first = (boundary - y0) as f32;
                for (a, v) in self.acc.iter_mut().zip(&r) {
                    *a += v * first / self.ratio as f32;
                }
                self.finish_row(&mut done);
                let rest = (y1 - boundary) as f32;
                for (a, v) in self.acc.iter_mut().zip(&r) {
                    *a += v * rest / self.ratio as f32;
                }
            }
            self.next_src += 1;
        }
        done
    }

    fn finish_row(&mut self, done: &mut Vec<f32>) {
        done.extend_from_slice(&self.acc);
        self.acc.iter_mut().for_each(|a| *a = 0.0);
        self.out_row += 1;
    }

}

#[cfg(test)]
impl Downscaler {
    fn rows_done(&self) -> usize {
        self.out_row
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic pseudo-random image.
    fn image(w: usize, h: usize, c: usize) -> Vec<f32> {
        let mut s = 12345u32;
        (0..w * h * c)
            .map(|_| {
                s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                (s >> 8) as f32 / (1u32 << 24) as f32
            })
            .collect()
    }

    /// The definition, computed the slow way: each output pixel is the mean of
    /// the source rectangle it covers, with fractional edges.
    fn naive(src: &[f32], sw: usize, sh: usize, dw: usize, dh: usize, c: usize) -> Vec<f32> {
        let (rx, ry) = (sw as f64 / dw as f64, sh as f64 / dh as f64);
        let mut out = vec![0f32; dw * dh * c];
        for y in 0..dh {
            for x in 0..dw {
                let mut acc = vec![0f64; c];
                for sy in 0..sh {
                    let wy = ((((y + 1) as f64 * ry).min(sy as f64 + 1.0)) - (y as f64 * ry).max(sy as f64)).max(0.0);
                    if wy <= 0.0 { continue; }
                    for sx in 0..sw {
                        let wx = ((((x + 1) as f64 * rx).min(sx as f64 + 1.0)) - (x as f64 * rx).max(sx as f64)).max(0.0);
                        if wx <= 0.0 { continue; }
                        for ch in 0..c {
                            acc[ch] += src[(sy * sw + sx) * c + ch] as f64 * wx * wy;
                        }
                    }
                }
                for ch in 0..c {
                    out[(y * dw + x) * c + ch] = (acc[ch] / (rx * ry)) as f32;
                }
            }
        }
        out
    }

    fn run(src: &[f32], sw: usize, sh: usize, dw: usize, dh: usize, c: usize, chunk_rows: usize) -> Vec<f32> {
        let mut d = Downscaler::new(sw, sh, dw, dh, c);
        let mut out = Vec::new();
        for rows in src.chunks(chunk_rows * sw * c) {
            out.extend(d.push_rows(rows, sw));
        }
        assert_eq!(d.rows_done(), dh, "emitted {} of {dh} rows", d.rows_done());
        out
    }

    fn close(a: &[f32], b: &[f32]) {
        assert_eq!(a.len(), b.len());
        let worst = a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
        assert!(worst < 2e-5, "worst difference {worst}");
    }

    #[test]
    fn it_matches_the_definition_at_awkward_ratios() {
        for (sw, sh, dw, dh) in [(40, 30, 13, 9), (37, 41, 10, 11), (64, 64, 32, 32), (50, 20, 7, 20), (23, 23, 22, 22)] {
            let src = image(sw, sh, 3);
            close(&run(&src, sw, sh, dw, dh, 3, 7), &naive(&src, sw, sh, dw, dh, 3));
        }
    }

    #[test]
    fn however_the_rows_arrive_the_result_is_the_same() {
        let src = image(60, 45, 3);
        let whole = run(&src, 60, 45, 17, 12, 3, 45);
        for chunk in [1, 2, 5, 16, 44] {
            close(&run(&src, 60, 45, 17, 12, 3, chunk), &whole);
        }
    }

    #[test]
    fn halving_is_the_mean_of_two_by_two() {
        let src = [1.0f32, 3.0, 5.0, 7.0, 2.0, 4.0, 6.0, 8.0];
        let out = run(&src, 4, 2, 2, 1, 1, 2);
        assert_eq!(out, [(1.0 + 3.0 + 2.0 + 4.0) / 4.0, (5.0 + 7.0 + 6.0 + 8.0) / 4.0]);
    }

    #[test]
    fn brightness_is_preserved_and_flat_stays_flat() {
        let flat = vec![0.37f32; 90 * 70 * 3];
        assert!(run(&flat, 90, 70, 31, 23, 3, 10).iter().all(|v| (v - 0.37).abs() < 1e-5));
        let src = image(90, 70, 3);
        let out = run(&src, 90, 70, 31, 23, 3, 10);
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        assert!((mean(&src) - mean(&out)).abs() < 0.005, "{} vs {}", mean(&src), mean(&out));
    }

    #[test]
    fn a_fine_grid_averages_to_grey() {
        // 1-pixel black and white stripes: any whole number of pairs averages to 0.5.
        let src: Vec<f32> = (0..64 * 64).map(|i| if (i % 64) % 2 == 0 { 0.0 } else { 1.0 }).collect();
        assert!(run(&src, 64, 64, 8, 8, 1, 8).iter().all(|v| (v - 0.5).abs() < 1e-5));
    }

    #[test]
    fn no_change_in_size_changes_nothing() {
        let src = image(20, 10, 3);
        close(&run(&src, 20, 10, 20, 10, 3, 3), &src);
    }
}
