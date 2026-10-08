//! Linear DNG export: the scan itself, unprocessed, as a Digital Negative
//! that raw developers (darktable, RawTherapee, Lightroom) open. This is the
//! "flat master": colour management and inversion are left to the developer,
//! and the frame's rotation travels as the Orientation tag, so nothing is
//! resampled.
//!
//! The file is written the way the DNG specification describes a linear
//! raw: IFD0 holds a small RGB preview and the colour tags, and a SubIFD
//! holds the picture as 16-bit `LinearRaw`, uncompressed. The picture is
//! streamed row by row and the directories go at the end, so a large scan
//! needs no second copy in memory. Classic TIFF offsets are 32 bits, so a
//! file over 4 GB is refused.

use std::io::{self, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicBool, Ordering};

use nv_image::frame_buf::read_sample;
use nv_image::FrameBuf;
use nv_process::{InputProfile, Orientation};

use crate::encode::ExportError;

const BYTE: u16 = 1;
const ASCII: u16 = 2;
const SHORT: u16 = 3;
const LONG: u16 = 4;
const RATIONAL: u16 = 5;
const SRATIONAL: u16 = 10;

/// Who made the file, for the Make, Model and Software tags.
#[derive(Debug, Clone)]
pub struct DngInfo {
    pub make: String,
    pub model: String,
    pub software: String,
}

impl Default for DngInfo {
    fn default() -> Self {
        Self { make: "Negative Vibes".into(), model: "Film scan".into(), software: format!("Negative Vibes {}", env!("CARGO_PKG_VERSION")) }
    }
}

/// The EXIF orientation value (1 to 8) that shows the scan as `o` does.
pub fn exif_orientation(o: Orientation) -> u16 {
    // What each EXIF value does to a stored picture, by definition.
    fn shown(value: u16, scan: &[u8], w: usize, h: usize) -> (usize, usize, Vec<u8>) {
        let at = |x: usize, y: usize| scan[y * w + x];
        let (ow, oh) = if value >= 5 { (h, w) } else { (w, h) };
        let mut out = Vec::with_capacity(ow * oh);
        for y in 0..oh {
            for x in 0..ow {
                out.push(match value {
                    1 => at(x, y),
                    2 => at(w - 1 - x, y),
                    3 => at(w - 1 - x, h - 1 - y),
                    4 => at(x, h - 1 - y),
                    5 => at(y, x),
                    6 => at(y, h - 1 - x),
                    7 => at(w - 1 - y, h - 1 - x),
                    _ => at(w - 1 - y, x),
                });
            }
        }
        (ow, oh, out)
    }
    let (w, h) = (3usize, 2usize);
    let scan: Vec<u8> = (0..(w * h) as u8).collect();
    let (ow, oh, made) = o.apply(&scan, w, h, 1);
    (1..=8u16).find(|v| shown(*v, &scan, w, h) == (ow, oh, made.clone())).unwrap_or(1)
}

fn invert3(m: [[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if det.abs() < 1e-9 {
        return None;
    }
    let c = |r: usize, k: usize| {
        let (r1, r2, k1, k2) = ((r + 1) % 3, (r + 2) % 3, (k + 1) % 3, (k + 2) % 3);
        m[r1][k1] * m[r2][k2] - m[r1][k2] * m[r2][k1]
    };
    let mut out = [[0.0; 3]; 3];
    for r in 0..3 {
        for k in 0..3 {
            out[k][r] = c(r, k) / det;
        }
    }
    Some(out)
}

/// XYZ (D65) to linear sRGB.
const XYZ_TO_SRGB: [[f64; 3]; 3] = [[3.2406, -1.5372, -0.4986], [-0.9689, 1.8758, 0.0415], [0.0557, -0.2040, 1.0570]];

/// DNG's ColorMatrix1: XYZ to the scanner's own RGB. A scan with no profile is
/// taken to be linear sRGB; with the scanner's IT8 profile, its matrix is
/// folded in so developers see the calibrated colour.
pub fn color_matrix(input: Option<&InputProfile>) -> [[f64; 3]; 3] {
    let Some(p) = input else { return XYZ_TO_SRGB };
    let m = p.matrix.map(|r| r.map(|v| v as f64)); // device -> linear sRGB
    let Some(back) = invert3(m) else { return XYZ_TO_SRGB };
    let mut out = [[0.0; 3]; 3];
    for r in 0..3 {
        for c in 0..3 {
            out[r][c] = (0..3).map(|k| back[r][k] * XYZ_TO_SRGB[k][c]).sum();
        }
    }
    out
}

struct Entry {
    tag: u16,
    typ: u16,
    count: u32,
    data: Vec<u8>,
}

fn short(tag: u16, v: &[u16]) -> Entry {
    Entry { tag, typ: SHORT, count: v.len() as u32, data: v.iter().flat_map(|x| x.to_le_bytes()).collect() }
}
fn long(tag: u16, v: &[u32]) -> Entry {
    Entry { tag, typ: LONG, count: v.len() as u32, data: v.iter().flat_map(|x| x.to_le_bytes()).collect() }
}
fn ascii(tag: u16, s: &str) -> Entry {
    let mut data = s.as_bytes().to_vec();
    data.push(0);
    Entry { tag, typ: ASCII, count: data.len() as u32, data }
}
fn bytes(tag: u16, v: &[u8]) -> Entry {
    Entry { tag, typ: BYTE, count: v.len() as u32, data: v.to_vec() }
}
fn rationals(tag: u16, v: &[f64]) -> Entry {
    let data = v.iter().flat_map(|x| [((x * 10000.0).round() as i32).to_le_bytes(), 10000i32.to_le_bytes()].concat()).collect();
    Entry { tag, typ: SRATIONAL, count: v.len() as u32, data }
}
fn unsigned_rationals(tag: u16, v: &[u32]) -> Entry {
    let data = v.iter().flat_map(|x| [x.to_le_bytes(), 1u32.to_le_bytes()].concat()).collect();
    Entry { tag, typ: RATIONAL, count: v.len() as u32, data }
}

/// Write the directory (and the values that do not fit in it) at the end of
/// the file so far; returns where it starts.
fn write_ifd<W: Write + Seek>(out: &mut W, mut entries: Vec<Entry>) -> io::Result<u32> {
    entries.sort_by_key(|e| e.tag);
    let mut values: Vec<Option<u32>> = Vec::new();
    for e in &entries {
        if e.data.len() > 4 {
            let at = out.seek(SeekFrom::End(0))?;
            out.write_all(&e.data)?;
            if e.data.len() % 2 == 1 {
                out.write_all(&[0])?;
            }
            values.push(Some(u32::try_from(at).map_err(|_| too_big())?));
        } else {
            values.push(None);
        }
    }
    let start = out.seek(SeekFrom::End(0))?;
    out.write_all(&(entries.len() as u16).to_le_bytes())?;
    for (e, at) in entries.iter().zip(&values) {
        out.write_all(&e.tag.to_le_bytes())?;
        out.write_all(&e.typ.to_le_bytes())?;
        out.write_all(&e.count.to_le_bytes())?;
        match at {
            Some(a) => out.write_all(&a.to_le_bytes())?,
            None => {
                let mut inline = e.data.clone();
                inline.resize(4, 0);
                out.write_all(&inline)?;
            }
        }
    }
    out.write_all(&0u32.to_le_bytes())?;
    u32::try_from(start).map_err(|_| too_big())
}

fn too_big() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "a DNG over 4 GB is not possible; scan at a lower resolution or export a TIFF")
}

/// The scan as a linear DNG. 8-bit scans are widened to 16 bits; a fourth
/// (infrared) sample is left out; a grey scan becomes three equal channels.
pub fn write_dng<W: Write + Seek>(
    mut out: W,
    raw: &FrameBuf,
    orientation: Orientation,
    input: Option<&InputProfile>,
    info: &DngInfo,
    progress: &dyn Fn(f32),
    cancel: &AtomicBool,
) -> Result<(u32, u32), ExportError> {
    let (w, h) = (raw.width as usize, raw.effective_height().min(raw.rows_available()) as usize);
    if w == 0 || h == 0 {
        return Err(ExportError::Empty);
    }
    let image_bytes = w as u64 * h as u64 * 6;
    if image_bytes > u32::MAX as u64 - (1 << 20) {
        return Err(ExportError::Io(too_big()));
    }
    let (ch, bps, bpr) = (raw.channels as usize, raw.depth as usize / 8, raw.bytes_per_row);
    let data = raw.as_bytes();

    out.write_all(b"II*\0\0\0\0\0")?;
    // The picture, in strips of about a megabyte.
    let rows_per_strip = ((1usize << 20) / (w * 6)).clamp(1, h);
    let mut strips: Vec<(u32, u32)> = Vec::new();
    let mut y = 0;
    while y < h {
        if cancel.load(Ordering::Relaxed) {
            return Err(ExportError::Cancelled);
        }
        let rows = rows_per_strip.min(h - y);
        let at = out.stream_position()?;
        let mut buf = Vec::with_capacity(rows * w * 6);
        for r in y..y + rows {
            for x in 0..w {
                let o = r * bpr + x * ch * bps;
                for c in 0..3 {
                    let v = read_sample(data, o + c.min(ch - 1) * bps, bps);
                    let v16 = if bps == 1 { (v * 257) as u16 } else { v as u16 };
                    buf.extend_from_slice(&v16.to_le_bytes());
                }
            }
        }
        out.write_all(&buf)?;
        strips.push((u32::try_from(at).map_err(|_| too_big())?, buf.len() as u32));
        y += rows;
        progress(y as f32 / h as f32 * 0.95);
    }

    // The preview for IFD0: small, 8-bit RGB.
    let (tw, th, rgba) = raw.thumbnail_rgba(256, None);
    let thumb: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    let thumb_at = u32::try_from(out.seek(SeekFrom::End(0))?).map_err(|_| too_big())?;
    out.write_all(&thumb)?;

    let offsets: Vec<u32> = strips.iter().map(|s| s.0).collect();
    let counts: Vec<u32> = strips.iter().map(|s| s.1).collect();
    let sub = write_ifd(
        &mut out,
        vec![
            long(254, &[0]),
            long(256, &[w as u32]),
            long(257, &[h as u32]),
            short(258, &[16, 16, 16]),
            short(259, &[1]),
            short(262, &[34892]), // LinearRaw
            long(273, &offsets),
            short(277, &[3]),
            long(278, &[rows_per_strip as u32]),
            long(279, &counts),
            short(284, &[1]),
            long(50717, &[65535, 65535, 65535]), // WhiteLevel
            long(50719, &[0, 0]),                // DefaultCropOrigin
            long(50720, &[w as u32, h as u32]),  // DefaultCropSize
        ],
    )?;
    let m = color_matrix(input);
    let first = write_ifd(
        &mut out,
        vec![
            long(254, &[1]),
            long(256, &[tw]),
            long(257, &[th]),
            short(258, &[8, 8, 8]),
            short(259, &[1]),
            short(262, &[2]),
            ascii(271, &info.make),
            ascii(272, &info.model),
            long(273, &[thumb_at]),
            short(274, &[exif_orientation(orientation)]),
            short(277, &[3]),
            long(278, &[th]),
            long(279, &[thumb.len() as u32]),
            short(284, &[1]),
            ascii(305, &info.software),
            long(330, &[sub]),
            bytes(50706, &[1, 4, 0, 0]),
            bytes(50707, &[1, 1, 0, 0]),
            ascii(50708, &format!("{} {}", info.make, info.model)),
            rationals(50721, &m.iter().flatten().cloned().collect::<Vec<_>>()),
            unsigned_rationals(50728, &[1, 1, 1]),
            short(50778, &[21]), // CalibrationIlluminant1: D65
        ],
    )?;
    out.seek(SeekFrom::Start(4))?;
    out.write_all(&first.to_le_bytes())?;
    out.flush()?;
    progress(1.0);
    Ok((w as u32, h as u32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn exif_orientation_matches_the_standard_values() {
        let o = |rotation, flip_h, flip_v| exif_orientation(Orientation { rotation, flip_h, flip_v });
        assert_eq!(o(0, false, false), 1);
        assert_eq!(o(0, true, false), 2);
        assert_eq!(o(2, false, false), 3);
        assert_eq!(o(0, false, true), 4);
        assert_eq!(o(1, false, false), 6);
        assert_eq!(o(3, false, false), 8);
        // All eight transforms get distinct values.
        let mut seen: Vec<u16> = Vec::new();
        for rotation in 0..4u8 {
            for flip_h in [false, true] {
                let v = exif_orientation(Orientation { rotation, flip_h, flip_v: false });
                if !seen.contains(&v) {
                    seen.push(v);
                }
            }
        }
        seen.sort();
        assert_eq!(seen, (1..=8).collect::<Vec<u16>>());
    }

    #[test]
    fn the_colour_matrix_is_srgb_without_a_profile_and_follows_a_profile() {
        assert_eq!(color_matrix(None), XYZ_TO_SRGB);
        let ident = InputProfile { name: "i".into(), matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], trc: Default::default() };
        let m = color_matrix(Some(&ident));
        for r in 0..3 {
            for c in 0..3 {
                assert!((m[r][c] - XYZ_TO_SRGB[r][c]).abs() < 1e-9);
            }
        }
        // A profile that halves green leaves the camera green twice as strong per unit of sRGB.
        let half = InputProfile { matrix: [[1.0, 0.0, 0.0], [0.0, 0.5, 0.0], [0.0, 0.0, 1.0]], ..ident };
        let m = color_matrix(Some(&half));
        assert!((m[1][1] - 2.0 * XYZ_TO_SRGB[1][1]).abs() < 1e-9);
    }

    #[test]
    fn matrix_inverse_round_trips() {
        let inv = invert3(XYZ_TO_SRGB).unwrap();
        for r in 0..3 {
            for c in 0..3 {
                let v: f64 = (0..3).map(|k| XYZ_TO_SRGB[r][k] * inv[k][c]).sum();
                assert!((v - if r == c { 1.0 } else { 0.0 }).abs() < 1e-9);
            }
        }
        assert!(invert3([[1.0, 2.0, 3.0], [2.0, 4.0, 6.0], [0.0, 0.0, 1.0]]).is_none());
    }

    #[test]
    fn an_empty_scan_is_refused_and_cancelling_stops() {
        let empty = FrameBuf::new(0, Some(0), 16, 3);
        let r = write_dng(Cursor::new(Vec::new()), &empty, Orientation::IDENTITY, None, &DngInfo::default(), &|_| {}, &AtomicBool::new(false));
        assert!(matches!(r, Err(ExportError::Empty)));
        let mut b = FrameBuf::new(4, Some(4), 8, 3);
        b.append(&[7u8; 48]);
        let r = write_dng(Cursor::new(Vec::new()), &b, Orientation::IDENTITY, None, &DngInfo::default(), &|_| {}, &AtomicBool::new(true));
        assert!(matches!(r, Err(ExportError::Cancelled)));
    }
}
