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
    [(rect[0] * w).round() as u32, (rect[1] * h).round() as u32, ((rect[2] - rect[0]) * w).round() as u32, ((rect[3] - rect[1]) * h).round() as u32]
}

/// The inverse of [`rect_to_crop`]: a stored crop as fractions of the image.
pub fn crop_to_rect(crop: [u32; 4], img_w: u32, img_h: u32) -> Option<[f64; 4]> {
    if img_w == 0 || img_h == 0 || crop[2] == 0 || crop[3] == 0 {
        return None;
    }
    let (w, h) = (img_w as f64, img_h as f64);
    Some([(crop[0] as f64 / w).min(1.0), (crop[1] as f64 / h).min(1.0), ((crop[0] + crop[2]) as f64 / w).min(1.0), ((crop[1] + crop[3]) as f64 / h).min(1.0)])
}

/// Next zoom level when stepping by `factor`, given the scale that fits the
/// image in the window. `None` means "fit": stepping out never goes below it.
pub fn step_zoom(current: Option<f64>, fit: f64, factor: f64) -> Option<f64> {
    let next = current.unwrap_or(fit) * factor;
    if next <= fit * 1.001 { None } else { Some(next.min(16.0)) }
}

/// Which part of a frame rectangle a press landed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grab {
    /// Inside: move the whole rectangle.
    Move,
    /// An edge or corner: `(x, y)` is -1 for the left/top side, 1 for the
    /// right/bottom side, 0 when that axis is not touched.
    Edge(i8, i8),
}

/// What a press at widget point `(px, py)` grabs of `rect` (image fractions in
/// view space), with `tol` pixels of reach around edges and corners.
pub fn hit_test(rect: [f64; 4], px: f64, py: f64, content: (f64, f64, f64, f64), tol: f64) -> Option<Grab> {
    let (cx, cy, cw, ch) = content;
    if cw <= 0.0 || ch <= 0.0 {
        return None;
    }
    let (x0, y0, x1, y1) = (cx + rect[0] * cw, cy + rect[1] * ch, cx + rect[2] * cw, cy + rect[3] * ch);
    if px < x0 - tol || px > x1 + tol || py < y0 - tol || py > y1 + tol {
        return None;
    }
    // Never let the handles swallow a small rectangle: cap the reach at a third of each side.
    let tx = tol.min((x1 - x0) / 3.0);
    let ty = tol.min((y1 - y0) / 3.0);
    let side = |p: f64, lo: f64, hi: f64, t: f64| -> i8 {
        if (p - lo).abs() <= t {
            -1
        } else if (p - hi).abs() <= t {
            1
        } else {
            0
        }
    };
    let (sx, sy) = (side(px, x0, x1, tx), side(py, y0, y1, ty));
    if sx != 0 || sy != 0 {
        Some(Grab::Edge(sx, sy))
    } else if px > x0 && px < x1 && py > y0 && py < y1 {
        Some(Grab::Move)
    } else {
        None
    }
}

/// `rect` after dragging `grab` by `(dx, dy)` image fractions. The result stays
/// inside the image and at least [`MIN_SIDE`] on each side. With `aspect` (width
/// over height in pixels, for an image of `img_aspect`), an edge drag keeps the
/// ratio, anchored at the opposite side.
pub fn drag_rect(rect: [f64; 4], grab: Grab, dx: f64, dy: f64, aspect: Option<f64>, img_aspect: f64) -> [f64; 4] {
    let [mut x0, mut y0, mut x1, mut y1] = rect;
    match grab {
        Grab::Move => {
            let (w, h) = (x1 - x0, y1 - y0);
            x0 = (x0 + dx).clamp(0.0, 1.0 - w);
            y0 = (y0 + dy).clamp(0.0, 1.0 - h);
            return [x0, y0, x0 + w, y0 + h];
        }
        Grab::Edge(sx, sy) => {
            if sx < 0 {
                x0 = (x0 + dx).clamp(0.0, x1 - MIN_SIDE);
            } else if sx > 0 {
                x1 = (x1 + dx).clamp(x0 + MIN_SIDE, 1.0);
            }
            if sy < 0 {
                y0 = (y0 + dy).clamp(0.0, y1 - MIN_SIDE);
            } else if sy > 0 {
                y1 = (y1 + dy).clamp(y0 + MIN_SIDE, 1.0);
            }
            if let Some(a) = aspect.filter(|a| *a > 0.0 && img_aspect > 0.0) {
                // Fractions of the image: width_frac / height_frac = a / img_aspect.
                let ratio = a / img_aspect;
                let (w, h) = (x1 - x0, y1 - y0);
                // Follow the axis the user moved furthest; a pure edge drag follows its own axis.
                let from_w = if sx != 0 && sy != 0 { dx.abs() >= dy.abs() } else { sx != 0 };
                let (mut nw, mut nh) = if from_w { (w, w / ratio) } else { (h * ratio, h) };
                // Fit inside the room left on the grown side(s).
                let room_w = if sx < 0 {
                    x1
                } else if sx > 0 {
                    1.0 - x0
                } else {
                    1.0
                };
                let room_h = if sy < 0 {
                    y1
                } else if sy > 0 {
                    1.0 - y0
                } else {
                    1.0
                };
                let shrink = (room_w / nw).min(room_h / nh).min(1.0);
                nw *= shrink;
                nh *= shrink;
                // Anchor opposite the grabbed side; a lone-axis drag grows about the centre of the other axis.
                if sx < 0 {
                    x0 = x1 - nw
                } else if sx > 0 {
                    x1 = x0 + nw
                } else {
                    let c = (x0 + x1) / 2.0;
                    x0 = (c - nw / 2.0).clamp(0.0, 1.0 - nw);
                    x1 = x0 + nw;
                }
                if sy < 0 {
                    y0 = y1 - nh
                } else if sy > 0 {
                    y1 = y0 + nh
                } else {
                    let c = (y0 + y1) / 2.0;
                    y0 = (c - nh / 2.0).clamp(0.0, 1.0 - nh);
                    y1 = y0 + nh;
                }
            }
        }
    }
    [x0, y0, x1, y1]
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

    const C: (f64, f64, f64, f64) = (0.0, 0.0, 1000.0, 1000.0);

    #[test]
    fn presses_find_corners_edges_and_the_inside() {
        let r = [0.2, 0.2, 0.8, 0.8];
        assert_eq!(hit_test(r, 200.0, 200.0, C, 10.0), Some(Grab::Edge(-1, -1)));
        assert_eq!(hit_test(r, 800.0, 205.0, C, 10.0), Some(Grab::Edge(1, -1)));
        assert_eq!(hit_test(r, 500.0, 795.0, C, 10.0), Some(Grab::Edge(0, 1)));
        assert_eq!(hit_test(r, 195.0, 500.0, C, 10.0), Some(Grab::Edge(-1, 0)));
        assert_eq!(hit_test(r, 500.0, 500.0, C, 10.0), Some(Grab::Move));
        assert_eq!(hit_test(r, 50.0, 50.0, C, 10.0), None);
    }

    #[test]
    fn a_small_rectangle_keeps_an_inside_to_grab() {
        let r = [0.5, 0.5, 0.52, 0.52]; // 20 px square
        assert_eq!(hit_test(r, 510.0, 510.0, C, 10.0), Some(Grab::Move));
    }

    #[test]
    fn moving_stops_at_the_image_edge_and_keeps_the_size() {
        let r = drag_rect([0.2, 0.2, 0.5, 0.6], Grab::Move, 0.9, -0.9, None, 1.0);
        for (a, b) in r.iter().zip([0.7, 0.0, 1.0, 0.4]) {
            assert!((a - b).abs() < 1e-9);
        }
    }

    #[test]
    fn resizing_cannot_invert_or_leave_the_image() {
        let r = drag_rect([0.2, 0.2, 0.5, 0.6], Grab::Edge(-1, 0), 5.0, 0.0, None, 1.0);
        assert!((r[2] - r[0] - MIN_SIDE).abs() < 1e-12);
        let r = drag_rect([0.2, 0.2, 0.5, 0.6], Grab::Edge(1, 1), 5.0, 5.0, None, 1.0);
        assert_eq!((r[2], r[3]), (1.0, 1.0));
    }

    #[test]
    fn aspect_lock_keeps_the_ratio_from_the_opposite_corner() {
        // 3:2 rectangle on a square image: width fraction 0.3, height fraction 0.2.
        let start = [0.1, 0.1, 0.4, 0.3];
        let r = drag_rect(start, Grab::Edge(1, 1), 0.3, 0.01, Some(1.5), 1.0);
        assert_eq!((r[0], r[1]), (0.1, 0.1));
        assert!(((r[2] - r[0]) / (r[3] - r[1]) - 1.5).abs() < 1e-9);
        assert!(r[2] > 0.4);
        // Dragging an edge past the image still keeps the ratio and stays inside.
        let r = drag_rect(start, Grab::Edge(1, 0), 5.0, 0.0, Some(1.5), 1.0);
        assert!(r[2] <= 1.0 && r[3] <= 1.0);
        assert!(((r[2] - r[0]) / (r[3] - r[1]) - 1.5).abs() < 1e-9);
    }
}
