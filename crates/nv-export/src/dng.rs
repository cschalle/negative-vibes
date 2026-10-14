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

use nv_image::FrameBuf;
use nv_image::frame_buf::read_sample;
use nv_process::{InputProfile, Orientation};

use crate::encode::ExportError;

const BYTE: u16 = 1;
const ASCII: u16 = 2;
const SHORT: u16 = 3;
const LONG: u16 = 4;
const RATIONAL: u16 = 5;
const UNDEFINED: u16 = 7;
const SRATIONAL: u16 = 10;

/// What the file says about itself: who made it, what it is of, and when.
/// A scan has no exposure time or aperture of its own, so those are left out;
/// the film's speed is the one exposure-like number it does have.
#[derive(Debug, Clone, Default)]
pub struct DngInfo {
    pub make: String,
    pub model: String,
    pub software: String,
    /// The photographer, copyright and a line about the picture (IFD0).
    pub artist: String,
    pub copyright: String,
    pub description: String,
    /// The lens the picture was taken with.
    pub lens: String,
    /// The film's speed, from its profile.
    pub film_iso: Option<u32>,
    /// When the photograph was taken and when it was scanned, `YYYY:MM:DD HH:MM:SS`.
    pub date_original: Option<String>,
    pub date_scanned: Option<String>,
}

impl DngInfo {
    /// Only who made the file.
    pub fn plain() -> Self {
        Self { make: "Negative Vibes".into(), model: "Film scan".into(), software: format!("Negative Vibes {}", env!("CARGO_PKG_VERSION")), ..Default::default() }
    }

    /// The tags a frame's metadata and scan details give. Empty fields stay out of the file.
    pub fn from_metadata(meta: &crate::metadata::Metadata, frame: &crate::metadata::FrameInfo, film_iso: Option<u32>) -> Self {
        let m = meta.cleaned();
        // The scanner's name without the backend's device path.
        let scanner: String = frame.scanner.split_whitespace().take_while(|w| !w.contains(':')).collect::<Vec<_>>().join(" ");
        let title = if m.title.is_empty() { frame.label.trim().to_owned() } else { m.title.clone() };
        let film = if m.film_stock.is_empty() { frame.film_profile.trim().to_owned() } else { m.film_stock.clone() };
        // One line about the picture: what it is, then what it was shot on.
        let parts: Vec<&str> = [title.as_str(), m.caption.as_str()].into_iter().filter(|s| !s.is_empty()).collect();
        let gear: Vec<&str> = [m.camera.as_str(), film.as_str()].into_iter().filter(|s| !s.is_empty()).collect();
        let description = [parts.join(". "), gear.join(", ")].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" — ");
        Self {
            make: "Negative Vibes".into(),
            model: if scanner.is_empty() { "Film scan".into() } else { scanner },
            software: format!("Negative Vibes {}", env!("CARGO_PKG_VERSION")),
            artist: m.creator.clone(),
            copyright: m.copyright.clone(),
            description,
            lens: m.lens.clone(),
            film_iso: film_iso.filter(|i| *i > 0),
            date_original: exif_date(&m.date_taken),
            date_scanned: exif_date(&frame.scan_date),
        }
    }
}

/// `YYYY:MM:DD HH:MM:SS` from an ISO date: a whole date, with a time or not.
/// A year or a month alone says too little to be a date and gives `None`.
pub fn exif_date(iso: &str) -> Option<String> {
    let s = iso.trim();
    let (date, time) = match s.split_once(['T', ' ']) {
        Some((d, t)) => (d, t.trim_end_matches('Z')),
        None => (s, ""),
    };
    let d: Vec<&str> = date.split('-').collect();
    if d.len() != 3 || d[0].len() != 4 || d[1].len() != 2 || d[2].len() != 2 || !d.iter().all(|p| p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let t: Vec<&str> = time.split(':').collect();
    let hms = match t.as_slice() {
        [h, m, s] if [h, m, s].iter().all(|p| p.len() >= 2 && p.bytes().take(2).all(|b| b.is_ascii_digit())) => format!("{}:{}:{}", &h[..2], &m[..2], &s[..2]),
        [h, m] if [h, m].iter().all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_digit())) => format!("{h}:{m}:00"),
        _ => "00:00:00".to_owned(),
    };
    Some(format!("{}:{}:{} {hms}", d[0], d[1], d[2]))
}

/// The picture inside the black the scanner's window shows around the film
/// holder: `[x, y, width, height]` in scan pixels, trimmed a little further
/// to leave out the film's own edge. The whole scan when there is no such
/// surround, or nothing can be told. Developers open the file cropped to this
/// and can still widen it, since the pixels are all there.
pub fn content_bounds(raw: &FrameBuf) -> [u32; 4] {
    let whole = [0, 0, raw.width, raw.effective_height().min(raw.rows_available())];
    let (w, h) = (raw.width as usize, whole[3] as usize);
    if w < 64 || h < 64 || raw.bytes_per_row == 0 {
        return whole;
    }
    let (ch, bps, bpr) = (raw.channels as usize, raw.depth as usize / 8, raw.bytes_per_row);
    let data = raw.as_bytes();
    let step = (w.max(h) / 400).max(1);
    let (cols, rows) = (w / step, h / step);
    // The green channel, or the only one.
    let g = usize::from(ch >= 2);
    let sample = |x: usize, y: usize| read_sample(data, y * step * bpr + (x * step * ch + g) * bps, bps) as f64;
    let mut col = vec![0f64; cols];
    let mut row = vec![0f64; rows];
    for y in 0..rows {
        for x in 0..cols {
            let v = sample(x, y);
            col[x] += v / rows as f64;
            row[y] += v / cols as f64;
        }
    }
    // What the picture area reads like: the middle of the middle.
    let median = |v: &[f64]| {
        let mut s = v.to_vec();
        s.sort_by(|a, b| a.total_cmp(b));
        s[s.len() / 2]
    };
    let centre = median(&col[cols / 4..cols - cols / 4]).max(median(&row[rows / 4..rows - rows / 4]));
    if centre <= 0.0 {
        return whole;
    }
    // The run around the middle that is lit, not black.
    let span = |v: &[f64]| -> (usize, usize) {
        let lit = |x: f64| x > 0.25 * centre;
        let mid = v.len() / 2;
        let (mut a, mut b) = (mid, mid);
        while a > 0 && lit(v[a - 1]) {
            a -= 1;
        }
        while b + 1 < v.len() && lit(v[b + 1]) {
            b += 1;
        }
        (a, b + 1)
    };
    let (x0, x1) = span(&col);
    let (y0, y1) = span(&row);
    let (px0, px1, py0, py1) = (x0 * step, (x1 * step).min(w), y0 * step, (y1 * step).min(h));
    let (bw, bh) = (px1.saturating_sub(px0), py1.saturating_sub(py0));
    // Hardly any surround: nothing to crop.
    if bw as f64 > 0.97 * w as f64 && bh as f64 > 0.97 * h as f64 {
        return whole;
    }
    // A sliver is not a picture: something went wrong, keep everything.
    if (bw as f64) < 0.3 * w as f64 || (bh as f64) < 0.3 * h as f64 {
        return whole;
    }
    // The film's own edge and holder shadow sit just inside the black.
    let (ix, iy) = ((bw as f64 * 0.015) as usize + step, (bh as f64 * 0.015) as usize + step);
    [(px0 + ix) as u32, (py0 + iy) as u32, bw.saturating_sub(2 * ix) as u32, bh.saturating_sub(2 * iy) as u32]
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
    xmp: Option<&[u8]>,
    progress: &dyn Fn(f32),
    cancel: &AtomicBool,
) -> Result<(u32, u32), ExportError> {
    if raw.width == 0 || raw.bytes_per_row == 0 {
        return Err(ExportError::Empty);
    }
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
    let thumb: Vec<u8> = rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
    let thumb_at = u32::try_from(out.seek(SeekFrom::End(0))?).map_err(|_| too_big())?;
    out.write_all(&thumb)?;

    let crop = content_bounds(raw);
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
            long(50719, &[crop[0], crop[1]]),    // DefaultCropOrigin
            long(50720, &[crop[2], crop[3]]),    // DefaultCropSize
        ],
    )?;
    let m = color_matrix(input);
    // The EXIF block: what is known about the photograph and its scan.
    let mut exif = vec![Entry { tag: 0x9000, typ: UNDEFINED, count: 4, data: b"0231".to_vec() }];
    if let Some(iso) = info.film_iso {
        exif.push(short(0x8827, &[iso.min(65535) as u16]));
    }
    if let Some(d) = &info.date_original {
        exif.push(ascii(0x9003, d));
    }
    if let Some(d) = &info.date_scanned {
        exif.push(ascii(0x9004, d)); // DateTimeDigitized: the scan
    }
    if !info.lens.is_empty() {
        exif.push(ascii(0xA434, &info.lens));
    }
    let exif_at = write_ifd(&mut out, exif)?;
    let mut tags = vec![
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
    ];
    tags.push(long(34665, &[exif_at]));
    for (tag, text) in [(270u16, &info.description), (315, &info.artist), (33432, &info.copyright)] {
        if !text.is_empty() {
            tags.push(ascii(tag, text));
        }
    }
    if let Some(d) = &info.date_scanned {
        tags.push(ascii(306, d));
    }
    if let Some(x) = xmp {
        tags.push(Entry { tag: 700, typ: BYTE, count: x.len() as u32, data: x.to_vec() });
    }
    let first = write_ifd(&mut out, tags)?;
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
    fn dates_become_exif_dates_only_when_they_are_whole_dates() {
        assert_eq!(exif_date("1987-06-15").as_deref(), Some("1987:06:15 00:00:00"));
        assert_eq!(exif_date("1987-06-15T14:30").as_deref(), Some("1987:06:15 14:30:00"));
        assert_eq!(exif_date("2026-10-08T12:34:56Z").as_deref(), Some("2026:10:08 12:34:56"));
        assert_eq!(exif_date("2026-10-08 12:34:56").as_deref(), Some("2026:10:08 12:34:56"));
        for no in ["", "1987", "1987-06", "June 1987", "1234567890", "1987-6-5"] {
            assert_eq!(exif_date(no), None, "{no}");
        }
    }

    #[test]
    fn the_tags_come_from_the_metadata_and_empty_ones_stay_out() {
        use crate::metadata::{FrameInfo, Metadata};
        let meta = Metadata {
            creator: "Ann".into(),
            title: "Harbour".into(),
            caption: "Boats at dawn".into(),
            copyright: "© Ann".into(),
            camera: "Nikon FM2".into(),
            lens: "50mm f/1.8".into(),
            date_taken: "1987-06-15".into(),
            ..Default::default()
        };
        let frame = FrameInfo {
            scanner: "Plustek OpticFilm 8200i plustek8200i:libusb:005:033".into(),
            scan_date: "2026-10-08T12:00:00Z".into(),
            film_profile: "Kodak Portra 400".into(),
            ..Default::default()
        };
        let i = DngInfo::from_metadata(&meta, &frame, Some(400));
        assert_eq!(i.model, "Plustek OpticFilm 8200i");
        assert_eq!(i.description, "Harbour. Boats at dawn — Nikon FM2, Kodak Portra 400");
        assert_eq!((i.artist.as_str(), i.lens.as_str(), i.film_iso), ("Ann", "50mm f/1.8", Some(400)));
        assert_eq!((i.date_original.as_deref(), i.date_scanned.as_deref()), (Some("1987:06:15 00:00:00"), Some("2026:10:08 12:00:00")));
        let bare = DngInfo::from_metadata(&Metadata::default(), &FrameInfo::default(), Some(0));
        assert_eq!((bare.model.as_str(), bare.description.as_str(), bare.film_iso, bare.date_original.is_none()), ("Film scan", "", None, true));
    }

    /// A 16-bit RGB frame: black surround of the given widths, a textured lit picture inside.
    fn framed(w: usize, h: usize, (l, r, t, b): (usize, usize, usize, usize)) -> FrameBuf {
        let mut buf = FrameBuf::new(w as u32, Some(h as u32), 16, 3);
        let mut bytes = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let inside = x >= l && x < w - r && y >= t && y < h - b;
                let v: u16 = if inside { 20000 + ((x * 7 + y * 13) % 5000) as u16 } else { 300 };
                for _ in 0..3 {
                    bytes.extend(v.to_ne_bytes());
                }
            }
        }
        buf.append(&bytes);
        buf
    }

    #[test]
    fn the_black_surround_is_found_and_left_out() {
        let raw = framed(800, 600, (80, 60, 70, 40));
        let [x, y, w, h] = content_bounds(&raw);
        // Inside the lit area (80..740 × 70..560), a little in from its edge.
        assert!((80..100).contains(&x) && (70..90).contains(&y), "origin {x},{y}");
        assert!(x + w <= 740 && x + w > 720 && y + h <= 560 && y + h > 540, "far corner {},{}", x + w, y + h);
        // No surround, nothing to crop; and a flat black frame is left alone.
        assert_eq!(content_bounds(&framed(800, 600, (0, 0, 0, 0))), [0, 0, 800, 600]);
        assert_eq!(content_bounds(&framed(800, 600, (400, 399, 0, 0))), [0, 0, 800, 600], "a sliver is not a picture");
        let mut dark = FrameBuf::new(200, Some(200), 16, 3);
        dark.append(&vec![0u8; 200 * 200 * 6]);
        assert_eq!(content_bounds(&dark), [0, 0, 200, 200]);
        let mut tiny = FrameBuf::new(10, Some(10), 16, 3);
        tiny.append(&vec![9u8; 600]);
        assert_eq!(content_bounds(&tiny), [0, 0, 10, 10]);
    }

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
        let r = write_dng(
            Cursor::new(Vec::new()),
            &empty,
            Orientation::IDENTITY,
            None,
            &DngInfo { make: "M".into(), model: "m".into(), software: "s".into(), ..Default::default() },
            None,
            &|_| {},
            &AtomicBool::new(false),
        );
        assert!(matches!(r, Err(ExportError::Empty)));
        let mut b = FrameBuf::new(4, Some(4), 8, 3);
        b.append(&[7u8; 48]);
        let r = write_dng(
            Cursor::new(Vec::new()),
            &b,
            Orientation::IDENTITY,
            None,
            &DngInfo { make: "M".into(), model: "m".into(), software: "s".into(), ..Default::default() },
            None,
            &|_| {},
            &AtomicBool::new(true),
        );
        assert!(matches!(r, Err(ExportError::Cancelled)));
    }
}
