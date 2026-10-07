/// A packed pixel buffer built incrementally from SANE `DataChunk` events.
///
/// Raw data is stored exactly as received (8 or 16 bit; see `read_sample`
/// for the byte order of 16-bit samples).
/// Call `to_rgb8()` to get a display-ready sRGB u8 copy.
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

    /// Convert to 8-bit RGB (3 channels) suitable for `GdkMemoryTexture`.
    ///
    /// Single-channel input is replicated to RGB.
    /// 16-bit input is downscaled to 8-bit with a simple 8-bit gamma approximation
    /// (linear/2^8 shift, no colour management — good enough for preview thumbnails).
    pub fn to_rgb8(&self) -> Vec<u8> {
        let h = self.effective_height() as usize;
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
    use super::read_sample;

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
}
