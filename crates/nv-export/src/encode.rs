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
use nv_process::tiles::{TILE, render_toned_region_straightened};
use nv_process::{Orientation, ProcessParams};

use crate::colour::{ColourSpace, GREY_GAMMA};
use crate::icc;
use crate::resize::Downscaler;
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
        if self.depth == 16 {
            o16.reserve(n * if self.grey { 1 } else { 3 })
        } else {
            o8.reserve(n * if self.grey { 1 } else { 3 })
        }
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

/// Encoded rows waiting to be written in whole bands.
struct Pending {
    samples: Samples,
    rows: usize,
    /// Samples per row, learned from the first rows pushed.
    per_row: usize,
}

impl Pending {
    fn new(depth: u8) -> Self {
        Self { samples: if depth == 16 { Samples::U16(Vec::new()) } else { Samples::U8(Vec::new()) }, rows: 0, per_row: 0 }
    }

    fn push(&mut self, s: Samples, rows: usize) {
        match (&mut self.samples, s) {
            (Samples::U8(a), Samples::U8(b)) => {
                self.per_row = b.len() / rows.max(1);
                a.extend(b);
            }
            (Samples::U16(a), Samples::U16(b)) => {
                self.per_row = b.len() / rows.max(1);
                a.extend(b);
            }
            _ => unreachable!("one bit depth per export"),
        }
        self.rows += rows;
    }

    /// The first `rows` rows.
    fn take(&mut self, rows: usize) -> Samples {
        let n = rows * self.per_row;
        self.rows -= rows;
        match &mut self.samples {
            Samples::U8(v) => Samples::U8(v.drain(..n).collect()),
            Samples::U16(v) => Samples::U16(v.drain(..n).collect()),
        }
    }
}

fn temp_name(dest: &Path) -> PathBuf {
    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    dest.with_file_name(format!(".{name}.part"))
}

/// Render `raw` with `frozen` parameters and write it to `dest`.
///
/// The picture is the whole of `raw`, turned and mirrored as `orientation`
/// says (a scan is already taken inside the frame's rectangle), then made
/// smaller if `settings.size` asks. `progress` is called with the fraction
/// done after each band; `cancel` is checked between bands.
pub fn export_image(
    raw: &FrameBuf,
    frozen: &ProcessParams,
    orientation: Orientation,
    settings: &ExportSettings,
    dest: &Path,
    progress: &dyn Fn(f32),
    cancel: &AtomicBool,
) -> Result<ExportReport, ExportError> {
    export_image_straightened(raw, frozen, orientation, 0.0, settings, dest, progress, cancel)
}

/// [`export_image`] with the picture turned by `angle_deg` (see
/// `nv_process::straighten`) after the orientation.
#[allow(clippy::too_many_arguments)]
pub fn export_image_straightened(
    raw: &FrameBuf,
    frozen: &ProcessParams,
    orientation: Orientation,
    angle_deg: f32,
    settings: &ExportSettings,
    dest: &Path,
    progress: &dyn Fn(f32),
    cancel: &AtomicBool,
) -> Result<ExportReport, ExportError> {
    export_image_with(raw, frozen, orientation, angle_deg, None, settings, dest, progress, cancel)
}

/// [`export_image_straightened`] with an XMP packet (see [`crate::metadata`])
/// embedded in the file, when `xmp` is given and the settings allow it.
#[allow(clippy::too_many_arguments)]
pub fn export_image_with(
    raw: &FrameBuf,
    frozen: &ProcessParams,
    orientation: Orientation,
    angle_deg: f32,
    xmp: Option<&[u8]>,
    settings: &ExportSettings,
    dest: &Path,
    progress: &dyn Fn(f32),
    cancel: &AtomicBool,
) -> Result<ExportReport, ExportError> {
    export_image_full(raw, frozen, orientation, angle_deg, xmp, None, settings, dest, progress, cancel)
}

/// [`export_image_with`] with the details a DNG carries about its picture and scan
/// (see [`crate::dng::DngInfo`]); other formats ignore them.
#[allow(clippy::too_many_arguments)]
pub fn export_image_full(
    raw: &FrameBuf,
    frozen: &ProcessParams,
    orientation: Orientation,
    angle_deg: f32,
    xmp: Option<&[u8]>,
    dng: Option<&crate::dng::DngInfo>,
    settings: &ExportSettings,
    dest: &Path,
    progress: &dyn Fn(f32),
    cancel: &AtomicBool,
) -> Result<ExportReport, ExportError> {
    let xmp = xmp.filter(|_| settings.embed_metadata);
    let (sw, sh) = (raw.width as usize, raw.effective_height() as usize);
    if sw == 0 || sh == 0 {
        return Err(ExportError::Empty);
    }
    let s = settings.normalised();
    if s.format == ExportFormat::Dng {
        return export_dng(raw, frozen, orientation, xmp, dng, dest, progress, cancel);
    }
    // The picture the orientation makes, and the size it is exported at.
    let (pw, ph) = orientation.output_size(sw, sh);
    let (w, h) = s.size.dims(pw, ph);
    let part = temp_name(dest);

    let result = write_file(raw, frozen, orientation, angle_deg, xmp, &s, &part, (pw, ph), (w, h), progress, cancel);
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

/// The raw scan as a linear DNG: the frame's orientation rides along as a
/// tag, and the scanner's profile, if the frame uses one, in the colour matrix.
fn export_dng(
    raw: &FrameBuf,
    frozen: &ProcessParams,
    orientation: Orientation,
    xmp: Option<&[u8]>,
    dng: Option<&crate::dng::DngInfo>,
    dest: &Path,
    progress: &dyn Fn(f32),
    cancel: &AtomicBool,
) -> Result<ExportReport, ExportError> {
    let part = temp_name(dest);
    let result = (|| {
        let file = BufWriter::new(File::create(&part)?);
        crate::dng::write_dng(file, raw, orientation, frozen.input.as_ref(), &dng.cloned().unwrap_or_else(crate::dng::DngInfo::plain), xmp, progress, cancel)
    })();
    match result {
        Ok((width, height)) => {
            std::fs::rename(&part, dest)?;
            Ok(ExportReport { width, height, path: dest.to_owned(), bytes: std::fs::metadata(dest)?.len() })
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
    orientation: Orientation,
    angle_deg: f32,
    xmp: Option<&[u8]>,
    s: &ExportSettings,
    part: &Path,
    (pw, ph): (usize, usize),
    (w, h): (usize, usize),
    progress: &dyn Fn(f32),
    cancel: &AtomicBool,
) -> Result<(), ExportError> {
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get());
    let enc = Encoder::new(s.colour_space, s.grey, s.bit_depth);
    let profile = s.embed_profile.then(|| if s.grey { icc::grey_profile() } else { icc::rgb_profile(s.colour_space) });
    let file = BufWriter::new(File::create(part)?);

    // Bands of `TILE` rows of the picture: the unit the renderer cuts anyway.
    // When the picture is made smaller, the rows that come out of the
    // downscaler are collected into bands of `TILE` rows again, so the writers
    // see the same thing either way.
    let mut bands = |sink: &mut dyn FnMut(Samples, usize) -> Result<(), ExportError>| -> Result<(), ExportError> {
        let mut scaler = ((pw, ph) != (w, h)).then(|| Downscaler::new(pw, ph, w, h, 3));
        let mut pending = Pending::new(s.bit_depth);
        let flush = |pending: &mut Pending, all: bool, sink: &mut dyn FnMut(Samples, usize) -> Result<(), ExportError>| -> Result<(), ExportError> {
            while pending.rows >= TILE || (all && pending.rows > 0) {
                let n = pending.rows.min(TILE);
                sink(pending.take(n), n)?;
            }
            Ok(())
        };
        let mut y = 0;
        while y < ph {
            if cancel.load(Ordering::Relaxed) {
                return Err(ExportError::Cancelled);
            }
            let rows = TILE.min(ph - y);
            let toned = render_toned_region_straightened(raw, frozen, orientation, angle_deg, 0, y, pw, rows, threads);
            let out = match scaler.as_mut() {
                Some(d) => d.push_rows(&toned, pw),
                None => toned,
            };
            if !out.is_empty() {
                pending.push(enc.band(&out), out.len() / (w * 3));
                flush(&mut pending, false, sink)?;
            }
            y += rows;
            progress(y as f32 / ph as f32);
        }
        flush(&mut pending, true, sink)
    };

    match s.format {
        ExportFormat::Tiff => write_tiff(file, s, w, h, profile.as_deref(), xmp, &mut bands),
        ExportFormat::Png => write_png(file, s, w, h, profile.as_deref(), xmp, &mut bands),
        ExportFormat::Jpeg => write_jpeg(file, s, w, h, profile.as_deref(), xmp, &mut bands),
        ExportFormat::Dng => unreachable!("a DNG is written from the raw scan, not from bands"),
    }
}

type Bands<'a> = &'a mut dyn FnMut(&mut dyn FnMut(Samples, usize) -> Result<(), ExportError>) -> Result<(), ExportError>;

// ── TIFF ──────────────────────────────────────────────────────────────────────

fn write_tiff<W: Write + Seek>(out: W, s: &ExportSettings, w: usize, h: usize, profile: Option<&[u8]>, xmp: Option<&[u8]>, bands: Bands) -> Result<(), ExportError> {
    let channels = if s.grey { 1 } else { 3 };
    let mut tiff =
        TiffWriter::new(out, w as u32, h as u32, channels, s.bit_depth as u16, s.compression, TIFF_STRIP_ROWS, profile.map(<[u8]>::to_vec))?.with_xmp(xmp.map(<[u8]>::to_vec));
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

fn write_png<W: Write>(out: W, s: &ExportSettings, w: usize, h: usize, profile: Option<&[u8]>, xmp: Option<&[u8]>, bands: Bands) -> Result<(), ExportError> {
    let png_err = |e: png::EncodingError| ExportError::Png(e.to_string());
    let mut info = png::Info::with_size(w as u32, h as u32);
    info.color_type = if s.grey { png::ColorType::Grayscale } else { png::ColorType::Rgb };
    info.bit_depth = if s.bit_depth == 16 { png::BitDepth::Sixteen } else { png::BitDepth::Eight };
    if let Some(p) = profile {
        info.icc_profile = Some(std::borrow::Cow::Owned(p.to_vec()));
    }
    if let Some(x) = xmp {
        // XMP lives in an uncompressed iTXt chunk with this keyword (XMP specification, part 3).
        info.utf8_text.push(png::text_metadata::ITXtChunk::new("XML:com.adobe.xmp", String::from_utf8_lossy(x).into_owned()));
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

fn write_jpeg<W: Write>(out: W, s: &ExportSettings, w: usize, h: usize, profile: Option<&[u8]>, xmp: Option<&[u8]>, bands: Bands) -> Result<(), ExportError> {
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
    if let Some(x) = xmp {
        // One APP1 segment holds up to 65502 bytes after its namespace.
        const NS: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
        if NS.len() + x.len() > 65533 {
            return Err(ExportError::Jpeg("the metadata is too long for a JPEG; shorten the caption or keywords".into()));
        }
        enc.add_app_segment(1, &[NS, x].concat()).map_err(|e| ExportError::Jpeg(e.to_string()))?;
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

// ── Raw scans ─────────────────────────────────────────────────────────────────

/// Write a scan as it came from the scanner (1, 3 or 4 channels, 8 or 16 bit,
/// samples in the host's byte order, rows possibly padded) as a TIFF for the
/// roll's raw folder. With `ir`, a one-channel plane of the same size and
/// depth is stored as a fourth sample after the colour (the HDRi RAW layout,
/// DESIGN.md §6.3). Rows are gathered a band at a time, so saving a 7200 dpi
/// frame needs a band of memory beyond the frame itself, not copies of it.
pub fn write_scan_tiff(buf: &FrameBuf, ir: Option<&FrameBuf>, dest: &Path, compression: crate::settings::Compression) -> Result<(), ExportError> {
    let (w, h) = (buf.width as usize, buf.effective_height() as usize);
    if w == 0 || h == 0 {
        return Err(ExportError::Empty);
    }
    let bad = |m: String| ExportError::Io(io::Error::other(m));
    let (channels, depth) = (buf.channels as usize, buf.depth as usize);
    if !matches!(channels, 1 | 3 | 4) || !matches!(depth, 8 | 16) {
        return Err(bad(format!("cannot write a {channels}-channel {depth}-bit scan")));
    }
    if let Some(ir) = ir {
        if channels != 3 || ir.channels != 1 || ir.depth != buf.depth || ir.width != buf.width || ir.effective_height() as usize != h {
            return Err(bad("the infrared plane does not match the colour scan".into()));
        }
    }
    let out_channels = channels + usize::from(ir.is_some());
    let row_bytes = w * channels * depth / 8;
    let bytes = buf.as_bytes();
    if bytes.len() < buf.bytes_per_row * (h - 1) + row_bytes {
        return Err(bad("incomplete scan data".into()));
    }
    let ir_row_bytes = w * depth / 8;
    if let Some(ir) = ir {
        if ir.as_bytes().len() < ir.bytes_per_row * (h - 1) + ir_row_bytes {
            return Err(bad("incomplete infrared data".into()));
        }
    }

    let part = temp_name(dest);
    let write = || -> Result<(), ExportError> {
        let file = BufWriter::new(File::create(&part)?);
        let mut tiff = TiffWriter::new(file, w as u32, h as u32, out_channels as u16, depth as u16, compression, TIFF_STRIP_ROWS, None)?;
        let sample = |b: &[u8], i: usize| -> u16 { if depth == 16 { u16::from_ne_bytes([b[i * 2], b[i * 2 + 1]]) } else { b[i] as u16 } };
        let mut y = 0;
        while y < h {
            let rows = TILE.min(h - y);
            let mut band: Vec<u16> = Vec::with_capacity(rows * w * out_channels);
            for r in 0..rows {
                let row = &bytes[(y + r) * buf.bytes_per_row..][..row_bytes];
                match ir {
                    None => band.extend((0..w * channels).map(|i| sample(row, i))),
                    Some(ir) => {
                        let irow = &ir.as_bytes()[(y + r) * ir.bytes_per_row..][..ir_row_bytes];
                        for x in 0..w {
                            band.extend((0..3).map(|c| sample(row, x * 3 + c)));
                            band.push(sample(irow, x));
                        }
                    }
                }
            }
            if depth == 16 {
                tiff.write_rows_u16(rows as u32, &band)?;
            } else {
                tiff.write_rows_u8(rows as u32, &band.iter().map(|v| *v as u8).collect::<Vec<u8>>())?;
            }
            y += rows;
        }
        tiff.finish()?;
        Ok(())
    };
    match write() {
        Ok(()) => Ok(std::fs::rename(&part, dest)?),
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}
