//! The holder's aperture (DESIGN.md §7.9): which part of a scan is picture.
//!
//! The scanner's window is larger than the hole in the film holder, and the
//! frame inside that hole is rarely centred, so a scan carries two kinds of
//! border that are not picture:
//!
//! * the **holder**, an opaque mount that passes no light. On a negative it
//!   inverts to a blown white band.
//! * the **rebate**, the clear film between frames. It passes nearly all of
//!   the light, so it inverts to a solid black band.
//!
//! Both give themselves away without any guess about where the frame ought to
//! be. Measured on a 3600 dpi scan of a colour negative: holder lines pass
//! 0.2-0.5% of what the film base does, while the rebate sits at 97-98% of the
//! base and is flat -- a spread of 0.025 where the picture's lines run 0.070
//! to 0.099.
//!
//! **Flatness alone is not evidence.** A sky or a white wall is a big, even
//! area of the scene, which on a negative is dense and featureless and looks
//! very much like a mount. Testing for it cropped the sky off the tulip
//! fixture. So darkness is judged only against the near-total opacity of the
//! holder, and flatness only ever helps identify the rebate, which has to be
//! clear as well as flat.

use nv_image::{FrameBuf, read_sample};

use crate::Proxy;

/// A line passing less than this share of the film base is the holder.
/// The measured margin is a factor of ten either side of it.
const OPAQUE: f32 = 0.03;

/// A line brighter than this share of the film base is bare film.
const CLEAR: f32 = 0.85;

/// A line whose spread is below this share of the median line's is flat.
/// Only ever applied together with [`CLEAR`].
const FLAT: f32 = 0.5;

/// How far past the edge of the mount the walk for the start of the picture
/// may go, as a share of the axis: the mount's edge is not sharp, and at 3600
/// dpi it took eleven rows to climb from the holder to full brightness.
///
/// A walk that runs to this limit has found no step and is discarded, because
/// the edge it would report is the limit rather than anything measured.
const PENUMBRA: f32 = 0.005;

/// Bins per line histogram.
const BINS: usize = 256;

/// Which way the lines run.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    Rows,
    Cols,
}

/// The value at `frac` of a line, from its histogram, in bins.
fn quantile(hist: &[u32], frac: f32) -> f32 {
    let total: u32 = hist.iter().sum();
    if total == 0 {
        return 0.0;
    }
    let target = ((total as f32 * frac) as u32).max(1);
    let mut seen = 0;
    for (b, n) in hist.iter().enumerate() {
        seen += n;
        if seen >= target {
            return b as f32;
        }
    }
    (BINS - 1) as f32
}

/// Somewhere to read the brightest colour of a scan pixel, in `[0, 1]`.
///
/// The same measurement runs on a raw buffer and on the reduced proxy the
/// canvas already holds, so that the editor, the loupe and the export all cut
/// at exactly the same place rather than each measuring for itself.
trait Field {
    fn size(&self) -> (usize, usize);
    fn at(&self, x: usize, y: usize) -> f32;
}

struct Raw<'a> {
    buf: &'a FrameBuf,
    bps: usize,
    ch: usize,
    colours: usize,
    h: usize,
    scale: f32,
}

impl Field for Raw<'_> {
    fn size(&self) -> (usize, usize) {
        (self.buf.width as usize, self.h)
    }
    fn at(&self, x: usize, y: usize) -> f32 {
        let o = y * self.buf.bytes_per_row + x * self.ch * self.bps;
        let data = self.buf.as_bytes();
        (0..self.colours).map(|c| read_sample(data, o + c * self.bps, self.bps)).max().unwrap_or(0) as f32 / self.scale
    }
}

impl Field for Proxy {
    fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }
    fn at(&self, x: usize, y: usize) -> f32 {
        let p = &self.pixels[(y * self.width + x) * 3..][..3];
        p[0].max(p[1]).max(p[2])
    }
}

/// A histogram of every line along `axis`, over the rectangle `x0..x1`,
/// `y0..y1` of `f`.
fn line_hists(f: &impl Field, (x0, x1): (usize, usize), (y0, y1): (usize, usize), axis: Axis) -> Vec<u32> {
    let n = if axis == Axis::Rows { y1 - y0 } else { x1 - x0 };
    let mut hists = vec![0u32; n * BINS];
    for y in y0..y1 {
        for x in x0..x1 {
            let b = ((f.at(x, y).clamp(0.0, 1.0) * (BINS - 1) as f32) as usize).min(BINS - 1);
            let line = if axis == Axis::Rows { y - y0 } else { x - x0 };
            hists[line * BINS + b] += 1;
        }
    }
    hists
}

/// The longest run of `true`, or the whole axis when there is none.
fn longest_run(pic: &[bool]) -> (usize, usize) {
    let n = pic.len();
    let (mut best, mut i) = (None::<(usize, usize)>, 0);
    while i < n {
        if !pic[i] {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < n && pic[j] {
            j += 1;
        }
        if best.is_none_or(|(a, b)| j - i > b - a + 1) {
            best = Some((i, j - 1));
        }
        i = j;
    }
    best.unwrap_or((0, n.saturating_sub(1)))
}

/// Where the picture starts and ends along one axis, as offsets into it.
fn span(hists: &[u32], n: usize, base: f32) -> (usize, usize) {
    let line = |i: usize| &hists[i * BINS..][..BINS];
    let mid: Vec<f32> = (0..n).map(|i| quantile(line(i), 0.50)).collect();
    let top: Vec<f32> = (0..n).map(|i| quantile(line(i), 0.90)).collect();
    let spread: Vec<f32> = (0..n).map(|i| top[i] - quantile(line(i), 0.10)).collect();
    let mut sorted = spread.clone();
    sorted.sort_by(f32::total_cmp);
    let reference = sorted[n / 2];

    let holder = |i: usize| top[i] < OPAQUE * base;
    let rebate = |i: usize| mid[i] > CLEAR * base && spread[i] < FLAT * reference;
    let pic: Vec<bool> = (0..n).map(|i| !(holder(i) || rebate(i))).collect();
    let (i, j) = longest_run(&pic);

    // In past the edge of the mount, as far as the picture. A walk that runs
    // to its limit found no step and is thrown away.
    let lim = ((PENUMBRA * n as f32) as usize).max(1);
    let (mut a, mut b) = (i, j);
    while a < i + lim && spread[a] < FLAT * reference {
        a += 1;
    }
    while b > j - lim && spread[b] < FLAT * reference {
        b -= 1;
    }
    (if a >= i + lim { i } else { a }, if b <= j - lim { j } else { b })
}

/// The picture in `f`, as pixels `[x0, y0, x1, y1]`, both ends inclusive.
///
/// Each axis is measured over what the other one kept, because a column that
/// runs through the holder bands above and below the film carries their
/// darkness into its own statistics and stops looking like the bare film it
/// is.
fn window(f: &impl Field) -> Option<[u32; 4]> {
    let (w, h) = f.size();
    if w < 64 || h < 64 {
        return None;
    }

    // The film base is the clearest film in the scan: the rebate is made of it.
    let whole = line_hists(f, (0, w), (0, h), Axis::Rows);
    let mut all = [0u32; BINS];
    for y in 0..h {
        for (b, n) in all.iter_mut().zip(&whole[y * BINS..][..BINS]) {
            *b += n;
        }
    }
    let base = quantile(&all, 0.995);
    if base <= 0.0 {
        return None;
    }

    let (y0, y1) = span(&whole, h, base);
    let (cx0, cx1) = span(&line_hists(f, (0, w), (y0, y1 + 1), Axis::Cols), w, base);
    let (ry0, ry1) = span(&line_hists(f, (cx0, cx1 + 1), (0, h), Axis::Rows), h, base);
    (cx1 > cx0 && ry1 > ry0).then_some([cx0 as u32, ry0 as u32, cx1 as u32, ry1 as u32])
}

/// The picture in a raw scan, as pixels `[x0, y0, x1, y1]`, ends inclusive.
pub fn picture_window(raw: &FrameBuf) -> Option<[u32; 4]> {
    let (bps, ch) = (raw.depth as usize / 8, raw.channels as usize);
    let h = raw.rows_available().min(raw.effective_height()) as usize;
    let scale = if bps == 1 { 255.0 } else { 65535.0 };
    window(&Raw { buf: raw, bps, ch, colours: ch.min(3), h, scale })
}

/// The picture in the scan `p` was reduced from, in that scan's own pixels.
///
/// The canvas already holds this proxy, so the editor pays nothing to ask, and
/// an export that asks the same proxy gets the same answer to the pixel.
pub fn picture_window_of(p: &Proxy) -> Option<[u32; 4]> {
    let [x0, y0, x1, y1] = window(p)?;
    let (fw, fh) = p.full;
    let (ox, oy) = (p.origin.0 as u32, p.origin.1 as u32);
    let f = p.factor as u32;
    let scale = |v: u32, o: u32, end: bool, lim: usize| -> u32 { (o + if end { (v + 1) * f - 1 } else { v * f }).min(lim as u32 - 1) };
    Some([scale(x0, ox, false, fw), scale(y0, oy, false, fh), scale(x1, ox, true, fw), scale(y1, oy, true, fh)])
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 200;
    const H: usize = 160;

    /// A 16-bit RGB scan whose transmission at each pixel is `f(x, y)` in
    /// [0, 1].
    fn scan(f: impl Fn(usize, usize) -> f32) -> FrameBuf {
        let mut buf = FrameBuf::new(W as u32, Some(H as u32), 16, 3);
        let mut row = Vec::with_capacity(W * 6);
        for y in 0..H {
            row.clear();
            for x in 0..W {
                let v = ((f(x, y).clamp(0.0, 1.0)) * 65535.0) as u16;
                row.extend_from_slice(&v.to_ne_bytes());
                row.extend_from_slice(&v.to_ne_bytes());
                row.extend_from_slice(&v.to_ne_bytes());
            }
            buf.append(&row);
        }
        buf
    }

    /// Deterministic texture, so a picture line has a spread and a flat one
    /// does not.
    fn grain(x: usize, y: usize) -> f32 {
        let n = (x * 7919 + y * 104_729) % 1000;
        n as f32 / 1000.0
    }

    /// The film base, and so the brightest thing a scan of film holds.
    const BASE: f32 = 0.55;

    #[test]
    fn the_holder_is_cut_off_the_top_and_bottom() {
        let holder = 20;
        let b = scan(|x, y| if y < holder || y >= H - holder { 0.001 } else { 0.10 + 0.30 * grain(x, y) });
        let [x0, y0, x1, y1] = picture_window(&b).expect("a window");
        assert!((y0 as i64 - holder as i64).abs() <= 2, "top edge at {y0}, holder ends at {holder}");
        assert!((y1 as i64 - (H - holder - 1) as i64).abs() <= 2, "bottom edge at {y1}");
        assert_eq!((x0, x1), (0, W as u32 - 1), "nothing to cut across");
    }

    #[test]
    fn the_rebate_is_cut_off_the_side() {
        let edge = 150;
        // Bare film: at the base and flat, where the picture varies.
        let b = scan(|x, y| if x >= edge { BASE } else { 0.10 + 0.30 * grain(x, y) });
        let [_, _, x1, _] = picture_window(&b).expect("a window");
        assert!(((x1 + 1) as i64 - edge as i64).abs() <= 2, "the picture ends at {x1}, the rebate starts at {edge}");
    }

    /// The failure this cost: a sky or a white wall is a big even area of the
    /// scene, which on a negative is dense and featureless and reads very much
    /// like an opaque mount. Judging a line by flatness alone cropped the top
    /// off the tulip fixture -- the building and the sky went with it.
    #[test]
    fn an_even_bright_sky_is_picture_and_is_kept() {
        let sky = 40;
        // Dense (dark) and almost perfectly flat, but nothing like opaque.
        let b = scan(|x, y| if y < sky { 0.080 + 0.0005 * grain(x, y) } else { 0.10 + 0.30 * grain(x, y) });
        let [_, y0, _, y1] = picture_window(&b).expect("a window");
        assert!(y0 <= 2, "the sky was cropped away: the picture starts at {y0}, not 0");
        assert!(y1 >= H as u32 - 3, "the bottom was cropped: ends at {y1}");
    }

    #[test]
    fn a_scan_that_is_all_picture_is_left_alone() {
        let b = scan(|x, y| 0.10 + 0.30 * grain(x, y));
        let [x0, y0, x1, y1] = picture_window(&b).expect("a window");
        assert_eq!((x0, y0, x1, y1), (0, 0, W as u32 - 1, H as u32 - 1));
    }

    #[test]
    fn a_scan_too_small_to_judge_is_refused() {
        assert!(picture_window(&FrameBuf::new(16, Some(16), 16, 3)).is_none());
    }
}
