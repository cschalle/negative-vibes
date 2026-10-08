//! The editing proxy: a raw scan decoded once to linear `f32` and reduced to
//! a few megapixels, so sliders re-render in milliseconds whatever the scan
//! resolution (DESIGN.md §7.2). Full-resolution work (1:1, export) goes
//! through [`Proxy::from_raw`] with a larger budget.

use std::sync::atomic::{AtomicU64, Ordering};

use nv_image::{read_sample, FrameBuf};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Default proxy size: about what a canvas shows, and fast to re-render.
pub const DEFAULT_PROXY_PIXELS: usize = 2_000_000;

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
}

impl Proxy {
    pub fn new(width: usize, height: usize, factor: usize, pixels: Vec<f32>) -> Self {
        Self { id: NEXT_ID.fetch_add(1, Ordering::Relaxed), width, height, factor, pixels }
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
        for y in 0..h {
            for x in 0..w {
                let mut acc = [0f32; 3];
                for dy in 0..factor {
                    let row = (y * factor + dy) * bpr;
                    for dx in 0..factor {
                        let off = row + (x * factor + dx) * ch * bps;
                        let r = read_sample(src, off, bps) as f32;
                        acc[0] += r;
                        acc[1] += if ch >= 2 { read_sample(src, off + bps, bps) as f32 } else { r };
                        acc[2] += if ch >= 3 { read_sample(src, off + 2 * bps, bps) as f32 } else { r };
                    }
                }
                let o = (y * w + x) * 3;
                pixels[o] = acc[0] * norm;
                pixels[o + 1] = acc[1] * norm;
                pixels[o + 2] = acc[2] * norm;
            }
        }
        Self::new(w, h, factor, pixels)
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
        Proxy::new(x1 - x0, y1 - y0, self.factor, pixels)
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
