//! Several passes over one frame (F§2, DESIGN §5.6): multi-sample averages
//! identical passes to lower the scanner's noise, and multi-exposure adds a
//! longer pass whose cleaner shadows are merged with the normal pass.
//!
//! Everything here works on the bytes a scan delivers (native-order 16-bit,
//! or 8-bit, with the backend's line stride) and never touches the scanner,
//! so it is tested on synthetic passes.

/// What to scan for one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PassPlan {
    /// Identical passes to average at each exposure, 1 to [`MAX_SAMPLES`].
    pub samples: u8,
    /// A second, longer exposure (a factor of the normal one) to merge in.
    pub long_exposure: Option<f32>,
    /// The farthest a pass may be out of line with the first, in pixels.
    pub max_offset: u32,
}

pub const MAX_SAMPLES: u8 = 8;

impl PassPlan {
    pub const SINGLE: PassPlan = PassPlan { samples: 1, long_exposure: None, max_offset: 20 };

    pub fn is_single(&self) -> bool {
        self.samples <= 1 && self.long_exposure.is_none()
    }

    /// Scans the scanner makes.
    pub fn total_scans(&self) -> u32 {
        self.samples.clamp(1, MAX_SAMPLES) as u32 * if self.long_exposure.is_some() { 2 } else { 1 }
    }

    /// The same plan with the numbers brought into range.
    pub fn normalised(&self) -> PassPlan {
        PassPlan { samples: self.samples.clamp(1, MAX_SAMPLES), long_exposure: self.long_exposure.map(|e| e.clamp(1.2, 4.0)), max_offset: self.max_offset.clamp(0, 64) }
    }
}

/// The shape of a scan's bytes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub width: usize,
    pub height: usize,
    pub channels: usize,
    pub depth: u8,
    pub bytes_per_line: usize,
}

impl Layout {
    pub fn max_value(&self) -> u32 {
        if self.depth == 16 { 65535 } else { 255 }
    }

    pub fn samples(&self) -> usize {
        self.width * self.height * self.channels
    }

    fn read(&self, data: &[u8], x: usize, y: usize, c: usize) -> u32 {
        let bps = (self.depth / 8) as usize;
        let o = y * self.bytes_per_line + (x * self.channels + c) * bps;
        if bps == 2 { u16::from_ne_bytes([data[o], data[o + 1]]) as u32 } else { data[o] as u32 }
    }

    /// Whether `data` holds a whole frame of this shape.
    pub fn fits(&self, data: &[u8]) -> bool {
        self.height > 0 && self.width > 0 && data.len() >= self.bytes_per_line * (self.height - 1) + self.width * self.channels * (self.depth / 8) as usize
    }

    /// Samples as a flat `width × height × channels` vector.
    pub fn to_flat(&self, data: &[u8]) -> Vec<u32> {
        let mut out = Vec::with_capacity(self.samples());
        for y in 0..self.height {
            for x in 0..self.width {
                for c in 0..self.channels {
                    out.push(self.read(data, x, y, c));
                }
            }
        }
        out
    }

    /// Flat samples back to bytes with the line stride (the padding is zero).
    pub fn from_flat(&self, flat: &[u32]) -> Vec<u8> {
        let bps = (self.depth / 8) as usize;
        let mut out = vec![0u8; self.bytes_per_line * self.height];
        for y in 0..self.height {
            for x in 0..self.width {
                for c in 0..self.channels {
                    let v = flat[(y * self.width + x) * self.channels + c].min(self.max_value());
                    let o = y * self.bytes_per_line + (x * self.channels + c) * bps;
                    if bps == 2 {
                        out[o..o + 2].copy_from_slice(&(v as u16).to_ne_bytes());
                    } else {
                        out[o] = v as u8;
                    }
                }
            }
        }
        out
    }
}

/// A central part of the first pass, kept to line the others up with.
struct Reference {
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
    rgb: Vec<f32>,
}

const CROP: usize = 512;

fn crop_f32(layout: &Layout, data: &[u8], x0: usize, y0: usize, w: usize, h: usize, rgb: bool) -> Vec<f32> {
    let max = layout.max_value() as f32;
    let mut out = Vec::with_capacity(w * h * if rgb { 3 } else { 1 });
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            if rgb {
                for c in 0..3 {
                    out.push(layout.read(data, x, y, c.min(layout.channels - 1)) as f32 / max);
                }
            } else {
                out.push(layout.read(data, x, y, 0) as f32 / max);
            }
        }
    }
    out
}

/// Averages passes of one exposure, lined up with the first.
pub struct Accumulator {
    layout: Layout,
    sum: Vec<u32>,
    n: u32,
    max_offset: u32,
    reference: Option<Reference>,
    /// The reference came from another accumulator, so even this one's first pass is lined up.
    reference_is_foreign: bool,
    /// How far each added pass was moved, `(dx, dy)` in pixels, for the report.
    pub shifts: Vec<(f32, f32)>,
}

impl Accumulator {
    pub fn new(layout: Layout, max_offset: u32) -> Self {
        Self { layout, sum: vec![0; layout.samples()], n: 0, max_offset, reference: None, reference_is_foreign: false, shifts: Vec::new() }
    }

    pub fn layout(&self) -> Layout {
        self.layout
    }

    pub fn count(&self) -> u32 {
        self.n
    }

    /// The offset of `data` from the first pass: `data(x + dx, y + dy)` lines up with
    /// the first pass at `(x, y)`. Measured on a full-resolution crop of the
    /// middle, where it is accurate to a fraction of a pixel.
    fn offset_of(&self, data: &[u8]) -> (f32, f32) {
        let Some(r) = &self.reference else { return (0.0, 0.0) };
        let cand = crop_f32(&self.layout, data, r.x0, r.y0, r.w, r.h, false);
        nv_process::dust::estimate_shift_within(&r.rgb, &cand, r.w, r.h, 1, self.max_offset as usize).map_or((0.0, 0.0), |s| (s[0], s[1]))
    }

    /// An accumulator for another exposure of the same frame, lined up with
    /// the same reference, so its first pass is placed like any other.
    pub fn linked(&self, max_offset: u32) -> Accumulator {
        let reference = self.reference.as_ref().map(|r| Reference { x0: r.x0, y0: r.y0, w: r.w, h: r.h, rgb: r.rgb.clone() });
        Accumulator { layout: self.layout, sum: vec![0; self.layout.samples()], n: 0, max_offset, reference_is_foreign: true, reference, shifts: Vec::new() }
    }

    /// Add a pass moved by a shift decided elsewhere (the infrared plane
    /// follows the colour it was scanned with).
    pub fn add_shifted(&mut self, data: &[u8], (sx, sy): (f32, f32)) {
        let l = self.layout;
        assert!(l.fits(data), "pass does not match the frame's shape");
        let (ix, iy) = (sx.floor(), sy.floor());
        let (fx, fy) = (sx - ix, sy - iy);
        let (ix, iy) = (ix as i32, iy as i32);
        let whole = fx < 0.02 && fy < 0.02;
        for y in 0..l.height {
            let y0 = (y as i32 + iy).clamp(0, l.height as i32 - 1) as usize;
            let y1 = (y as i32 + iy + 1).clamp(0, l.height as i32 - 1) as usize;
            for x in 0..l.width {
                let x0 = (x as i32 + ix).clamp(0, l.width as i32 - 1) as usize;
                let x1 = (x as i32 + ix + 1).clamp(0, l.width as i32 - 1) as usize;
                for c in 0..l.channels {
                    let v = if whole {
                        l.read(data, x0, y0, c) as f32
                    } else {
                        // A fraction of a pixel: interpolate, rather than round and be half a pixel out.
                        let (a, b, d, e) = (l.read(data, x0, y0, c) as f32, l.read(data, x1, y0, c) as f32, l.read(data, x0, y1, c) as f32, l.read(data, x1, y1, c) as f32);
                        a * (1.0 - fx) * (1.0 - fy) + b * fx * (1.0 - fy) + d * (1.0 - fx) * fy + e * fx * fy
                    };
                    self.sum[(y * l.width + x) * l.channels + c] += (v + 0.5) as u32;
                }
            }
        }
        self.n += 1;
        self.shifts.push((sx, sy));
    }

    /// Add a pass. Returns the shift applied to it, in pixels.
    pub fn add(&mut self, data: &[u8]) -> (f32, f32) {
        let l = self.layout;
        assert!(l.fits(data), "pass does not match the frame's shape");
        let (mut sx, mut sy) = (0f32, 0f32);
        if self.n == 0 && !self.reference_is_foreign && self.max_offset > 0 && l.width >= 64 && l.height >= 64 {
            let (w, h) = (CROP.min(l.width), CROP.min(l.height));
            let (x0, y0) = ((l.width - w) / 2, (l.height - h) / 2);
            self.reference = Some(Reference { x0, y0, w, h, rgb: crop_f32(&l, data, x0, y0, w, h, true) });
        }
        if self.reference.is_some() && (self.n > 0 || self.reference_is_foreign) {
            let (dx, dy) = self.offset_of(data);
            let lim = self.max_offset as f32;
            // A shift under a fifth of a pixel is within what the estimate can tell from
            // nothing, and moving by it would only blur.
            if dx.abs() >= 0.2 && dx.abs() <= lim {
                sx = dx;
            }
            if dy.abs() >= 0.2 && dy.abs() <= lim {
                sy = dy;
            }
        }
        self.add_shifted(data, (sx, sy));
        (sx, sy)
    }

    /// The mean of what was added, as flat samples.
    pub fn mean(&self) -> Vec<u32> {
        let n = self.n.max(1);
        self.sum.iter().map(|s| (s + n / 2) / n).collect()
    }
}

/// How much of a sample from the long exposure to trust: all of it until it
/// nears full scale, none once it clips (a clipped sample is only a bound).
fn long_trust(v: u32, max: u32) -> f32 {
    let f = v as f32 / max as f32;
    let t = ((0.97 - f) / (0.97 - 0.85)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How a long-pass sample converts to the normal pass's units: a quadratic
/// `short = a + b·x + d·x²` in `x = long / max`, fitted to the picture itself.
/// The scanner's exposure setting is only nominal (the 8200i's factor of 2.5
/// measured 2.56 in the shadows and 2.61 higher up, with an offset of about
/// −270 counts), so no single factor or line fits; a gentle curve does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LongMap {
    pub a: f32,
    pub b: f32,
    pub d: f32,
    /// The average factor, for weighting.
    pub gain: f32,
    max: f32,
}

impl LongMap {
    pub fn nominal(e: f32, max: u32) -> Self {
        Self { a: 0.0, b: 1.0 / e, d: 0.0, gain: e, max: max as f32 }
    }

    /// The long sample in the normal pass's units.
    pub fn apply(&self, long: u32) -> f32 {
        let x = long as f32 / self.max;
        (self.a + self.b * x + self.d * x * x) * self.max
    }
}

/// Fit the map over the samples bright enough to matter and well short of
/// clipping in the long pass. Falls back to the nominal factor when too little
/// qualifies or the fitted curve is implausible (not rising, or far from the
/// nominal factor anywhere in the range).
pub fn fit_long_map(short: &[u32], long: &[u32], channels: usize, channel: usize, nominal: f32, max: u32) -> LongMap {
    let fallback = LongMap::nominal(nominal, max);
    let m = max as f64;
    // Normal equations for short ~ 1, x, x² with x = long / max.
    let mut ata = [[0f64; 3]; 3];
    let mut atb = [0f64; 3];
    let mut n = 0f64;
    let stride = channels.max(1);
    for (&s, &l) in short.iter().zip(long).skip(channel).step_by(stride * 2) {
        if (l as f32) < 0.75 * max as f32 && (s as f32) > 0.02 * max as f32 {
            let (x, y) = (l as f64 / m, s as f64 / m);
            let row = [1.0, x, x * x];
            for i in 0..3 {
                atb[i] += row[i] * y;
                for j in 0..3 {
                    ata[i][j] += row[i] * row[j];
                }
            }
            n += 1.0;
        }
    }
    if n < 1000.0 {
        return fallback;
    }
    // Solve the 3×3 system by Cramer's rule.
    let det3 = |m: [[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let d = det3(ata);
    if d.abs() < 1e-18 {
        return fallback;
    }
    let mut sol = [0f64; 3];
    for (c, s) in sol.iter_mut().enumerate() {
        let mut mm = ata;
        for r in 0..3 {
            mm[r][c] = atb[r];
        }
        *s = det3(mm) / d;
    }
    let map = LongMap { a: sol[0] as f32, b: sol[1] as f32, d: sol[2] as f32, gain: 0.0, max: max as f32 };
    // Plausible: rising across the range, and the local factor near the nominal one.
    let at = |x: f32| (map.a + map.b * x + map.d * x * x, map.b + 2.0 * map.d * x);
    let mut mean_slope = 0.0;
    for x in [0.05f32, 0.2, 0.4, 0.6, 0.75] {
        let (_, slope) = at(x);
        if slope <= 0.0 {
            return fallback;
        }
        let factor = 1.0 / slope;
        if factor < nominal * 0.8 || factor > nominal * 1.25 {
            return fallback;
        }
        mean_slope += slope / 5.0;
    }
    let (zero, _) = at(0.0);
    if zero.abs() > 0.03 {
        return fallback;
    }
    LongMap { gain: 1.0 / mean_slope, ..map }
}

/// How much a sample of the long pass is worth next to the same sample of the
/// normal pass, per unit of the exposure factor. Theory for shot-limited noise
/// says 1; the 8200i's long pass measured noisier than that (its noise in the
/// normal pass's units was 85 counts against 108 for the normal pass at about
/// 7500 counts, where theory gave 68), which makes the worth 0.62 per unit.
pub const LONG_WORTH: f32 = 0.62;

/// Merge a normal and a long exposure (`e` times the light, nominally) into
/// one picture in the normal exposure's units: the long pass is brought to the
/// normal one's scale by the fitted gain and offset, weighted by how
/// informative it is, and counts for nothing where it clips.
pub fn merge_exposures(short: &[u32], long: &[u32], channels: usize, e_nominal: f32, max: u32) -> Vec<u32> {
    assert_eq!(short.len(), long.len());
    let channels = channels.max(1);
    // Each colour channel has its own relation between the exposures.
    let maps: Vec<LongMap> = (0..channels).map(|c| fit_long_map(short, long, channels, c, e_nominal, max)).collect();
    short
        .iter()
        .zip(long)
        .enumerate()
        .map(|(i, (&a, &b))| {
            let map = &maps[i % channels];
            let w = map.gain * LONG_WORTH * long_trust(b, max);
            let v = (a as f32 + w * map.apply(b)) / (1.0 + w);
            // The normal pass is itself clipped at full scale; keep that as a floor.
            let v = if a >= max { a as f32 } else { v };
            (v + 0.5).clamp(0.0, max as f32) as u32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(w: usize, h: usize, ch: usize, depth: u8, pad: usize) -> Layout {
        Layout { width: w, height: h, channels: ch, depth, bytes_per_line: w * ch * (depth / 8) as usize + pad }
    }

    /// A tiny deterministic generator, uniform in [-1, 1].
    fn noise(seed: u64) -> impl FnMut() -> f32 {
        let mut s = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        move || {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((s >> 33) as f32 / (1u64 << 30) as f32) - 1.0
        }
    }

    /// Smooth, aperiodic picture in [0.1, 0.9] with structure at every scale.
    fn scene(x: f32, y: f32) -> f32 {
        let hash = |ix: i32, iy: i32| {
            let v = (ix.wrapping_mul(73856093) ^ iy.wrapping_mul(19349663)) as u32;
            (v.wrapping_mul(2654435761).rotate_left(11).wrapping_mul(40503) >> 8) as f32 / (1u32 << 24) as f32
        };
        let (gx, gy) = (x / 9.0, y / 9.0);
        let (ix, iy) = (gx.floor() as i32, gy.floor() as i32);
        let (tx, ty) = (gx - ix as f32, gy - iy as f32);
        let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
        let top = hash(ix, iy) * (1.0 - sx) + hash(ix + 1, iy) * sx;
        let bottom = hash(ix, iy + 1) * (1.0 - sx) + hash(ix + 1, iy + 1) * sx;
        0.1 + 0.8 * (top * (1.0 - sy) + bottom * sy)
    }

    /// A pass of the scene displaced by `(sx, sy)`, with noise `sigma` (in counts).
    fn make_pass(l: &Layout, scale: f32, sx: i32, sy: i32, sigma: f32, seed: u64) -> Vec<u8> {
        let mut rng = noise(seed);
        let mut flat = Vec::with_capacity(l.samples());
        for y in 0..l.height {
            for x in 0..l.width {
                for c in 0..l.channels {
                    let v = scene((x as i32 + sx) as f32, (y as i32 + sy) as f32 + 0.5 * c as f32) * scale * l.max_value() as f32;
                    flat.push((v + sigma * rng()).round().clamp(0.0, l.max_value() as f32) as u32);
                }
            }
        }
        l.from_flat(&flat)
    }

    fn rms(a: &[u32], b: &[u32]) -> f32 {
        (a.iter().zip(b).map(|(x, y)| (*x as f32 - *y as f32).powi(2)).sum::<f32>() / a.len() as f32).sqrt()
    }

    #[test]
    fn plans_count_their_scans_and_are_brought_into_range() {
        assert!(PassPlan::SINGLE.is_single());
        assert_eq!(PassPlan { samples: 3, long_exposure: Some(2.0), max_offset: 20 }.total_scans(), 6);
        assert_eq!(PassPlan { samples: 4, long_exposure: None, max_offset: 20 }.total_scans(), 4);
        let n = PassPlan { samples: 99, long_exposure: Some(9.0), max_offset: 500 }.normalised();
        assert_eq!((n.samples, n.long_exposure, n.max_offset), (MAX_SAMPLES, Some(4.0), 64));
        assert_eq!(PassPlan { samples: 0, long_exposure: Some(1.0), max_offset: 0 }.normalised().long_exposure, Some(1.2));
    }

    #[test]
    fn bytes_round_trip_through_the_flat_form_with_padding_and_both_depths() {
        for (depth, pad, ch) in [(16u8, 6usize, 3usize), (8, 3, 3), (16, 0, 1)] {
            let l = layout(7, 5, ch, depth, pad);
            let bytes = make_pass(&l, 1.0, 0, 0, 100.0, 3);
            assert!(l.fits(&bytes));
            let flat = l.to_flat(&bytes);
            assert_eq!(flat.len(), l.samples());
            assert_eq!(l.from_flat(&flat), bytes, "depth {depth} pad {pad}");
        }
        assert!(!layout(7, 5, 3, 16, 0).fits(&[0u8; 10]));
    }

    #[test]
    fn averaging_aligned_passes_lowers_the_noise_by_the_square_root_of_their_number() {
        let l = layout(200, 160, 3, 16, 4);
        let clean = l.to_flat(&make_pass(&l, 1.0, 0, 0, 0.0, 1));
        let mut acc = Accumulator::new(l, 20);
        let mut single = Vec::new();
        for k in 0..4 {
            let p = make_pass(&l, 1.0, 0, 0, 800.0, 10 + k);
            if k == 0 {
                single = l.to_flat(&p);
            }
            assert_eq!(acc.add(&p), (0.0, 0.0), "aligned passes must not be moved");
        }
        let (e1, e4) = (rms(&single, &clean), rms(&acc.mean(), &clean));
        assert!(e4 < e1 * 0.6 && e4 > e1 * 0.4, "noise {e1} -> {e4}, expected about half");
    }

    #[test]
    fn passes_out_of_line_are_lined_up_before_averaging() {
        let l = layout(300, 240, 3, 16, 0);
        let clean = l.to_flat(&make_pass(&l, 1.0, 0, 0, 0.0, 1));
        let mut acc = Accumulator::new(l, 20);
        let shifts = [(0, 0), (3, -2), (-4, 5), (1, 1)];
        for (k, (sx, sy)) in shifts.into_iter().enumerate() {
            let moved = acc.add(&make_pass(&l, 1.0, sx, sy, 300.0, 20 + k as u64));
            if k > 0 {
                assert!(
                    (moved.0 + sx as f32).abs() < 0.4 && (moved.1 + sy as f32).abs() < 0.4,
                    "pass {k}: moved {moved:?}, the shift that lines it up is the opposite of the displacement ({sx}, {sy})"
                );
            }
        }
        // Away from the borders the average matches the picture.
        let mean = acc.mean();
        let mut worst = 0f32;
        for y in 20..l.height - 20 {
            for x in 20..l.width - 20 {
                let i = (y * l.width + x) * 3;
                worst = worst.max((mean[i] as f32 - clean[i] as f32).abs());
            }
        }
        assert!(worst < 1500.0, "worst error {worst} counts after lining up");
        // Without lining up the picture would be blurred by the shifts.
        let mut loose = Accumulator::new(l, 0);
        for (k, (sx, sy)) in shifts.into_iter().enumerate() {
            loose.add(&make_pass(&l, 1.0, sx, sy, 300.0, 20 + k as u64));
        }
        assert!(rms(&loose.mean(), &clean) > rms(&mean, &clean) * 2.0);
    }

    #[test]
    fn a_linked_accumulator_places_its_first_pass_by_the_other_one_reference() {
        let l = layout(300, 240, 3, 16, 0);
        let mut short = Accumulator::new(l, 20);
        short.add(&make_pass(&l, 1.0, 0, 0, 100.0, 1));
        let mut long = short.linked(20);
        // The long exposure is three times brighter and displaced by (2, -3).
        let moved = long.add(&make_pass(&l, 0.3, 2, -3, 100.0, 2));
        assert!((moved.0 + 2.0).abs() < 0.4 && (moved.1 - 3.0).abs() < 0.4, "{moved:?}");
        assert_eq!(long.count(), 1);
        assert_eq!(short.count(), 1, "the original is untouched");
    }

    #[test]
    fn a_shift_beyond_the_limit_is_left_alone_and_tiny_images_do_not_panic() {
        let l = layout(300, 240, 3, 16, 0);
        let mut acc = Accumulator::new(l, 3);
        acc.add(&make_pass(&l, 1.0, 0, 0, 100.0, 1));
        assert_eq!(acc.add(&make_pass(&l, 1.0, 9, 0, 100.0, 2)), (0.0, 0.0));
        let small = layout(20, 20, 1, 8, 0);
        let mut a = Accumulator::new(small, 20);
        a.add(&make_pass(&small, 1.0, 0, 0, 5.0, 1));
        a.add(&make_pass(&small, 1.0, 0, 0, 5.0, 2));
        assert_eq!(a.mean().len(), 400);
        assert_eq!(a.count(), 2);
    }

    #[test]
    fn the_long_exposure_cleans_up_the_shadows_and_the_clipped_highlights_stay_right() {
        let l = layout(160, 120, 3, 16, 0);
        let e = 3.0f32;
        // A dark picture: radiance at 2% to 47% of full scale, so the long pass clips in the brighter part.
        let truth: Vec<u32> = {
            let mut v = Vec::new();
            for y in 0..l.height {
                for x in 0..l.width {
                    for _ in 0..3 {
                        v.push((((scene(x as f32, y as f32) - 0.1) / 0.8 * 0.45 + 0.02) * 65535.0) as u32);
                    }
                }
            }
            v
        };
        let mut rng = noise(5);
        // Shot-like noise, as measured: about 1.2 counts per root count.
        let sigma = 1.2 * (65535.0f32 * 0.1).sqrt();
        // The scanner's factor is only nominal: the long pass really has 3.5% more light.
        let real = e * 1.035;
        let offset = -250.0f32;
        let observe = |gain: f32, rng: &mut dyn FnMut() -> f32| -> Vec<u32> {
            truth
                .iter()
                .map(|t| {
                    let m = *t as f32 * gain + if gain > 1.0 { offset } else { 0.0 };
                    (m + 1.2 * m.max(0.0).sqrt() * rng() * 1.7).round().clamp(0.0, 65535.0) as u32
                })
                .collect()
        };
        let short = observe(1.0, &mut rng);
        let long = observe(real, &mut rng);
        let merged = merge_exposures(&short, &long, 3, e, 65535);
        // Shadows (truth below 15% of full scale): the merge is much closer to the truth.
        let dark: Vec<usize> = (0..truth.len()).filter(|i| truth[*i] < 9800).collect();
        assert!(dark.len() > 2000);
        let sub = |v: &[u32]| dark.iter().map(|i| v[*i]).collect::<Vec<_>>();
        let (e_short, e_merged) = (rms(&sub(&short), &sub(&truth)), rms(&sub(&merged), &sub(&truth)));
        assert!(e_merged < e_short * 0.8, "shadow noise {e_short} -> {e_merged}");
        // And the picture is not brightened by the nominal factor being a little off.
        let mean = |v: &[u32]| v.iter().map(|x| *x as f64).sum::<f64>() / v.len() as f64;
        assert!((mean(&sub(&merged)) / mean(&sub(&truth)) - 1.0).abs() < 0.004, "bias {}", mean(&sub(&merged)) / mean(&sub(&truth)) - 1.0);
        // Where the long pass clips (truth * 3 above full scale) the answer follows the normal pass.
        let bright: Vec<usize> = (0..truth.len()).filter(|i| truth[*i] as f32 * e > 65535.0 * 0.99).collect();
        assert!(!bright.is_empty(), "the scene must have a part where the long pass clips");
        let subb = |v: &[u32]| bright.iter().map(|i| v[*i]).collect::<Vec<_>>();
        assert!((rms(&subb(&merged), &subb(&truth)) - rms(&subb(&short), &subb(&truth))).abs() < sigma * 0.3, "clipped part must not be worse than the normal pass");
        // And no sample jumps out of range.
        assert!(merged.iter().all(|v| *v <= 65535));
    }

    #[test]
    fn the_curve_between_the_exposures_is_read_from_the_picture_and_nonsense_is_ignored() {
        let truth: Vec<u32> = (0..30000).map(|i| 3000 + (i * 7 % 30000) as u32).collect();
        // The 8200i's measured behaviour: the factor creeps up from 2.56 to 2.61, with an offset, clipping at full scale.
        let long: Vec<u32> = truth
            .iter()
            .map(|t| {
                let x = *t as f32;
                ((x * (2.56 + 0.05 * x / 33000.0) - 270.0).round().max(0.0) as u32).min(65535)
            })
            .collect();
        let map = fit_long_map(&truth, &long, 1, 0, 2.5, 65535);
        for t in [4000u32, 10000, 20000, 28000] {
            let l = ((t as f32) * (2.56 + 0.05 * t as f32 / 33000.0) - 270.0) as u32;
            assert!((map.apply(l) - t as f32).abs() < 60.0, "long {l} should be {t}, got {}", map.apply(l));
        }
        assert!((map.gain - 2.58).abs() < 0.06, "{}", map.gain);
        // Too few usable samples, or a relation far from the nominal factor: the nominal map.
        assert_eq!(fit_long_map(&truth[..100], &long[..100], 1, 0, 2.5, 65535), LongMap::nominal(2.5, 65535));
        let wrong: Vec<u32> = truth.iter().map(|t| t * 5).collect();
        assert_eq!(fit_long_map(&truth, &wrong, 1, 0, 2.5, 65535), LongMap::nominal(2.5, 65535));
        assert!((LongMap::nominal(2.0, 65535).apply(40000) - 20000.0).abs() < 1.0);
    }

    #[test]
    fn a_merge_of_identical_exposure_one_passes_is_their_average_and_full_scale_stays_full() {
        let a = vec![100u32, 65535, 0, 30000];
        let b = vec![110u32, 65535, 0, 30010];
        let m = merge_exposures(&a, &b, 1, 1.0, 65535);
        assert_eq!(m[1], 65535);
        assert_eq!(m[2], 0);
        assert!((m[0] as i32 - 105).abs() <= 1 || m[0] >= 100);
    }
}
