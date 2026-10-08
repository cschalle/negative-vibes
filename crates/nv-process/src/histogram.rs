//! Histograms of rendered output, for the inspector and for auto levels.

/// 256-bin histograms of an 8-bit RGB image.
#[derive(Debug, Clone)]
pub struct Histogram {
    pub r: [u32; 256],
    pub g: [u32; 256],
    pub b: [u32; 256],
    /// Rec. 709 luma of the 8-bit values.
    pub luma: [u32; 256],
}

impl Histogram {
    pub fn from_rgb8(rgb: &[u8]) -> Self {
        let mut h = Self { r: [0; 256], g: [0; 256], b: [0; 256], luma: [0; 256] };
        for px in rgb.chunks_exact(3) {
            h.r[px[0] as usize] += 1;
            h.g[px[1] as usize] += 1;
            h.b[px[2] as usize] += 1;
            let y = (2126 * px[0] as u32 + 7152 * px[1] as u32 + 722 * px[2] as u32 + 5000) / 10000;
            h.luma[y.min(255) as usize] += 1;
        }
        h
    }

    /// Largest bin count over the colour channels, ignoring the two end bins
    /// (clipping piles up there and would flatten everything else).
    pub fn peak(&self) -> u32 {
        [&self.r, &self.g, &self.b]
            .iter()
            .flat_map(|c| c[1..255].iter().copied())
            .max()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_each_channel_separately() {
        let h = Histogram::from_rgb8(&[10, 20, 30, 10, 200, 30]);
        assert_eq!((h.r[10], h.g[20], h.g[200], h.b[30]), (2, 1, 1, 2));
        assert_eq!(h.luma.iter().sum::<u32>(), 2);
    }

    #[test]
    fn peak_ignores_clipped_ends() {
        let h = Histogram::from_rgb8(&[0, 0, 0, 0, 0, 0, 255, 255, 255, 100, 100, 100]);
        assert_eq!(h.peak(), 1);
    }
}
