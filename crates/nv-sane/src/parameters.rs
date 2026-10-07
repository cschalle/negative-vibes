use nv_sane_sys as sys;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameFormat {
    Gray,
    Rgb,
    Red,
    Green,
    Blue,
    /// Four-channel RGBI — an unofficial SANE extension (0x10).
    Rgbi,
}

impl FrameFormat {
    pub(crate) fn from_raw(raw: sys::SANE_Frame) -> Option<Self> {
        Some(match raw {
            sys::SANE_FRAME_GRAY => Self::Gray,
            sys::SANE_FRAME_RGB => Self::Rgb,
            sys::SANE_FRAME_RED => Self::Red,
            sys::SANE_FRAME_GREEN => Self::Green,
            sys::SANE_FRAME_BLUE => Self::Blue,
            sys::SANE_FRAME_RGBI => Self::Rgbi,
            _ => return None,
        })
    }

    pub fn channels(&self) -> u8 {
        match self {
            Self::Rgb => 3,
            Self::Rgbi => 4,
            _ => 1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Parameters {
    pub format: FrameFormat,
    /// `false` means more frames follow (three-pass or multi-frame).
    pub last_frame: bool,
    pub bytes_per_line: usize,
    pub pixels_per_line: usize,
    /// `None` when the scanner cannot predict the height (hand-scanner / sheet-fed).
    pub lines: Option<usize>,
    pub depth: u8,
}

impl Parameters {
    pub(crate) fn from_raw(raw: sys::SANE_Parameters) -> Option<Self> {
        Some(Self {
            format: FrameFormat::from_raw(raw.format)?,
            last_frame: raw.last_frame != sys::SANE_FALSE,
            bytes_per_line: raw.bytes_per_line as usize,
            pixels_per_line: raw.pixels_per_line as usize,
            lines: if raw.lines < 0 { None } else { Some(raw.lines as usize) },
            depth: raw.depth as u8,
        })
    }

    pub fn bytes_total(&self) -> Option<usize> {
        self.lines.map(|l| self.bytes_per_line * l)
    }
}
