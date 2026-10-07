use nv_sane::parameters::FrameFormat;

/// Describes how to interpret the bytes in a single SANE frame.
#[derive(Debug, Clone)]
pub struct FrameMeta {
    pub format: FrameFormat,
    pub width_px: u32,
    pub height_px: Option<u32>,
    pub bytes_per_line: usize,
    pub depth: u8,
    pub is_last: bool,
}

impl FrameMeta {
    pub fn channels(&self) -> u8 {
        self.format.channels()
    }

    pub fn bytes_total(&self) -> Option<usize> {
        self.height_px.map(|h| self.bytes_per_line * h as usize)
    }
}
