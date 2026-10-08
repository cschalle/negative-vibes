//! Rotating and flipping a picture in 90° steps (DESIGN.md §7.1 stage 1).
//!
//! An [`Orientation`] is the transform from the scan to the picture shown and
//! exported: mirror the scan as `flip_h` / `flip_v` say, then turn it
//! clockwise by `rotation` quarter turns. Everything the pipeline measures
//! stays in scan coordinates; only the edges (the canvas, mouse positions,
//! exported files) go through the orientation.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Orientation {
    /// Clockwise quarter turns, 0 to 3.
    pub rotation: u8,
    pub flip_h: bool,
    pub flip_v: bool,
}

/// A user's edit to the picture as shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayOp {
    RotateCw,
    RotateCcw,
    FlipHorizontal,
    FlipVertical,
}

impl Orientation {
    pub const IDENTITY: Orientation = Orientation { rotation: 0, flip_h: false, flip_v: false };

    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }

    fn quarter(&self) -> u8 {
        self.rotation % 4
    }

    /// Whether width and height trade places.
    pub fn swaps_axes(&self) -> bool {
        self.quarter() % 2 == 1
    }

    /// Size of the picture made from a `sw × sh` scan.
    pub fn output_size(&self, sw: usize, sh: usize) -> (usize, usize) {
        if self.swaps_axes() { (sh, sw) } else { (sw, sh) }
    }

    /// The scan pixel that ends up at `(ox, oy)` in the picture.
    pub fn source_of(&self, ox: usize, oy: usize, sw: usize, sh: usize) -> (usize, usize) {
        // Undo the rotation first: where was this pixel in the mirrored scan?
        let (fx, fy) = match self.quarter() {
            0 => (ox, oy),
            1 => (oy, sh - 1 - ox),
            2 => (sw - 1 - ox, sh - 1 - oy),
            _ => (sw - 1 - oy, ox),
        };
        (if self.flip_h { sw - 1 - fx } else { fx }, if self.flip_v { sh - 1 - fy } else { fy })
    }

    /// Where scan pixel `(sx, sy)` ends up in the picture: the inverse of
    /// [`source_of`](Self::source_of).
    pub fn output_of(&self, sx: usize, sy: usize, sw: usize, sh: usize) -> (usize, usize) {
        let fx = if self.flip_h { sw - 1 - sx } else { sx };
        let fy = if self.flip_v { sh - 1 - sy } else { sy };
        match self.quarter() {
            0 => (fx, fy),
            1 => (sh - 1 - fy, fx),
            2 => (sw - 1 - fx, sh - 1 - fy),
            _ => (fy, sw - 1 - fx),
        }
    }

    /// Reorder `src` (`w × h` pixels of `channels` values each) into the picture.
    pub fn apply<T: Copy>(&self, src: &[T], w: usize, h: usize, channels: usize) -> (usize, usize, Vec<T>) {
        if self.is_identity() || src.len() < w * h * channels {
            return (w, h, src.to_vec());
        }
        let (ow, oh) = self.output_size(w, h);
        let mut out = Vec::with_capacity(ow * oh * channels);
        for oy in 0..oh {
            for ox in 0..ow {
                let (sx, sy) = self.source_of(ox, oy, w, h);
                out.extend_from_slice(&src[(sy * w + sx) * channels..][..channels]);
            }
        }
        (ow, oh, out)
    }

    /// The orientation after the user does `op` to the picture as shown.
    ///
    /// Found by trying: apply `op` to a small lopsided picture made from the
    /// current orientation, and pick the orientation that makes the same
    /// picture from the scan. Sixteen candidates, so there is nothing to get
    /// subtly wrong in a table of cases.
    pub fn then(&self, op: DisplayOp) -> Orientation {
        let (w, h) = (3usize, 2usize);
        let scan: Vec<u8> = (0..(w * h) as u8).collect();
        let (dw, dh, shown) = self.apply(&scan, w, h, 1);
        let extra = match op {
            DisplayOp::RotateCw => Orientation { rotation: 1, ..Self::IDENTITY },
            DisplayOp::RotateCcw => Orientation { rotation: 3, ..Self::IDENTITY },
            DisplayOp::FlipHorizontal => Orientation { flip_h: true, ..Self::IDENTITY },
            DisplayOp::FlipVertical => Orientation { flip_v: true, ..Self::IDENTITY },
        };
        let (_, _, wanted) = extra.apply(&shown, dw, dh, 1);
        let (ew, eh) = extra.output_size(dw, dh);
        Self::find(&scan, w, h, (ew, eh), &wanted).unwrap_or(*self)
    }

    /// The same transform in one fixed form. A half turn is also both flips,
    /// so there are sixteen ways to write eight different transforms.
    pub fn canonical(&self) -> Orientation {
        let (w, h) = (3usize, 2usize);
        let scan: Vec<u8> = (0..(w * h) as u8).collect();
        let (ow, oh, made) = self.apply(&scan, w, h, 1);
        Self::find(&scan, w, h, (ow, oh), &made).unwrap_or(*self)
    }

    /// The first orientation (in a fixed order) that makes `wanted` from `scan`.
    fn find(scan: &[u8], w: usize, h: usize, size: (usize, usize), wanted: &[u8]) -> Option<Orientation> {
        for rotation in 0..4u8 {
            for flip_h in [false, true] {
                for flip_v in [false, true] {
                    let o = Orientation { rotation, flip_h, flip_v };
                    let (cw, ch, got) = o.apply(scan, w, h, 1);
                    if (cw, ch) == size && got == wanted {
                        return Some(o);
                    }
                }
            }
        }
        None
    }

    /// A rectangle `[x0, y0, x1, y1]` in fractions of the picture, as fractions
    /// of the scan.
    pub fn rect_to_source(&self, r: [f64; 4]) -> [f64; 4] {
        self.map_rect(r, true)
    }

    /// A rectangle in fractions of the scan, as fractions of the picture.
    pub fn rect_from_source(&self, r: [f64; 4]) -> [f64; 4] {
        self.map_rect(r, false)
    }

    fn map_rect(&self, r: [f64; 4], to_source: bool) -> [f64; 4] {
        // Orientations only permute and mirror the axes, so mapping the two
        // corners (as points in the unit square) is enough.
        let map = |x: f64, y: f64| -> (f64, f64) {
            let (fx, fy) = if to_source { (x, y) } else { (if self.flip_h { 1.0 - x } else { x }, if self.flip_v { 1.0 - y } else { y }) };
            let (gx, gy) = if to_source {
                match self.quarter() {
                    0 => (fx, fy),
                    1 => (fy, 1.0 - fx),
                    2 => (1.0 - fx, 1.0 - fy),
                    _ => (1.0 - fy, fx),
                }
            } else {
                match self.quarter() {
                    0 => (fx, fy),
                    1 => (1.0 - fy, fx),
                    2 => (1.0 - fx, 1.0 - fy),
                    _ => (fy, 1.0 - fx),
                }
            };
            if to_source { (if self.flip_h { 1.0 - gx } else { gx }, if self.flip_v { 1.0 - gy } else { gy }) } else { (gx, gy) }
        };
        let (a, b) = (map(r[0], r[1]), map(r[2], r[3]));
        [a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Vec<Orientation> {
        (0..4u8).flat_map(|r| [false, true].into_iter().flat_map(move |h| [false, true].map(move |v| Orientation { rotation: r, flip_h: h, flip_v: v }))).collect()
    }

    const SCAN: [u8; 6] = [1, 2, 3, 4, 5, 6]; // 3 wide, 2 high: 1 2 3 / 4 5 6

    #[test]
    fn rotating_clockwise_turns_the_first_row_into_the_last_column() {
        let o = Orientation { rotation: 1, ..Orientation::IDENTITY };
        let (w, h, px) = o.apply(&SCAN, 3, 2, 1);
        assert_eq!((w, h), (2, 3));
        assert_eq!(px, [4, 1, 5, 2, 6, 3]); // 4 1 / 5 2 / 6 3
        let (_, _, px) = Orientation { rotation: 3, ..Orientation::IDENTITY }.apply(&SCAN, 3, 2, 1);
        assert_eq!(px, [3, 6, 2, 5, 1, 4]);
        let (_, _, px) = Orientation { rotation: 2, ..Orientation::IDENTITY }.apply(&SCAN, 3, 2, 1);
        assert_eq!(px, [6, 5, 4, 3, 2, 1]);
    }

    #[test]
    fn flips_mirror_the_scan_first() {
        let (_, _, h) = Orientation { flip_h: true, ..Orientation::IDENTITY }.apply(&SCAN, 3, 2, 1);
        assert_eq!(h, [3, 2, 1, 6, 5, 4]);
        let (_, _, v) = Orientation { flip_v: true, ..Orientation::IDENTITY }.apply(&SCAN, 3, 2, 1);
        assert_eq!(v, [4, 5, 6, 1, 2, 3]);
        // Mirrored, then turned: the order matters.
        let (_, _, both) = Orientation { rotation: 1, flip_h: true, flip_v: false }.apply(&SCAN, 3, 2, 1);
        assert_eq!(both, [6, 3, 5, 2, 4, 1]);
    }

    #[test]
    fn source_of_and_output_of_are_inverses_for_every_orientation() {
        let (sw, sh) = (7usize, 5usize);
        for o in all() {
            let (ow, oh) = o.output_size(sw, sh);
            let mut seen = std::collections::HashSet::new();
            for oy in 0..oh {
                for ox in 0..ow {
                    let (sx, sy) = o.source_of(ox, oy, sw, sh);
                    assert!(sx < sw && sy < sh, "{o:?}");
                    assert_eq!(o.output_of(sx, sy, sw, sh), (ox, oy), "{o:?}");
                    assert!(seen.insert((sx, sy)), "{o:?} used a scan pixel twice");
                }
            }
            assert_eq!(seen.len(), sw * sh);
        }
    }

    #[test]
    fn the_four_turns_and_two_flips_come_back_to_where_they_began() {
        for o in all() {
            let same = |a: Orientation, b: Orientation| a.canonical() == b.canonical();
            let spun = (0..4).fold(o, |o, _| o.then(DisplayOp::RotateCw));
            assert!(same(spun, o), "{o:?}");
            assert!(same(o.then(DisplayOp::RotateCw).then(DisplayOp::RotateCcw), o));
            assert!(same(o.then(DisplayOp::FlipHorizontal).then(DisplayOp::FlipHorizontal), o));
            assert!(same(o.then(DisplayOp::FlipVertical).then(DisplayOp::FlipVertical), o));
        }
    }

    /// The point of `then`: an edit to the picture as shown, applied to the
    /// scan through the new orientation, gives the edited picture.
    #[test]
    fn an_edit_to_the_shown_picture_is_the_new_orientation_of_the_scan() {
        for o in all() {
            let (dw, dh, shown) = o.apply(&SCAN, 3, 2, 1);
            for (op, via) in [
                (DisplayOp::RotateCw, Orientation { rotation: 1, ..Orientation::IDENTITY }),
                (DisplayOp::RotateCcw, Orientation { rotation: 3, ..Orientation::IDENTITY }),
                (DisplayOp::FlipHorizontal, Orientation { flip_h: true, ..Orientation::IDENTITY }),
                (DisplayOp::FlipVertical, Orientation { flip_v: true, ..Orientation::IDENTITY }),
            ] {
                let (_, _, edited) = via.apply(&shown, dw, dh, 1);
                let (_, _, direct) = o.then(op).apply(&SCAN, 3, 2, 1);
                assert_eq!(direct, edited, "{o:?} then {op:?}");
            }
        }
    }

    #[test]
    fn a_half_turn_and_two_flips_are_the_same_transform() {
        let half = Orientation { rotation: 2, ..Orientation::IDENTITY };
        let flips = Orientation { flip_h: true, flip_v: true, ..Orientation::IDENTITY };
        assert_ne!(half, flips);
        assert_eq!(half.canonical(), flips.canonical());
        assert_eq!(half.apply(&SCAN, 3, 2, 1), flips.apply(&SCAN, 3, 2, 1));
        assert!(Orientation::IDENTITY.canonical().is_identity());
    }

    #[test]
    fn rectangles_map_between_the_scan_and_the_picture() {
        let r = [0.1, 0.2, 0.4, 0.9];
        for o in all() {
            let back = o.rect_to_source(o.rect_from_source(r));
            for i in 0..4 {
                assert!((back[i] - r[i]).abs() < 1e-12, "{o:?} {back:?}");
            }
        }
        // A quarter turn clockwise: the scan's top-left corner region ends up top-right.
        let o = Orientation { rotation: 1, ..Orientation::IDENTITY };
        let shown = o.rect_from_source([0.0, 0.0, 0.5, 0.25]);
        assert!((shown[0] - 0.75).abs() < 1e-12 && (shown[2] - 1.0).abs() < 1e-12 && shown[1] == 0.0 && (shown[3] - 0.5).abs() < 1e-12, "{shown:?}");
        // And it agrees with where pixels go.
        let (sw, sh) = (8usize, 4usize);
        let (ox, oy) = o.output_of(0, 0, sw, sh);
        assert_eq!((ox, oy), (sh - 1, 0));
    }
}
