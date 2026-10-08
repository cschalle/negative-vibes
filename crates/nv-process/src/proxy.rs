//! The editing proxy: a raw scan decoded once to linear `f32` and reduced to
//! a few megapixels, so sliders re-render in milliseconds whatever the scan
//! resolution (DESIGN.md §7.2). Full-resolution work (1:1, export) goes
//! through [`Proxy::from_raw`] with a larger budget.

use std::sync::atomic::{AtomicU64, Ordering};

use nv_image::{read_sample, FrameBuf};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Default proxy size: about what a canvas shows, and fast to re-render.
pub const DEFAULT_PROXY_PIXELS: usize = 2_000_000;

/// Builds a [`Proxy`] from rows that arrive one at a time, so a scan on disk
/// can be reduced without ever being held whole. Rows are linear RGB in
/// `[0, 1]`, `width × 3` values each, in any order and any number of times
/// per row index at most once.
pub struct ProxyBuilder {
    width: usize,
    factor: usize,
    out_w: usize,
    out_h: usize,
    acc: Vec<f32>,
    ir_acc: Option<Vec<f32>>,
}

impl ProxyBuilder {
    /// The same reduction [`Proxy::from_raw`] makes: the smallest whole factor
    /// that brings the pixel count to `max_pixels` or fewer.
    pub fn new(width: usize, height: usize, max_pixels: usize) -> Self {
        let mut factor = 1;
        while (width / factor) * (height / factor) > max_pixels.max(1) && factor < width.min(height).max(1) {
            factor += 1;
        }
        let (out_w, out_h) = (width / factor, height / factor);
        Self { width, factor, out_w, out_h, acc: vec![0.0; out_w * out_h * 3], ir_acc: None }
    }

    /// Also collect an infrared plane: feed it with [`add_ir_row`](Self::add_ir_row).
    pub fn with_ir(mut self) -> Self {
        self.ir_acc = Some(vec![0.0; self.out_w * self.out_h]);
        self
    }

    /// One row of the infrared plane, `width` values.
    pub fn add_ir_row(&mut self, y: usize, row: &[f32]) {
        let oy = y / self.factor;
        let Some(acc) = self.ir_acc.as_mut() else { return };
        if oy >= self.out_h || row.len() < self.width {
            return;
        }
        for ox in 0..self.out_w {
            for x in ox * self.factor..(ox + 1) * self.factor {
                acc[oy * self.out_w + ox] += row[x];
            }
        }
    }

    pub fn add_row(&mut self, y: usize, row: &[f32]) {
        let oy = y / self.factor;
        if oy >= self.out_h || row.len() < self.width * 3 {
            return; // a ragged edge row, as in from_raw
        }
        for ox in 0..self.out_w {
            let o = (oy * self.out_w + ox) * 3;
            for x in ox * self.factor..(ox + 1) * self.factor {
                for c in 0..3 {
                    self.acc[o + c] += row[x * 3 + c];
                }
            }
        }
    }

    pub fn finish(self) -> Proxy {
        let norm = 1.0 / (self.factor * self.factor) as f32;
        let pixels = self.acc.into_iter().map(|v| v * norm).collect();
        let p = Proxy::new(self.out_w, self.out_h, self.factor, pixels);
        match self.ir_acc {
            Some(ir) => p.with_ir(ir.into_iter().map(|v| v * norm).collect()),
            None => p,
        }
    }
}

/// Linear RGB in `[0, 1]`, three `f32` per pixel, row-major.
#[derive(Debug, Clone)]
pub struct Proxy {
    /// Distinct for every proxy built, so caches can tell pictures apart.
    pub id: u64,
    pub width: usize,
    pub height: usize,
    /// How many raw pixels wide and high one proxy pixel covers.
    pub factor: usize,
    pub pixels: Vec<f32>,
    /// The infrared plane, linear in `[0, 1]`, one value per pixel, when the
    /// scan has one (the fourth sample of an RGB+IR raw).
    pub ir: Option<Vec<f32>>,
}

impl Proxy {
    pub fn new(width: usize, height: usize, factor: usize, pixels: Vec<f32>) -> Self {
        Self { id: NEXT_ID.fetch_add(1, Ordering::Relaxed), width, height, factor, pixels, ir: None }
    }

    /// The same proxy with an infrared plane (`width × height` values).
    pub fn with_ir(mut self, ir: Vec<f32>) -> Self {
        debug_assert_eq!(ir.len(), self.width * self.height);
        self.ir = Some(ir);
        self
    }

    /// Decode `raw`, box-averaging by the smallest whole factor that brings
    /// the pixel count to `max_pixels` or fewer. Grey input becomes RGB.
    pub fn from_raw(raw: &FrameBuf, max_pixels: usize) -> Self {
        let (rw, rh) = (raw.width as usize, raw.effective_height() as usize);
        if rw == 0 || rh == 0 {
            return Self::new(0, 0, 1, Vec::new());
        }
        let mut factor = 1;
        while (rw / factor) * (rh / factor) > max_pixels.max(1) && factor < rw.min(rh) {
            factor += 1;
        }
        let (w, h) = (rw / factor, rh / factor);

        let bps = raw.depth as usize / 8;
        let ch = raw.channels as usize;
        let bpr = raw.bytes_per_row;
        let src = raw.as_bytes();
        let scale = if bps == 2 { 65535.0f32 } else { 255.0 };
        let norm = 1.0 / (scale * (factor * factor) as f32);

        let mut pixels = vec![0.0f32; w * h * 3];
        let mut ir = (ch == 4).then(|| vec![0.0f32; w * h]);
        for y in 0..h {
            for x in 0..w {
                let mut acc = [0f32; 4];
                for dy in 0..factor {
                    let row = (y * factor + dy) * bpr;
                    for dx in 0..factor {
                        let off = row + (x * factor + dx) * ch * bps;
                        let r = read_sample(src, off, bps) as f32;
                        acc[0] += r;
                        acc[1] += if ch >= 2 { read_sample(src, off + bps, bps) as f32 } else { r };
                        acc[2] += if ch >= 3 { read_sample(src, off + 2 * bps, bps) as f32 } else { r };
                        if ch == 4 {
                            acc[3] += read_sample(src, off + 3 * bps, bps) as f32;
                        }
                    }
                }
                let o = (y * w + x) * 3;
                pixels[o] = acc[0] * norm;
                pixels[o + 1] = acc[1] * norm;
                pixels[o + 2] = acc[2] * norm;
                if let Some(ir) = ir.as_mut() {
                    ir[y * w + x] = acc[3] * norm;
                }
            }
        }
        let p = Self::new(w, h, factor, pixels);
        match ir { Some(ir) => p.with_ir(ir), None => p }
    }

    /// Decode the `w × h` block of `raw` at `(x0, y0)` at full resolution.
    /// The block is clipped to the scan.
    pub fn from_raw_region(raw: &FrameBuf, x0: usize, y0: usize, w: usize, h: usize) -> Self {
        let (rw, rh) = (raw.width as usize, raw.effective_height() as usize);
        let (x1, y1) = ((x0 + w).min(rw), (y0 + h).min(rh));
        if x0 >= x1 || y0 >= y1 {
            return Self::new(0, 0, 1, Vec::new());
        }
        let (w, h) = (x1 - x0, y1 - y0);
        let bps = raw.depth as usize / 8;
        let ch = raw.channels as usize;
        let src = raw.as_bytes();
        let scale = 1.0 / if bps == 2 { 65535.0f32 } else { 255.0 };
        let mut pixels = vec![0.0f32; w * h * 3];
        let mut ir = (ch == 4).then(|| vec![0.0f32; w * h]);
        for y in 0..h {
            for x in 0..w {
                let off = (y0 + y) * raw.bytes_per_row + (x0 + x) * ch * bps;
                let r = read_sample(src, off, bps) as f32 * scale;
                let o = (y * w + x) * 3;
                pixels[o] = r;
                pixels[o + 1] = if ch >= 2 { read_sample(src, off + bps, bps) as f32 * scale } else { r };
                pixels[o + 2] = if ch >= 3 { read_sample(src, off + 2 * bps, bps) as f32 * scale } else { r };
                if let Some(ir) = ir.as_mut() {
                    ir[y * w + x] = read_sample(src, off + 3 * bps, bps) as f32 * scale;
                }
            }
        }
        let p = Self::new(w, h, 1, pixels);
        match ir { Some(ir) => p.with_ir(ir), None => p }
    }

    /// A small RGBA8 picture of the proxy, longest side `max_side`, averaged
    /// and given the display curve the sidebar thumbnails use (the square
    /// root of linear light). Returns `(width, height, pixels)`.
    pub fn thumbnail_rgba(&self, max_side: u32) -> (u32, u32, Vec<u8>) {
        let (iw, ih) = (self.width, self.height);
        if iw == 0 || ih == 0 || max_side == 0 {
            return (0, 0, Vec::new());
        }
        let scale = (max_side as f64 / iw.max(ih) as f64).min(1.0);
        let (ow, oh) = (((iw as f64 * scale).round() as usize).max(1), ((ih as f64 * scale).round() as usize).max(1));
        let mut out = Vec::with_capacity(ow * oh * 4);
        for oy in 0..oh {
            let (y0, y1) = (oy * ih / oh, ((oy + 1) * ih / oh).max(oy * ih / oh + 1));
            for ox in 0..ow {
                let (x0, x1) = (ox * iw / ow, ((ox + 1) * iw / ow).max(ox * iw / ow + 1));
                let mut acc = [0f32; 3];
                for y in y0..y1 {
                    for x in x0..x1 {
                        for c in 0..3 {
                            acc[c] += self.pixels[(y * iw + x) * 3 + c];
                        }
                    }
                }
                let n = ((y1 - y0) * (x1 - x0)) as f32;
                out.extend(acc.map(|v| ((v / n).clamp(0.0, 1.0).sqrt() * 255.0) as u8));
                out.push(255);
            }
        }
        (ow as u32, oh as u32, out)
    }

    /// The part of the proxy inside `rect` (`[x0, y0, x1, y1]`, fractions).
    pub fn crop(&self, rect: [f32; 4]) -> Proxy {
        let px = |f: f32, n: usize| ((f.clamp(0.0, 1.0) * n as f32).round() as usize).min(n);
        let (x0, x1) = (px(rect[0], self.width), px(rect[2], self.width));
        let (y0, y1) = (px(rect[1], self.height), px(rect[3], self.height));
        if x1 <= x0 || y1 <= y0 {
            return Proxy::new(0, 0, self.factor, Vec::new());
        }
        let mut pixels = Vec::with_capacity((x1 - x0) * (y1 - y0) * 3);
        for y in y0..y1 {
            pixels.extend_from_slice(&self.pixels[(y * self.width + x0) * 3..(y * self.width + x1) * 3]);
        }
        let mut out = Proxy::new(x1 - x0, y1 - y0, self.factor, pixels);
        if let Some(ir) = &self.ir {
            let mut v = Vec::with_capacity((x1 - x0) * (y1 - y0));
            for y in y0..y1 {
                v.extend_from_slice(&ir[y * self.width + x0..y * self.width + x1]);
            }
            out = out.with_ir(v);
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.pixels.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: u32, h: u32, v: u16) -> FrameBuf {
        let mut b = FrameBuf::new(w, Some(h), 16, 3);
        for _ in 0..w * h * 3 {
            b.append(&v.to_ne_bytes());
        }
        b
    }

    #[test]
    fn small_scans_are_not_reduced() {
        let p = Proxy::from_raw(&flat(40, 30, 65535), 10_000);
        assert_eq!((p.width, p.height, p.factor), (40, 30, 1));
        assert!((p.pixels[0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn large_scans_are_box_averaged_to_the_budget() {
        let p = Proxy::from_raw(&flat(400, 300, 32768), 10_000);
        assert!(p.width * p.height <= 10_000, "{}x{}", p.width, p.height);
        assert!(p.factor >= 4);
        assert!((p.pixels[0] - 0.5).abs() < 1e-3);
    }

    #[test]
    fn building_from_rows_gives_the_same_proxy_as_from_raw() {
        let (w, h) = (97u32, 61u32);
        let mut raw = FrameBuf::new(w, Some(h), 16, 3);
        for i in 0..w * h * 3 {
            raw.append(&((i.wrapping_mul(2654435761) >> 12) as u16).to_ne_bytes());
        }
        for max in [usize::MAX, 1500, 200] {
            let whole = Proxy::from_raw(&raw, max);
            let mut b = ProxyBuilder::new(w as usize, h as usize, max);
            for y in 0..h as usize {
                let row: Vec<f32> = (0..w as usize * 3).map(|i| read_sample(raw.as_bytes(), (y * w as usize * 3 + i) * 2, 2) as f32 / 65535.0).collect();
                b.add_row(y, &row);
            }
            let built = b.finish();
            assert_eq!((built.width, built.height, built.factor), (whole.width, whole.height, whole.factor), "max {max}");
            let worst = built.pixels.iter().zip(&whole.pixels).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
            assert!(worst < 1e-5, "max {max}: {worst}");
        }
    }

    #[test]
    fn a_proxy_thumbnail_is_small_and_keeps_the_aspect() {
        let p = Proxy::new(400, 100, 1, vec![0.25; 400 * 100 * 3]);
        let (w, h, px) = p.thumbnail_rgba(40);
        assert_eq!((w, h), (40, 10));
        assert_eq!(px.len(), 40 * 10 * 4);
        assert_eq!(&px[..4], &[127, 127, 127, 255]); // sqrt(0.25) = 0.5
        assert_eq!(Proxy::new(8, 4, 1, vec![0.0; 96]).thumbnail_rgba(64).0, 8, "never enlarged");
    }

    #[test]
    fn crop_takes_the_rectangle() {
        let mut b = FrameBuf::new(4, Some(2), 8, 1);
        b.append(&[10, 20, 30, 40, 50, 60, 70, 80]);
        let p = Proxy::from_raw(&b, 100);
        let c = p.crop([0.5, 0.5, 1.0, 1.0]);
        assert_eq!((c.width, c.height), (2, 1));
        assert!((c.pixels[0] - 70.0 / 255.0).abs() < 1e-6 && (c.pixels[3] - 80.0 / 255.0).abs() < 1e-6);
        assert!(p.crop([0.5, 0.5, 0.5, 1.0]).is_empty());
    }

    #[test]
    fn averaging_blends_neighbours() {
        // Two columns, 0 and 65535: halving the width gives mid-grey.
        let mut b = FrameBuf::new(2, Some(2), 16, 1);
        for _ in 0..2 {
            b.append(&0u16.to_ne_bytes());
            b.append(&65535u16.to_ne_bytes());
        }
        let p = Proxy::from_raw(&b, 1);
        assert_eq!((p.width, p.height), (1, 1));
        assert!((p.pixels[0] - 0.5).abs() < 1e-4);
        assert_eq!(p.pixels[0], p.pixels[2], "grey is replicated to RGB");
    }
}
