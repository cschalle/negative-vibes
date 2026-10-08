//! Frame detection on an overscan preview (DESIGN.md §7.7, F§7 Find Frame).
//!
//! Film in a holder shows up as a strip of frames separated by narrow gaps
//! (the rebate of a negative, the dark bars of a slide holder). We reduce the
//! preview to a brightness profile along the strip, split it into bright and
//! dark runs, and keep the wide runs of the class that holds the picture
//! area. Frames come back as fractions `[x0, y0, x1, y1]` of the preview,
//! ordered along the strip, so they stay valid at any resolution.

/// Tunables for [`detect_frames`].
#[derive(Debug, Clone, Copy)]
pub struct DetectOptions {
    /// Fraction of each frame's size trimmed off every side (Auto Frame Inset).
    pub inset: f64,
    /// Narrowest run, as a fraction of the strip's short side, that can be a frame.
    pub min_frame: f64,
}

impl Default for DetectOptions {
    fn default() -> Self {
        Self { inset: 0.01, min_frame: 0.4 }
    }
}

/// Find frames in an 8-bit brightness image of `width × height` pixels.
///
/// Returns at least one rectangle: when no structure is found, the whole
/// image is one frame, which is also what a single-window scanner wants.
pub fn detect_frames(luma: &[u8], width: usize, height: usize, opts: DetectOptions) -> Vec<[f64; 4]> {
    let whole = vec![[0.0, 0.0, 1.0, 1.0]];
    if width < 8 || height < 8 || luma.len() < width * height {
        return whole;
    }

    // The strip runs along the long side.
    let horizontal = width >= height;
    let (long, short) = if horizontal { (width, height) } else { (height, width) };
    let at = |l: usize, s: usize| -> f64 {
        let (x, y) = if horizontal { (l, s) } else { (s, l) };
        luma[y * width + x] as f64
    };

    let mut profile: Vec<f64> = (0..long)
        .map(|l| (0..short).map(|s| at(l, s)).sum::<f64>() / short as f64)
        .collect();
    smooth(&mut profile, (long / 200).max(1));

    let t = otsu(&profile);
    let runs = runs_of(&profile, t);
    let min_len = (opts.min_frame * short as f64) as usize;

    // Frames are the wide runs of one class. Choose the class with more wide
    // runs; on a tie the one with the larger total span (gaps are narrow).
    let wide = |bright: bool| -> Vec<(usize, usize)> {
        runs.iter().filter(|r| r.2 == bright && r.1 - r.0 >= min_len).map(|r| (r.0, r.1)).collect()
    };
    let (dark, bright) = (wide(false), wide(true));
    let span = |v: &[(usize, usize)]| v.iter().map(|r| r.1 - r.0).sum::<usize>();
    let spans = if (dark.len(), span(&dark)) >= (bright.len(), span(&bright)) { dark } else { bright };

    if spans.is_empty() || (spans.len() == 1 && spans[0].1 - spans[0].0 >= long * 95 / 100) {
        return whole;
    }

    // Across the strip, trim the holder: rows where the frame-class brightness
    // holds, judged on the columns inside the frames.
    let mut out = Vec::with_capacity(spans.len());
    for (a, b) in spans {
        let cross: Vec<f64> = (0..short)
            .map(|s| (a..b).map(|l| at(l, s)).sum::<f64>() / (b - a) as f64)
            .collect();
        let (s0, s1) = widest_run(&cross).unwrap_or((0, short));
        let (l0, l1) = (a as f64 / long as f64, b as f64 / long as f64);
        let (c0, c1) = (s0 as f64 / short as f64, s1 as f64 / short as f64);
        let (l0, l1) = inset(l0, l1, opts.inset);
        let (c0, c1) = inset(c0, c1, opts.inset);
        out.push(if horizontal { [l0, c0, l1, c1] } else { [c0, l0, c1, l1] });
    }
    out
}

fn inset(a: f64, b: f64, frac: f64) -> (f64, f64) {
    let d = (b - a) * frac;
    (a + d, b - d)
}

fn smooth(v: &mut [f64], radius: usize) {
    let src = v.to_vec();
    for i in 0..v.len() {
        let (a, b) = (i.saturating_sub(radius), (i + radius + 1).min(src.len()));
        v[i] = src[a..b].iter().sum::<f64>() / (b - a) as f64;
    }
}

/// Otsu's threshold on a profile of values in 0..=255.
fn otsu(v: &[f64]) -> f64 {
    let mut hist = [0f64; 256];
    for x in v {
        hist[x.clamp(0.0, 255.0) as usize] += 1.0;
    }
    let total: f64 = hist.iter().sum();
    let sum_all: f64 = hist.iter().enumerate().map(|(i, h)| i as f64 * h).sum();
    let (mut w0, mut sum0, mut best, mut best_t) = (0.0, 0.0, -1.0, 128.0);
    for (i, h) in hist.iter().enumerate() {
        w0 += h;
        if w0 == 0.0 {
            continue;
        }
        let w1 = total - w0;
        if w1 == 0.0 {
            break;
        }
        sum0 += i as f64 * h;
        let (m0, m1) = (sum0 / w0, (sum_all - sum0) / w1);
        let between = w0 * w1 * (m0 - m1) * (m0 - m1);
        if between > best {
            best = between;
            best_t = i as f64 + 0.5;
        }
    }
    best_t
}

/// Runs of consecutive samples on the same side of `t`: `(start, end, above)`.
fn runs_of(v: &[f64], t: f64) -> Vec<(usize, usize, bool)> {
    let mut out = Vec::new();
    let mut start = 0;
    for i in 1..=v.len() {
        if i == v.len() || (v[i] > t) != (v[start] > t) {
            out.push((start, i, v[start] > t));
            start = i;
        }
    }
    out
}

/// Longest run on one side of the profile's own Otsu threshold, taking the
/// side whose run is longer (the picture area, not the holder).
fn widest_run(v: &[f64]) -> Option<(usize, usize)> {
    let (lo, hi) = v.iter().fold((f64::MAX, f64::MIN), |(l, h), &x| (l.min(x), h.max(x)));
    if hi - lo < 12.0 {
        return None; // no holder visible: keep everything
    }
    runs_of(v, otsu(v)).into_iter().map(|r| (r.0, r.1)).max_by_key(|r| r.1 - r.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A horizontal strip: `n` frames of `fw` px on a dark holder, `gap` px
    /// between them, `margin` px of holder above and below.
    fn strip(n: usize, fw: usize, gap: usize, margin: usize, frame: u8, holder: u8) -> (Vec<u8>, usize, usize) {
        let w = gap + n * (fw + gap);
        let h = fw * 2 / 3 + 2 * margin;
        let mut img = vec![holder; w * h];
        for i in 0..n {
            let x0 = gap + i * (fw + gap);
            for y in margin..h - margin {
                for x in x0..x0 + fw {
                    // Texture inside the frame so it is not a flat run.
                    img[y * w + x] = frame.saturating_add(((x * 7 + y * 3) % 20) as u8);
                }
            }
        }
        (img, w, h)
    }

    #[test]
    fn finds_six_frames_on_a_dark_holder() {
        let (img, w, h) = strip(6, 150, 12, 10, 150, 20);
        let f = detect_frames(&img, w, h, DetectOptions::default());
        assert_eq!(f.len(), 6);
        for pair in f.windows(2) {
            assert!(pair[0][2] <= pair[1][0], "frames overlap: {pair:?}");
        }
        // Frame 3 starts where the synthetic strip puts it, within a few px.
        let x0 = (12 + 2 * (150 + 12)) as f64 / w as f64;
        assert!((f[2][0] - x0).abs() < 0.01, "{} vs {x0}", f[2][0]);
        // The holder margin above and below is trimmed.
        assert!(f[0][1] > 0.05 && f[0][3] < 0.95, "{:?}", f[0]);
    }

    #[test]
    fn finds_frames_separated_by_bright_rebate() {
        // Negative: dark frames, bright gaps.
        let (img, w, h) = strip(4, 150, 12, 0, 20, 230);
        let f = detect_frames(&img, w, h, DetectOptions::default());
        assert_eq!(f.len(), 4);
    }

    #[test]
    fn vertical_strips_work_too() {
        let (img, w, h) = strip(3, 150, 12, 10, 150, 20);
        let mut t = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                t[x * h + y] = img[y * w + x];
            }
        }
        let f = detect_frames(&t, h, w, DetectOptions::default());
        assert_eq!(f.len(), 3);
        assert!(f[0][3] < f[1][1] + 1e-9, "ordered top to bottom");
    }

    #[test]
    fn featureless_image_is_one_frame() {
        let img = vec![100u8; 200 * 100];
        assert_eq!(detect_frames(&img, 200, 100, DetectOptions::default()), vec![[0.0, 0.0, 1.0, 1.0]]);
    }

    #[test]
    fn degenerate_input_is_one_frame() {
        assert_eq!(detect_frames(&[], 0, 0, DetectOptions::default()).len(), 1);
    }
}
