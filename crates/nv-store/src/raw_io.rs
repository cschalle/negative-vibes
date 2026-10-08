use std::io::{self, Read};
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

    /// Write a full-resolution scan as a TIFF (Deflate with the horizontal
    /// predictor), keeping the scanner's bit depth. 16-bit samples go to disk
    /// from the host's order, which the TIFF header records, so nothing is
    /// lost or swapped. Rows are written a band at a time.
    pub fn write_scan(buf: &FrameBuf, path: &Path) -> Result<()> {
        nv_export::write_scan_tiff(buf, path, nv_export::Compression::Deflate).map_err(|e| StoreError::Other(e.to_string()))
    }
}

/// Read a raw or preview file back into a FrameBuf.
pub struct RawReader;

impl RawReader {
    /// The working-size copy of a scan on disk, and the scan's size in pixels.
    ///
    /// A strip-based TIFF (what the roll stores) is reduced as it is read, a
    /// strip at a time, so opening a 7200 dpi frame to look at it never holds
    /// the 482 MB frame. Anything else (PNM, DNG, tiled TIFF) is read whole
    /// and reduced.
    pub fn proxy(path: &Path, max_pixels: usize) -> Result<(nv_process::Proxy, (u32, u32))> {
        if let Some(r) = proxy_from_strips(path, max_pixels)? {
            return Ok(r);
        }
        let buf = Self::read(path)?;
        Ok((nv_process::Proxy::from_raw(&buf, max_pixels), (buf.width, buf.effective_height())))
    }

    /// Read a PNM or TIFF file into a FrameBuf.
    pub fn read(path: &Path) -> Result<FrameBuf> {
        // A PNM is streamed from the file: reading it whole and then copying
        // it into a frame would hold the scan twice (or, with the byte swap, three times).
        let mut head = [0u8; 2];
        let n = std::fs::File::open(path)?.read(&mut head)?;
        if n == 2 && (&head == b"P6" || &head == b"P5") {
            return read_pnm_from(std::io::BufReader::with_capacity(1 << 20, std::fs::File::open(path)?));
        }
        let data = std::fs::read(path)?;
        if crate::dng::is_dng(&data) {
            crate::dng::read_dng(&data)
        } else if data.starts_with(b"II") || data.starts_with(b"MM") {
            read_tiff(path)
        } else {
            Err(StoreError::Other(format!("unknown file format: {}", path.display())))
        }
    }
}

/// The next header token of a PNM, skipping whitespace and `#` comments (a
/// comment runs to the end of the line and may sit anywhere in the header).
/// The one whitespace byte after a token is consumed with it, which is what
/// separates the last header value from the pixels.
fn pnm_token<R: std::io::BufRead>(r: &mut R) -> Result<String> {
    let mut byte = || -> Result<Option<u8>> {
        let b = r.fill_buf()?.first().copied();
        if b.is_some() {
            r.consume(1);
        }
        Ok(b)
    };
    let mut token = Vec::new();
    loop {
        match byte()? {
            None if token.is_empty() => return Err(StoreError::Other("invalid PNM header".into())),
            None => break,
            Some(b'#') => while !matches!(byte()?, Some(b'\n') | None) {},
            Some(c) if c.is_ascii_whitespace() => {
                if !token.is_empty() {
                    break;
                }
            }
            Some(c) => {
                token.push(c);
                if token.len() > 16 {
                    return Err(StoreError::Other("invalid PNM header".into()));
                }
            }
        }
    }
    String::from_utf8(token).map_err(|_| StoreError::Other("invalid PNM header".into()))
}

#[cfg(test)]
fn read_pnm(data: &[u8]) -> Result<FrameBuf> {
    read_pnm_from(std::io::Cursor::new(data))
}

/// Read a PNM from a stream into a frame, a megabyte at a time. A file that
/// ends early gives the rows it has.
fn read_pnm_from<R: std::io::BufRead>(mut r: R) -> Result<FrameBuf> {
    let bad = || StoreError::Other("invalid PNM header".into());
    let channels: u8 = match pnm_token(&mut r)?.as_str() {
        "P6" => 3,
        "P5" => 1,
        _ => return Err(bad()),
    };
    let mut number = || -> Result<u32> { pnm_token(&mut r)?.parse().map_err(|_| bad()) };
    let (w, h, maxval) = (number()?, number()?, number()?);
    let depth: u8 = if maxval > 255 { 16 } else { 8 };
    let total = w as u64 * h as u64 * channels as u64 * (depth as u64 / 8);
    if w == 0 || h == 0 || total > MAX_SCAN_BYTES {
        return Err(StoreError::Other(format!("{w}×{h} PNM is not a size we can open")));
    }

    let mut buf = FrameBuf::new(w, Some(h), depth, channels);
    let mut chunk = vec![0u8; 1 << 20];
    let mut left = total as usize;
    while left > 0 {
        let want = left.min(chunk.len());
        let got = read_full(&mut r, &mut chunk[..want])?;
        if got == 0 {
            break;
        }
        let got = if depth == 16 { got & !1 } else { got };
        if depth == 16 {
            // PNM stores 16-bit samples most significant byte first; FrameBuf
            // holds them in the host's order, the same as SANE delivers them.
            for p in chunk[..got].chunks_exact_mut(2) {
                let v = u16::from_be_bytes([p[0], p[1]]).to_ne_bytes();
                p.copy_from_slice(&v);
            }
        }
        buf.append(&chunk[..got]);
        left -= got;
        if got < want {
            break;
        }
    }
    Ok(buf)
}

/// `read_exact` that reports a short read instead of failing.
fn read_full<R: std::io::Read>(r: &mut R, out: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < out.len() {
        match r.read(&mut out[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

/// [`RawReader::proxy`] for a TIFF stored in strips, or `None` when the file
/// is something else and must be read whole.
fn proxy_from_strips(path: &Path, max_pixels: usize) -> Result<Option<(nv_process::Proxy, (u32, u32))>> {
    use tiff::decoder::{ChunkType, DecodingResult, Limits};

    let mut head = [0u8; 4];
    if std::fs::File::open(path)?.read(&mut head)? < 4 || !(&head == b"II*\0" || &head == b"MM\0*") {
        return Ok(None);
    }
    let file = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut d = tiff::decoder::Decoder::new(file)?.with_limits(Limits::unlimited());
    // A DNG's first directory is a thumbnail; its picture needs our own reader.
    if d.find_tag(tiff::tags::Tag::Unknown(50706))?.is_some() || d.get_chunk_type() != ChunkType::Strip {
        return Ok(None);
    }
    let (w, h) = d.dimensions()?;
    let (channels, depth) = match d.colortype()? {
        tiff::ColorType::Gray(8) => (1usize, 8u32),
        tiff::ColorType::Gray(16) => (1, 16),
        tiff::ColorType::RGB(8) => (3, 8),
        tiff::ColorType::RGB(16) => (3, 16),
        _ => return Ok(None),
    };
    let scale = if depth == 16 { 65535.0f32 } else { 255.0 };
    let mut builder = nv_process::ProxyBuilder::new(w as usize, h as usize, max_pixels);
    let mut y0 = 0usize;
    let mut row = vec![0f32; w as usize * 3];
    for i in 0..d.strip_count()? {
        let (cw, rows) = d.chunk_data_dimensions(i);
        let samples: Vec<f32> = match d.read_chunk(i)? {
            DecodingResult::U8(v) => v.into_iter().map(|x| x as f32 / scale).collect(),
            DecodingResult::U16(v) => v.into_iter().map(|x| x as f32 / scale).collect(),
            _ => return Ok(None),
        };
        if cw != w || samples.len() < rows as usize * w as usize * channels {
            return Err(StoreError::Other("damaged strip".into()));
        }
        for r in 0..rows as usize {
            let src = &samples[r * w as usize * channels..][..w as usize * channels];
            for x in 0..w as usize {
                let g = src[x * channels];
                row[x * 3] = g;
                row[x * 3 + 1] = if channels == 3 { src[x * channels + 1] } else { g };
                row[x * 3 + 2] = if channels == 3 { src[x * channels + 2] } else { g };
            }
            builder.add_row(y0 + r, &row);
        }
        y0 += rows as usize;
    }
    Ok(Some((builder.finish(), (w, h))))
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
    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("nv-store-proxy-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Reading a proxy from the strips of a TIFF gives what reading the whole
    /// frame and reducing it gives, for each kind of scan the roll stores.
    #[test]
    fn a_streamed_proxy_equals_reading_the_whole_frame_and_reducing_it() {
        let dir = scratch("stream");
        for (channels, depth, w, h) in [(3u8, 16u8, 150u32, 95u32), (1, 16, 120, 70), (3, 8, 64, 80), (1, 8, 33, 41)] {
            let mut buf = FrameBuf::new(w, Some(h), depth, channels);
            for i in 0..(w * h * channels as u32) {
                let v = i.wrapping_mul(2654435761) >> 9;
                if depth == 16 { buf.append(&(v as u16).to_ne_bytes()) } else { buf.append(&[v as u8]) }
            }
            let path = dir.join(format!("{channels}x{depth}.tif"));
            RawWriter::write_scan(&buf, &path).unwrap();
            let whole = nv_process::Proxy::from_raw(&RawReader::read(&path).unwrap(), 2500);
            let (streamed, size) = RawReader::proxy(&path, 2500).unwrap();
            assert_eq!(size, (w, h));
            assert_eq!((streamed.width, streamed.height, streamed.factor), (whole.width, whole.height, whole.factor), "{channels}x{depth}");
            let worst = streamed.pixels.iter().zip(&whole.pixels).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
            assert!(worst < 1e-5, "{channels}x{depth}: {worst}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Formats that cannot be streamed are read whole, and give the right picture
    /// (a DNG's first directory is a thumbnail, not the scan).
    #[test]
    fn other_formats_fall_back_to_a_whole_read() {
        let dir = scratch("fallback");
        let pnm = dir.join("a.pnm");
        std::fs::write(&pnm, b"P5\n2 1\n255\n\x10\x20").unwrap();
        let (p, size) = RawReader::proxy(&pnm, 100).unwrap();
        assert_eq!((size, p.width, p.height), ((2, 1), 2, 1));
        let dng = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/plustek8200i/bw-negative-glass-with-light.dng");
        let (p, size) = RawReader::proxy(&dng, usize::MAX).unwrap();
        assert_eq!(size, (2508, 1680), "got the thumbnail instead of the picture");
        assert_eq!((p.width, p.height), (2508, 1680));
        assert!(RawReader::proxy(&dir.join("missing.tif"), 100).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

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
