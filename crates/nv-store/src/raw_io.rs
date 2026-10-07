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

fn read_pnm(data: &[u8]) -> Result<FrameBuf> {
    // Parse the ASCII header without trying to decode the binary pixel data.
    // PNM header is: "<magic>\n<width> <height>\n<maxval>\n"
    // We scan for the end of the header (3 newlines after the magic line).
    let mut header_bytes = 0usize;
    let mut newlines = 0usize;
    for (i, &b) in data.iter().enumerate() {
        if b == b'\n' {
            newlines += 1;
            if newlines == 3 {
                header_bytes = i + 1;
                break;
            }
        }
    }
    if header_bytes == 0 {
        return Err(StoreError::Other("invalid PNM header".into()));
    }

    let header = std::str::from_utf8(&data[..header_bytes])
        .map_err(|e| StoreError::Other(e.to_string()))?;

    let mut iter = header.split_ascii_whitespace();
    let magic = iter.next().unwrap_or("");
    let channels: u8 = if magic == "P6" { 3 } else { 1 };

    let w: u32 = iter.next().unwrap_or("0").parse().unwrap_or(0);
    let h: u32 = iter.next().unwrap_or("0").parse().unwrap_or(0);
    let maxval: u32 = iter.next().unwrap_or("255").parse().unwrap_or(255);
    let depth: u8 = if maxval > 255 { 16 } else { 8 };

    let pixel_data = &data[header_bytes..];
    let mut buf = FrameBuf::new(w, Some(h), depth, channels);
    buf.append(pixel_data);
    Ok(buf)
}

fn read_tiff(path: &Path) -> Result<FrameBuf> {
    let file = std::fs::File::open(path)?;
    let mut decoder = tiff::decoder::Decoder::new(file)?;

    let (w, h) = decoder.dimensions()?;
    let depth: u8 = match decoder.colortype()? {
        tiff::ColorType::Gray(16) | tiff::ColorType::RGB(16) => 16,
        _ => 8,
    };
    let channels: u8 = match decoder.colortype()? {
        tiff::ColorType::Gray(_) => 1,
        tiff::ColorType::RGB(_) => 3,
        tiff::ColorType::RGBA(_) => 4,
        _ => 3,
    };

    let image = decoder.read_image()?;
    let raw = match image {
        tiff::decoder::DecodingResult::U8(v)  => v.iter().map(|&x| x as u8).collect::<Vec<_>>(),
        tiff::decoder::DecodingResult::U16(v) => {
            v.iter().flat_map(|&x| x.to_ne_bytes()).collect()
        }
        _ => return Err(StoreError::Other("unsupported TIFF sample format".into())),
    };

    let mut buf = FrameBuf::new(w, Some(h), depth, channels);
    buf.append(&raw);
    Ok(buf)
}
