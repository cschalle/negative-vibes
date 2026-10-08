/// A packed pixel buffer built incrementally from SANE `DataChunk` events.
///
/// Raw data is stored exactly as received (8 or 16 bit; see `read_sample`
/// for the byte order of 16-bit samples).
/// Call `to_rgb8()` to get a display-ready sRGB u8 copy.
#[derive(Clone)]
pub struct FrameBuf {
    pub width: u32,
    /// Known height, or `None` for sheet-fed / hand scanners (`lines = -1`).
    pub height: Option<u32>,
    pub bytes_per_row: usize,
    pub depth: u8,
    pub channels: u8,
    data: Vec<u8>,
}

impl FrameBuf {
    pub fn new(width: u32, height: Option<u32>, depth: u8, channels: u8) -> Self {
        let bpr = width as usize * channels as usize * (depth as usize / 8);
        Self::with_stride(width, height, depth, channels, bpr)
    }

    /// Create a FrameBuf using the exact bytes-per-line reported by the backend.
    /// Use this when SANE's `bytes_per_line` may differ from the pixel-packed size.
    pub fn with_stride(
        width: u32,
        height: Option<u32>,
        depth: u8,
        channels: u8,
        bytes_per_line: usize,
    ) -> Self {
        let bpr = bytes_per_line;
        let capacity = height.map(|h| bpr * h as usize).unwrap_or(bpr * 1024);
        Self {
            width,
            height,
            bytes_per_row: bpr,
            depth,
            channels,
            data: Vec::with_capacity(capacity),
        }
    }

    /// Append a raw chunk (straight from `sane_read`).
    pub fn append(&mut self, chunk: &[u8]) {
        self.data.extend_from_slice(chunk);
    }

    pub fn bytes_received(&self) -> usize {
        self.data.len()
    }

    /// Current number of complete rows available.
    pub fn rows_available(&self) -> u32 {
        (self.data.len() / self.bytes_per_row) as u32
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Effective height: known height or rows received so far.
    pub fn effective_height(&self) -> u32 {
        self.height.unwrap_or_else(|| self.rows_available())
    }

    /// The `w × h` block at `(x0, y0)` as a frame of its own, clipped to this
    /// one. Empty when the block lies outside it.
    pub fn crop(&self, x0: u32, y0: u32, w: u32, h: u32) -> FrameBuf {
        let (sw, sh) = (self.width, self.rows_available().min(self.effective_height()));
        let (x1, y1) = (x0.saturating_add(w).min(sw), y0.saturating_add(h).min(sh));
        if x0 >= x1 || y0 >= y1 {
            return FrameBuf::new(0, Some(0), self.depth, self.channels);
        }
        let px = self.channels as usize * (self.depth as usize / 8);
        let mut out = FrameBuf::new(x1 - x0, Some(y1 - y0), self.depth, self.channels);
        for y in y0..y1 {
            let start = y as usize * self.bytes_per_row + x0 as usize * px;
            out.append(&self.data[start..start + (x1 - x0) as usize * px]);
        }
        out
    }

    /// A small RGBA8 copy for list thumbnails, longest side `max_side`.
    ///
    /// `region` is `[x0, y0, x1, y1]` as fractions of the image (a frame's
    /// crop); `None` takes everything. Samples are box-averaged per output
    /// pixel and given the same display curve as [`to_rgb8`](Self::to_rgb8).
    /// Returns `(width, height, pixels)`.
    pub fn thumbnail_rgba(&self, max_side: u32, region: Option<[f64; 4]>) -> (u32, u32, Vec<u8>) {
        let (iw, ih) = (self.width as usize, self.rows_available().min(self.effective_height()) as usize);
        if iw == 0 || ih == 0 || max_side == 0 {
            return (0, 0, Vec::new());
        }
        let [fx0, fy0, fx1, fy1] = region.unwrap_or([0.0, 0.0, 1.0, 1.0]);
        let x0 = ((fx0.clamp(0.0, 1.0) * iw as f64) as usize).min(iw - 1);
        let y0 = ((fy0.clamp(0.0, 1.0) * ih as f64) as usize).min(ih - 1);
        let x1 = ((fx1.clamp(0.0, 1.0) * iw as f64).ceil() as usize).clamp(x0 + 1, iw);
        let y1 = ((fy1.clamp(0.0, 1.0) * ih as f64).ceil() as usize).clamp(y0 + 1, ih);
        let (rw, rh) = (x1 - x0, y1 - y0);
        let scale = (max_side as f64 / rw.max(rh) as f64).min(1.0);
        let (ow, oh) = (((rw as f64 * scale).round() as usize).max(1), ((rh as f64 * scale).round() as usize).max(1));

        let bps = self.depth as usize / 8;
        let ch = self.channels as usize;
        let full = if bps == 2 { 65535.0f32 } else { 255.0 };
        let mut out = Vec::with_capacity(ow * oh * 4);
        for oy in 0..oh {
            let (sy0, sy1) = (y0 + oy * rh / oh, (y0 + (oy + 1) * rh / oh).max(y0 + oy * rh / oh + 1));
            for ox in 0..ow {
                let (sx0, sx1) = (x0 + ox * rw / ow, (x0 + (ox + 1) * rw / ow).max(x0 + ox * rw / ow + 1));
                let mut acc = [0f32; 3];
                let mut n = 0f32;
                for y in sy0..sy1 {
                    for x in sx0..sx1 {
                        let off = y * self.bytes_per_row + x * ch * bps;
                        let r = read_sample(&self.data, off, bps) as f32;
                        let g = if ch >= 2 { read_sample(&self.data, off + bps, bps) as f32 } else { r };
                        let b = if ch >= 3 { read_sample(&self.data, off + 2 * bps, bps) as f32 } else { r };
                        acc[0] += r;
                        acc[1] += g;
                        acc[2] += b;
                        n += 1.0;
                    }
                }
                for c in acc {
                    out.push(((c / n / full).sqrt() * 255.0) as u8);
                }
                out.push(255);
            }
        }
        (ow as u32, oh as u32, out)
    }

    /// Convert to 8-bit RGB (3 channels) suitable for `GdkMemoryTexture`.
    ///
    /// Single-channel input is replicated to RGB.
    /// 16-bit input is downscaled to 8-bit with a simple 8-bit gamma approximation
    /// (linear/2^8 shift, no colour management — good enough for preview thumbnails).
    pub fn to_rgb8(&self) -> Vec<u8> {
        // Only complete rows: a scan in progress has fewer than it announced.
        let h = self.effective_height().min(self.rows_available()) as usize;
        let w = self.width as usize;
        let mut out = vec![0u8; w * h * 3];

        let bps = self.depth as usize / 8; // bytes per sample
        let ch = self.channels as usize;
        let src = &self.data[..self.bytes_per_row * h];

        for row in 0..h {
            for col in 0..w {
                let src_off = row * self.bytes_per_row + col * ch * bps;
                let dst_off = (row * w + col) * 3;

                let r = read_sample(src, src_off, bps);
                let g = if ch >= 2 { read_sample(src, src_off + bps, bps) } else { r };
                let b = if ch >= 3 { read_sample(src, src_off + 2 * bps, bps) } else { r };

                // For display: simple gamma≈2.0 via sqrt on normalized [0,1].
                let scale = if bps == 2 { 65535.0f32 } else { 255.0f32 };
                let r8 = ((r as f32 / scale).sqrt() * 255.0) as u8;
                let g8 = ((g as f32 / scale).sqrt() * 255.0) as u8;
                let b8 = ((b as f32 / scale).sqrt() * 255.0) as u8;

                out[dst_off] = r8;
                out[dst_off + 1] = g8;
                out[dst_off + 2] = b8;
            }
        }
        out
    }
}

/// Read one sample out of a raw SANE buffer.
///
/// SANE transmits 16-bit samples in the **native byte order of the machine
/// the backend runs on** -- not big-endian. The standard's wording is "the
/// bytes of each sample value are transmitted in the machine's native byte
/// order", and the `net` backend is what reconciles the two ends when a
/// saned server and its client disagree. Frontends must not byte-swap.
///
/// This used to use `from_be_bytes`, which swapped every sample on x86 and
/// turned each pixel's noisy low byte into its high byte -- the scan came
/// out as noise rather than an image.
pub fn read_sample(data: &[u8], offset: usize, bps: usize) -> u32 {
    if offset + bps > data.len() {
        return 0;
    }
    if bps == 1 {
        data[offset] as u32
    } else {
        u16::from_ne_bytes([data[offset], data[offset + 1]]) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::{read_sample, FrameBuf};

    #[test]
    fn sixteen_bit_samples_are_host_order() {
        // SANE hands over 16-bit samples in the host's byte order. Decoding
        // them as big-endian swaps every sample, which turns each pixel's
        // noisy low byte into its high byte and the scan into noise.
        let v: u16 = 0x1234;
        let buf = v.to_ne_bytes();
        assert_eq!(read_sample(&buf, 0, 2), 0x1234);
        assert_ne!(read_sample(&buf, 0, 2), 0x3412);
    }

    #[test]
    fn eight_bit_samples_pass_through() {
        assert_eq!(read_sample(&[0x7f], 0, 1), 0x7f);
    }

    #[test]
    fn reads_past_the_end_are_zero() {
        assert_eq!(read_sample(&[0x01], 0, 2), 0);
        assert_eq!(read_sample(&[], 0, 1), 0);
    }

    // ── crop ──

    #[test]
    fn a_crop_is_the_block_and_is_clipped_to_the_frame() {
        let mut b = FrameBuf::with_stride(4, Some(3), 8, 2, 10); // padded rows
        for y in 0..3u8 {
            b.append(&[y * 10, y * 10 + 1, y * 10 + 2, y * 10 + 3, y * 10 + 4, y * 10 + 5, y * 10 + 6, y * 10 + 7, 99, 99]);
        }
        let c = b.crop(1, 1, 2, 2);
        assert_eq!((c.width, c.effective_height(), c.channels), (2, 2, 2));
        assert_eq!(c.as_bytes(), &[12, 13, 14, 15, 22, 23, 24, 25]);
        let edge = b.crop(3, 2, 10, 10);
        assert_eq!((edge.width, edge.effective_height()), (1, 1));
        assert_eq!(edge.as_bytes(), &[26, 27]);
        assert_eq!(b.crop(9, 9, 2, 2).width, 0);
    }

    // ── thumbnails ──

    fn gradient(w: u32, h: u32) -> FrameBuf {
        let mut b = FrameBuf::new(w, Some(h), 8, 3);
        for _y in 0..h {
            for x in 0..w {
                b.append(&[(x * 255 / (w - 1)) as u8; 3]);
            }
        }
        b
    }

    #[test]
    fn thumbnail_keeps_aspect_and_caps_the_long_side() {
        let (w, h, px) = gradient(400, 100).thumbnail_rgba(40, None);
        assert_eq!((w, h), (40, 10));
        assert_eq!(px.len(), 40 * 10 * 4);
    }

    #[test]
    fn thumbnail_of_a_region_shows_only_that_region() {
        let b = gradient(200, 100);
        // Right half only: even its darkest pixel is mid-grey or brighter.
        let (_, _, px) = b.thumbnail_rgba(20, Some([0.5, 0.0, 1.0, 1.0]));
        assert!(px.chunks(4).all(|p| p[0] >= 150), "{:?}", &px[..8]);
    }

    #[test]
    fn small_images_are_not_upscaled() {
        assert_eq!(gradient(8, 4).thumbnail_rgba(64, None).0, 8);
    }
}
