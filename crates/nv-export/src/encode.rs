//! Rendering a frame at full resolution and writing it as TIFF, JPEG or PNG.
//!
//! The frame is rendered in bands one tile high ([`nv_process::tiles::TILE`]),
//! so memory stays at one band however large the frame: that is what keeps a
//! 7200 dpi export inside the memory limit (DESIGN.md check H8). TIFF and PNG
//! are written as the bands arrive; JPEG, whose encoder wants the whole
//! picture, holds the 8-bit result.
//!
//! Files are written under a temporary name in the destination folder and
//! renamed when complete, so a cancelled or failed export leaves no half file.

use std::fs::File;
use std::io::{self, BufWriter, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use nv_image::FrameBuf;
use nv_process::tiles::{render_toned_region, TILE};
use nv_process::ProcessParams;

use crate::colour::{ColourSpace, GREY_GAMMA};
use crate::icc;
use crate::settings::{ExportFormat, ExportSettings};
use crate::tiff::TiffWriter;

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("JPEG: {0}")]
    Jpeg(String),
    #[error("PNG: {0}")]
    Png(String),
    #[error("export cancelled")]
    Cancelled,
    #[error("nothing to export")]
    Empty,
}

#[derive(Debug, Clone)]
pub struct ExportReport {
    pub width: u32,
    pub height: u32,
    pub path: PathBuf,
    pub bytes: u64,
}

/// Rows per TIFF strip. Small enough that a band (one tile high) splits into
/// strips that compress in parallel; a divisor of the tile height.
const TIFF_STRIP_ROWS: u32 = 32;

/// Linear light to an encoded value through a table: `powf` per sample would
/// make a large export wait on the maths.
struct Lut {
    t: Vec<f32>,
}

const LUT_SIZE: usize = 1 << 14;

impl Lut {
    fn new(f: impl Fn(f32) -> f32) -> Self {
        Self { t: (0..=LUT_SIZE).map(|i| f(i as f32 / LUT_SIZE as f32)).collect() }
    }

    #[inline]
    fn get(&self, v: f32) -> f32 {
        let x = v.clamp(0.0, 1.0) * LUT_SIZE as f32;
        let i = (x as usize).min(LUT_SIZE - 1);
        self.t[i] + (self.t[i + 1] - self.t[i]) * (x - i as f32)
    }
}

enum Samples {
    U8(Vec<u8>),
    U16(Vec<u16>),
}

struct Encoder {
    matrix: [[f32; 3]; 3],
    lut: Lut,
    grey: bool,
    depth: u8,
}

impl Encoder {
    fn new(space: ColourSpace, grey: bool, depth: u8) -> Self {
        let lut = if grey { Lut::new(|v| v.powf(1.0 / GREY_GAMMA)) } else { Lut::new(move |v| space.encode(v)) };
        Self { matrix: space.matrix(), lut, grey, depth }
    }

    fn quantise(&self, e: f32, out8: &mut Vec<u8>, out16: &mut Vec<u16>) {
        if self.depth == 16 {
            out16.push((e.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16);
        } else {
            out8.push((e.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
        }
    }

    /// Linear light in sRGB primaries to samples in the output space.
    fn band(&self, toned: &[f32]) -> Samples {
        let n = toned.len() / 3;
        let (mut o8, mut o16) = (Vec::new(), Vec::new());
        if self.depth == 16 { o16.reserve(n * if self.grey { 1 } else { 3 }) } else { o8.reserve(n * if self.grey { 1 } else { 3 }) }
        for px in toned.chunks_exact(3) {
            if self.grey {
                let y = 0.2126 * px[0] + 0.7152 * px[1] + 0.0722 * px[2];
                self.quantise(self.lut.get(y), &mut o8, &mut o16);
            } else {
                for row in &self.matrix {
                    let v = row[0] * px[0] + row[1] * px[1] + row[2] * px[2];
                    self.quantise(self.lut.get(v), &mut o8, &mut o16);
                }
            }
        }
        if self.depth == 16 { Samples::U16(o16) } else { Samples::U8(o8) }
    }
}

fn temp_name(dest: &Path) -> PathBuf {
    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    dest.with_file_name(format!(".{name}.part"))
}

/// Render `raw` with `frozen` parameters and write it to `dest`.
///
/// `progress` is called with the fraction done after each band; `cancel`
/// is checked between bands. The picture is the whole of `raw`: a scan is
/// already taken inside the frame's rectangle.
pub fn export_image(
    raw: &FrameBuf,
    frozen: &ProcessParams,
    settings: &ExportSettings,
    dest: &Path,
    progress: &dyn Fn(f32),
    cancel: &AtomicBool,
) -> Result<ExportReport, ExportError> {
    let (w, h) = (raw.width as usize, raw.effective_height() as usize);
    if w == 0 || h == 0 {
        return Err(ExportError::Empty);
    }
    let s = settings.normalised();
    let part = temp_name(dest);

    let result = write_file(raw, frozen, &s, &part, w, h, progress, cancel);
    match result {
        Ok(()) => {
            std::fs::rename(&part, dest)?;
            let bytes = std::fs::metadata(dest)?.len();
            Ok(ExportReport { width: w as u32, height: h as u32, path: dest.to_owned(), bytes })
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn write_file(
    raw: &FrameBuf,
    frozen: &ProcessParams,
    s: &ExportSettings,
    part: &Path,
    w: usize,
    h: usize,
    progress: &dyn Fn(f32),
    cancel: &AtomicBool,
) -> Result<(), ExportError> {
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get());
    let enc = Encoder::new(s.colour_space, s.grey, s.bit_depth);
    let profile = s.embed_profile.then(|| if s.grey { icc::grey_profile() } else { icc::rgb_profile(s.colour_space) });
    let file = BufWriter::new(File::create(part)?);

    // Each band is `TILE` rows: the unit the renderer cuts anyway.
    let mut bands = |sink: &mut dyn FnMut(Samples, usize) -> Result<(), ExportError>| -> Result<(), ExportError> {
        let mut y = 0;
        while y < h {
            if cancel.load(Ordering::Relaxed) {
                return Err(ExportError::Cancelled);
            }
            let rows = TILE.min(h - y);
            let toned = render_toned_region(raw, frozen, 0, y, w, rows, threads);
            sink(enc.band(&toned), rows)?;
            y += rows;
            progress(y as f32 / h as f32);
        }
        Ok(())
    };

    match s.format {
        ExportFormat::Tiff => write_tiff(file, s, w, h, profile.as_deref(), &mut bands),
        ExportFormat::Png => write_png(file, s, w, h, profile.as_deref(), &mut bands),
        ExportFormat::Jpeg => write_jpeg(file, s, w, h, profile.as_deref(), &mut bands),
    }
}

type Bands<'a> = &'a mut dyn FnMut(&mut dyn FnMut(Samples, usize) -> Result<(), ExportError>) -> Result<(), ExportError>;

// ── TIFF ──────────────────────────────────────────────────────────────────────

fn write_tiff<W: Write + Seek>(out: W, s: &ExportSettings, w: usize, h: usize, profile: Option<&[u8]>, bands: Bands) -> Result<(), ExportError> {
    let channels = if s.grey { 1 } else { 3 };
    let mut tiff = TiffWriter::new(out, w as u32, h as u32, channels, s.bit_depth as u16, s.compression, TIFF_STRIP_ROWS, profile.map(<[u8]>::to_vec))?;
    bands(&mut |samples, rows| {
        match samples {
            Samples::U8(v) => tiff.write_rows_u8(rows as u32, &v)?,
            Samples::U16(v) => tiff.write_rows_u16(rows as u32, &v)?,
        }
        Ok(())
    })?;
    tiff.finish()?;
    Ok(())
}

// ── PNG ───────────────────────────────────────────────────────────────────────

fn write_png<W: Write>(out: W, s: &ExportSettings, w: usize, h: usize, profile: Option<&[u8]>, bands: Bands) -> Result<(), ExportError> {
    let png_err = |e: png::EncodingError| ExportError::Png(e.to_string());
    let mut info = png::Info::with_size(w as u32, h as u32);
    info.color_type = if s.grey { png::ColorType::Grayscale } else { png::ColorType::Rgb };
    info.bit_depth = if s.bit_depth == 16 { png::BitDepth::Sixteen } else { png::BitDepth::Eight };
    if let Some(p) = profile {
        info.icc_profile = Some(std::borrow::Cow::Owned(p.to_vec()));
    }
    let encoder = png::Encoder::with_info(out, info).map_err(png_err)?;
    let mut writer = encoder.write_header().map_err(png_err)?;
    let mut stream = writer.stream_writer().map_err(png_err)?;
    bands(&mut |samples, _| {
        match samples {
            Samples::U8(v) => stream.write_all(&v)?,
            // PNG stores 16-bit samples most significant byte first.
            Samples::U16(v) => stream.write_all(&v.iter().flat_map(|x| x.to_be_bytes()).collect::<Vec<u8>>())?,
        }
        Ok(())
    })?;
    stream.finish().map_err(png_err)?;
    Ok(())
}

// ── JPEG ──────────────────────────────────────────────────────────────────────

fn write_jpeg<W: Write>(out: W, s: &ExportSettings, w: usize, h: usize, profile: Option<&[u8]>, bands: Bands) -> Result<(), ExportError> {
    if w > u16::MAX as usize || h > u16::MAX as usize {
        return Err(ExportError::Jpeg("a JPEG cannot be wider or taller than 65535 pixels".into()));
    }
    let mut pixels: Vec<u8> = Vec::with_capacity(w * h * if s.grey { 1 } else { 3 });
    bands(&mut |samples, _| {
        match samples {
            Samples::U8(v) => pixels.extend_from_slice(&v),
            Samples::U16(_) => unreachable!("JPEG is 8 bit"),
        }
        Ok(())
    })?;
    let mut enc = jpeg_encoder::Encoder::new(out, s.jpeg_quality);
    if s.jpeg_quality >= 90 {
        enc.set_sampling_factor(jpeg_encoder::SamplingFactor::F_1_1);
    }
    if let Some(p) = profile {
        enc.add_icc_profile(p).map_err(|e| ExportError::Jpeg(e.to_string()))?;
    }
    let color = if s.grey { jpeg_encoder::ColorType::Luma } else { jpeg_encoder::ColorType::Rgb };
    enc.encode(&pixels, w as u16, h as u16, color).map_err(|e| ExportError::Jpeg(e.to_string()))
}

// ── Raw ───────────────────────────────────────────────────────────────────────

/// HDR RAW export: the roll's raw scan, byte for byte (DESIGN.md §6.3).
pub fn copy_raw(src: &Path, dest: &Path) -> Result<u64, ExportError> {
    let part = temp_name(dest);
    let result = std::fs::copy(src, &part).and_then(|n| std::fs::rename(&part, dest).map(|_| n));
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    Ok(result?)
}
