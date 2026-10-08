use std::path::Path;
use nv_image::FrameBuf;
use crate::{Result, StoreError};

/// Write a FrameBuf as a 16-bit tiled TIFF (our raw / preview format).
pub struct RawWriter;

impl RawWriter {
    pub fn write_preview(buf: &FrameBuf, path: &Path) -> Result<()> {
        let w = buf.width;
        let h = buf.effective_height();
        if w == 0 || h == 0 {
            return Err(StoreError::Other("empty frame buffer".into()));
        }

        // Build an 8-bit RGB PPM as a fast fallback (TIFF writing needs libtiff or
        // the `tiff` crate for tiled 16-bit — implemented in M4).
        // For M2 previews we write a simple PNM that can be loaded back.
        let rgb8 = buf.to_rgb8();
        let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
        out.extend_from_slice(&rgb8);
        std::fs::write(path, out)?;
        Ok(())
    }

    /// Write a full-resolution scan as an uncompressed TIFF, keeping the
    /// scanner's bit depth (16-bit samples are stored in the host's order,
    /// which the TIFF header records, so nothing is lost or swapped).
    pub fn write_scan(buf: &FrameBuf, path: &Path) -> Result<()> {
        use tiff::encoder::{colortype, TiffEncoder};

        let w = buf.width;
        let h = buf.effective_height();
        if w == 0 || h == 0 {
            return Err(StoreError::Other("empty frame buffer".into()));
        }

        let bps = buf.depth as usize / 8;
        let row_len = w as usize * buf.channels as usize * bps;
        let bytes = buf.as_bytes();
        if bytes.len() < buf.bytes_per_row * h as usize {
            return Err(StoreError::Other("incomplete scan data".into()));
        }
        // Drop any per-line alignment padding the backend added.
        let mut packed = Vec::with_capacity(row_len * h as usize);
        for row in 0..h as usize {
            let start = row * buf.bytes_per_row;
            packed.extend_from_slice(&bytes[start..start + row_len]);
        }

        let file = std::io::BufWriter::new(std::fs::File::create(path)?);
        let mut enc = TiffEncoder::new(file)?;
        let samples16 = || -> Vec<u16> {
            packed.chunks_exact(2).map(|p| u16::from_ne_bytes([p[0], p[1]])).collect()
        };
        match (buf.channels, buf.depth) {
            (3, 16) => enc.write_image::<colortype::RGB16>(w, h, &samples16())?,
            (3, 8) => enc.write_image::<colortype::RGB8>(w, h, &packed)?,
            (1, 16) => enc.write_image::<colortype::Gray16>(w, h, &samples16())?,
            (1, 8) => enc.write_image::<colortype::Gray8>(w, h, &packed)?,
            (c, d) => {
                return Err(StoreError::Other(format!(
                    "cannot write {c}-channel {d}-bit scan"
                )))
            }
        }
        Ok(())
    }
}

/// Read a raw or preview file back into a FrameBuf.
pub struct RawReader;

impl RawReader {
    /// Read a PNM or TIFF file into a FrameBuf.
    pub fn read(path: &Path) -> Result<FrameBuf> {
        let data = std::fs::read(path)?;
        if data.starts_with(b"P6") || data.starts_with(b"P5") {
            read_pnm(&data)
        } else if data.starts_with(b"II") || data.starts_with(b"MM") {
            read_tiff(path)
        } else {
            Err(StoreError::Other(format!("unknown file format: {}", path.display())))
        }
    }
}

/// Read the next whitespace-delimited header token, skipping `#` comments
/// (which run to the end of the line and may sit anywhere in the header).
fn pnm_token<'a>(data: &'a [u8], pos: &mut usize) -> Option<&'a str> {
    loop {
        match data.get(*pos)? {
            b'#' => {
                while *data.get(*pos)? != b'\n' {
                    *pos += 1;
                }
            }
            c if c.is_ascii_whitespace() => *pos += 1,
            _ => break,
        }
    }
    let start = *pos;
    while data.get(*pos).is_some_and(|c| !c.is_ascii_whitespace() && *c != b'#') {
        *pos += 1;
    }
    std::str::from_utf8(&data[start..*pos]).ok()
}

fn read_pnm(data: &[u8]) -> Result<FrameBuf> {
    let bad = || StoreError::Other("invalid PNM header".into());
    let mut pos = 0;
    let magic = pnm_token(data, &mut pos).ok_or_else(bad)?;
    let channels: u8 = match magic {
        "P6" => 3,
        "P5" => 1,
        _ => return Err(bad()),
    };
    let mut number = || -> Result<u32> { pnm_token(data, &mut pos).and_then(|t| t.parse().ok()).ok_or_else(bad) };
    let (w, h, maxval) = (number()?, number()?, number()?);
    let depth: u8 = if maxval > 255 { 16 } else { 8 };
    // Exactly one whitespace byte separates the header from the pixels.
    let header_bytes = pos + 1;
    if header_bytes > data.len() {
        return Err(bad());
    }

    let pixel_data = &data[header_bytes..];
    let mut buf = FrameBuf::new(w, Some(h), depth, channels);
    if depth == 16 {
        // PNM stores 16-bit samples most significant byte first; FrameBuf
        // holds them in the host's order, the same as SANE delivers them.
        let swapped: Vec<u8> = pixel_data
            .chunks_exact(2)
            .flat_map(|p| u16::from_be_bytes([p[0], p[1]]).to_ne_bytes())
            .collect();
        buf.append(&swapped);
    } else {
        buf.append(pixel_data);
    }
    Ok(buf)
}

/// Largest scan we agree to read: a 7200 dpi frame is 482 MB, and this leaves
/// room for several times that, but not for a corrupt header that claims a
/// terabyte.
const MAX_SCAN_BYTES: u64 = 4 << 30;

fn read_tiff(path: &Path) -> Result<FrameBuf> {
    use tiff::decoder::{ChunkType, DecodingResult, Limits};

    let file = std::io::BufReader::new(std::fs::File::open(path)?);
    // The crate's default limits (256 MB per decoded buffer) refuse exactly
    // the frames this program exists to scan. Our own size check replaces them.
    let mut decoder = tiff::decoder::Decoder::new(file)?.with_limits(Limits::unlimited());

    let (w, h) = decoder.dimensions()?;
    let colortype = decoder.colortype()?;
    let depth: u8 = match colortype {
        tiff::ColorType::Gray(16) | tiff::ColorType::RGB(16) => 16,
        _ => 8,
    };
    let channels: u8 = match colortype {
        tiff::ColorType::Gray(_) => 1,
        tiff::ColorType::RGB(_) => 3,
        tiff::ColorType::RGBA(_) => 4,
        _ => 3,
    };
    let bytes = w as u64 * h as u64 * channels as u64 * (depth as u64 / 8);
    if bytes > MAX_SCAN_BYTES {
        return Err(StoreError::Other(format!("{w}×{h} scan is too large to open ({} MB)", bytes >> 20)));
    }

    let mut buf = FrameBuf::new(w, Some(h), depth, channels);
    let append = |buf: &mut FrameBuf, image: DecodingResult| -> Result<()> {
        match image {
            DecodingResult::U8(v) => buf.append(&v),
            // Samples are kept in the host's order, as SANE delivers them.
            // Converted a block at a time: a whole-image byte copy would
            // double the memory of a big scan.
            DecodingResult::U16(v) => {
                let mut bytes = Vec::with_capacity(1 << 20);
                for block in v.chunks(1 << 19) {
                    bytes.clear();
                    bytes.extend(block.iter().flat_map(|x| x.to_ne_bytes()));
                    buf.append(&bytes);
                }
            }
            _ => return Err(StoreError::Other("unsupported TIFF sample format".into())),
        }
        Ok(())
    };

    if decoder.get_chunk_type() == ChunkType::Strip {
        // A strip at a time: the file's samples and the frame never both
        // exist in full.
        let strips = decoder.strip_count()?;
        for i in 0..strips {
            let chunk = decoder.read_chunk(i)?;
            append(&mut buf, chunk)?;
        }
    } else {
        append(&mut buf, decoder.read_image()?)?;
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::{read_pnm, RawReader, RawWriter};

    /// `pnmtopnm`, GIMP and the SANE tools write comments into the header.
    #[test]
    fn pnm_header_comments_are_skipped() {
        let mut data = b"P6\n# SANE data follows\n2 1 # size\n255\n".to_vec();
        data.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        let buf = read_pnm(&data).expect("header with comments");
        assert_eq!((buf.width, buf.effective_height(), buf.depth, buf.channels), (2, 1, 8, 3));
        assert_eq!(buf.as_bytes(), &[1, 2, 3, 4, 5, 6]);
    }

    /// Pixel data that happens to start with a `#` byte or a newline is not a
    /// comment or whitespace to skip: it comes after the single separator.
    #[test]
    fn pnm_pixels_starting_with_hash_or_newline_survive() {
        let mut data = b"P5\n2 1\n255\n".to_vec();
        data.extend_from_slice(&[b'#', b'\n']);
        let buf = read_pnm(&data).unwrap();
        assert_eq!(buf.as_bytes(), &[b'#', b'\n']);
    }
    use nv_image::{read_sample, FrameBuf};

    /// A 16-bit scan written to disk must read back sample for sample,
    /// including when the backend pads each line.
    #[test]
    fn sixteen_bit_scan_round_trips_through_tiff() {
        let (w, h, stride) = (2u32, 2u32, 16usize); // 12 data bytes + 4 padding
        let mut buf = FrameBuf::with_stride(w, Some(h), 16, 3, stride);
        let mut next = 0x1001u16;
        for _ in 0..h {
            let mut row = Vec::new();
            for _ in 0..w * 3 {
                row.extend_from_slice(&next.to_ne_bytes());
                next += 0x0111;
            }
            row.extend_from_slice(&[0xee; 4]);
            buf.append(&row);
        }

        let path = std::env::temp_dir().join("nv-store-scan-roundtrip.tif");
        RawWriter::write_scan(&buf, &path).expect("write");
        let back = RawReader::read(&path).expect("read");
        let _ = std::fs::remove_file(&path);

        assert_eq!((back.width, back.effective_height(), back.depth, back.channels), (2, 2, 16, 3));
        let mut expect = 0x1001u32;
        for i in 0..(w * h * 3) as usize {
            assert_eq!(read_sample(back.as_bytes(), i * 2, 2), expect, "sample {i}");
            expect += 0x0111;
        }
    }

    /// PNM stores 16-bit samples most significant byte first. FrameBuf holds
    /// them in the host's order, the same as SANE delivers them, so the
    /// reader has to convert. Loading the bytes verbatim only looked right
    /// because the writer was not converting either.
    #[test]
    fn sixteen_bit_pnm_is_big_endian_on_disk() {
        let mut data = b"P6\n1 1\n65535\n".to_vec();
        data.extend_from_slice(&0x1234u16.to_be_bytes());
        data.extend_from_slice(&0x5678u16.to_be_bytes());
        data.extend_from_slice(&0x9abcu16.to_be_bytes());

        let buf = read_pnm(&data).expect("valid PNM");
        assert_eq!(buf.depth, 16);
        assert_eq!(buf.channels, 3);
        assert_eq!(nv_image::read_sample(buf.as_bytes(), 0, 2), 0x1234);
        assert_eq!(nv_image::read_sample(buf.as_bytes(), 2, 2), 0x5678);
        assert_eq!(nv_image::read_sample(buf.as_bytes(), 4, 2), 0x9abc);
    }

    #[test]
    fn eight_bit_pnm_is_untouched() {
        let mut data = b"P6\n1 1\n255\n".to_vec();
        data.extend_from_slice(&[0x11, 0x22, 0x33]);
        let buf = read_pnm(&data).expect("valid PNM");
        assert_eq!(buf.depth, 8);
        assert_eq!(buf.as_bytes()[..3], [0x11, 0x22, 0x33]);
    }
}
