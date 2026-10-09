//! Densitometer readings (F§5.8): the colour under a point, averaged over a
//! small square, before and after the corrections.

/// Mean of the `(2r+1)²` square of pixels around the fractional position
/// `(fx, fy)` of a `w × h` picture with `ch` samples per pixel, as three
/// channels (a single-channel picture is repeated). `None` when the picture is
/// empty or the position is off it.
pub fn sample<T: Copy + Into<f64>>(data: &[T], w: usize, h: usize, ch: usize, fx: f64, fy: f64, r: usize) -> Option<[f64; 3]> {
    if w == 0 || h == 0 || ch == 0 || data.len() < w * h * ch || !(0.0..=1.0).contains(&fx) || !(0.0..=1.0).contains(&fy) {
        return None;
    }
    let (cx, cy) = (((fx * w as f64) as usize).min(w - 1), ((fy * h as f64) as usize).min(h - 1));
    let (x0, x1) = (cx.saturating_sub(r), (cx + r).min(w - 1));
    let (y0, y1) = (cy.saturating_sub(r), (cy + r).min(h - 1));
    let mut acc = [0f64; 3];
    let mut n = 0f64;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let i = (y * w + x) * ch;
            for (c, a) in acc.iter_mut().enumerate() {
                *a += data[i + c.min(ch - 1)].into();
            }
            n += 1.0;
        }
    }
    Some([acc[0] / n, acc[1] / n, acc[2] / n])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_reads_its_pixel_and_a_radius_averages_the_square() {
        // 3 × 3, value = column * 10, three channels.
        let d: Vec<f32> = (0..9)
            .flat_map(|i| {
                let v = (i % 3) as f32 * 10.0;
                [v, v + 1.0, v + 2.0]
            })
            .collect();
        assert_eq!(sample(&d, 3, 3, 3, 0.5, 0.5, 0), Some([10.0, 11.0, 12.0]));
        assert_eq!(sample(&d, 3, 3, 3, 0.5, 0.5, 1), Some([10.0, 11.0, 12.0])); // whole picture
        let corner = sample(&d, 3, 3, 3, 0.0, 0.0, 1).unwrap(); // columns 0 and 1
        assert!((corner[0] - 5.0).abs() < 1e-9);
        assert_eq!(sample(&d, 3, 3, 3, 1.0, 1.0, 0), Some([20.0, 21.0, 22.0])); // the far edge is inside
    }

    #[test]
    fn grey_input_repeats_and_bad_input_is_none() {
        assert_eq!(sample(&[7u8, 9], 2, 1, 1, 0.9, 0.0, 0), Some([9.0; 3]));
        assert_eq!(sample::<f32>(&[], 0, 0, 3, 0.5, 0.5, 0), None);
        assert_eq!(sample(&[1.0f32; 12], 2, 2, 3, 1.5, 0.5, 0), None);
        assert_eq!(sample(&[1.0f32; 5], 2, 2, 3, 0.5, 0.5, 0), None);
    }
}
