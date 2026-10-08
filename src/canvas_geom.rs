//! Geometry for the frame rectangle drawn over the canvas picture.
//!
//! The picture is shown with `ContentFit::Contain`, so the image occupies a
//! centred, letterboxed rectangle inside the widget. A frame rectangle is kept
//! as fractions `[x0, y0, x1, y1]` of the *image*, which stay valid at any
//! window size and map straight onto the scanner's scan window.

/// Where the image sits inside a widget of `wid × hei`: `(x, y, w, h)`.
pub fn content_rect(wid: f64, hei: f64, img_w: f64, img_h: f64) -> (f64, f64, f64, f64) {
    if wid <= 0.0 || hei <= 0.0 || img_w <= 0.0 || img_h <= 0.0 {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let scale = (wid / img_w).min(hei / img_h);
    let (w, h) = (img_w * scale, img_h * scale);
    ((wid - w) / 2.0, (hei - h) / 2.0, w, h)
}

/// A widget-space point as image fractions, clamped to the image.
pub fn point_to_frac(px: f64, py: f64, content: (f64, f64, f64, f64)) -> Option<(f64, f64)> {
    let (cx, cy, cw, ch) = content;
    if cw <= 0.0 || ch <= 0.0 {
        return None;
    }
    Some((((px - cx) / cw).clamp(0.0, 1.0), ((py - cy) / ch).clamp(0.0, 1.0)))
}

/// Order two corner points into `[x0, y0, x1, y1]`.
pub fn normalised(a: (f64, f64), b: (f64, f64)) -> [f64; 4] {
    [a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)]
}

/// Smallest side, as a fraction of the image, that counts as a drawn frame.
/// Anything smaller was a click, not a drag.
pub const MIN_SIDE: f64 = 0.02;

/// A frame rectangle as the pixel crop `[x, y, w, h]` stored in the roll.
pub fn rect_to_crop(rect: [f64; 4], img_w: u32, img_h: u32) -> [u32; 4] {
    let (w, h) = (img_w as f64, img_h as f64);
    [
        (rect[0] * w).round() as u32,
        (rect[1] * h).round() as u32,
        ((rect[2] - rect[0]) * w).round() as u32,
        ((rect[3] - rect[1]) * h).round() as u32,
    ]
}

/// The inverse of [`rect_to_crop`]: a stored crop as fractions of the image.
pub fn crop_to_rect(crop: [u32; 4], img_w: u32, img_h: u32) -> Option<[f64; 4]> {
    if img_w == 0 || img_h == 0 || crop[2] == 0 || crop[3] == 0 {
        return None;
    }
    let (w, h) = (img_w as f64, img_h as f64);
    Some([
        (crop[0] as f64 / w).min(1.0),
        (crop[1] as f64 / h).min(1.0),
        ((crop[0] + crop[2]) as f64 / w).min(1.0),
        ((crop[1] + crop[3]) as f64 / h).min(1.0),
    ])
}

/// Next zoom level when stepping by `factor`, given the scale that fits the
/// image in the window. `None` means "fit": stepping out never goes below it.
pub fn step_zoom(current: Option<f64>, fit: f64, factor: f64) -> Option<f64> {
    let next = current.unwrap_or(fit) * factor;
    if next <= fit * 1.001 { None } else { Some(next.min(16.0)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_and_rect_round_trip() {
        let rect = [0.1, 0.25, 0.6, 0.75];
        let crop = rect_to_crop(rect, 400, 200);
        assert_eq!(crop, [40, 50, 200, 100]);
        let back = crop_to_rect(crop, 400, 200).unwrap();
        for (a, b) in rect.iter().zip(back) {
            assert!((a - b).abs() < 1e-9);
        }
    }

    #[test]
    fn empty_or_unsized_crops_have_no_rect() {
        assert_eq!(crop_to_rect([0, 0, 0, 10], 100, 100), None);
        assert_eq!(crop_to_rect([0, 0, 10, 10], 0, 100), None);
    }

    #[test]
    fn zooming_out_stops_at_fit() {
        assert_eq!(step_zoom(None, 0.2, 0.8), None);
        assert_eq!(step_zoom(Some(0.25), 0.2, 0.8), None);
        assert_eq!(step_zoom(None, 0.2, 1.25), Some(0.25));
        assert_eq!(step_zoom(Some(15.0), 0.2, 1.25), Some(16.0));
    }

    #[test]
    fn wide_widget_letterboxes_the_sides() {
        // 400×100 widget, square image: 100×100 centred.
        let (x, y, w, h) = content_rect(400.0, 100.0, 50.0, 50.0);
        assert_eq!((x, y, w, h), (150.0, 0.0, 100.0, 100.0));
    }

    #[test]
    fn tall_widget_letterboxes_top_and_bottom() {
        let (x, y, w, h) = content_rect(100.0, 400.0, 200.0, 100.0);
        assert_eq!((x, y, w, h), (0.0, 175.0, 100.0, 50.0));
    }

    #[test]
    fn points_outside_the_image_clamp_to_its_edge() {
        let c = content_rect(400.0, 100.0, 50.0, 50.0);
        assert_eq!(point_to_frac(0.0, 50.0, c), Some((0.0, 0.5)));
        assert_eq!(point_to_frac(400.0, 200.0, c), Some((1.0, 1.0)));
        assert_eq!(point_to_frac(200.0, 50.0, c), Some((0.5, 0.5)));
    }

    #[test]
    fn degenerate_sizes_give_nothing() {
        assert_eq!(point_to_frac(1.0, 1.0, content_rect(0.0, 0.0, 10.0, 10.0)), None);
    }

    #[test]
    fn corners_normalise_in_any_drag_direction() {
        assert_eq!(normalised((0.8, 0.9), (0.2, 0.1)), [0.2, 0.1, 0.8, 0.9]);
    }
}
