//! A streaming TIFF writer.
//!
//! The `tiff` crate (0.9) writes a compressed image corrupt when it is given
//! as strips (whole-image writes are fine, but a 7200 dpi frame must not be
//! held whole), and it cannot apply a predictor. So the container is written
//! here: strips are compressed and written as the bands arrive, and the
//! directory goes at the end, once every strip's offset is known.
//!
//! Little-endian, classic TIFF (offsets are 32-bit, so a file over 4 GB is
//! refused; an 80-megapixel 16-bit RGB frame is 480 MB). Chunky RGB or grey,
//! (an optional fourth sample is the infrared plane), 8 or 16 bit, optional LZW or Deflate with the horizontal predictor.

use std::io::{self, Seek, SeekFrom, Write};

use crate::settings::Compression;

const SHORT: u16 = 3;
const LONG: u16 = 4;
const ASCII: u16 = 2;
const UNDEFINED: u16 = 7;

pub struct TiffWriter<W: Write + Seek> {
    out: W,
    width: u32,
    height: u32,
    channels: u16,
    depth: u16,
    compression: Compression,
    rows_per_strip: u32,
    profile: Option<Vec<u8>>,
    strips: Vec<(u32, u32)>,
    rows_written: u32,
}

impl<W: Write + Seek> TiffWriter<W> {
    pub fn new(mut out: W, width: u32, height: u32, channels: u16, depth: u16, compression: Compression, rows_per_strip: u32, profile: Option<Vec<u8>>) -> io::Result<Self> {
        // Header: byte order, magic, and room for the directory offset.
        out.write_all(b"II*\0\0\0\0\0")?;
        Ok(Self { out, width, height, channels, depth, compression, rows_per_strip, profile, strips: Vec::new(), rows_written: 0 })
    }

    /// Write the next `rows` rows as strips of `rows_per_strip`, compressing
    /// the strips of this call on all cores. `samples` are 8-bit.
    pub fn write_rows_u8(&mut self, rows: u32, samples: &[u8]) -> io::Result<()> {
        self.write_rows(rows, samples.len(), |range| samples[range].to_vec())
    }

    /// The same for 16-bit samples in native order.
    pub fn write_rows_u16(&mut self, rows: u32, samples: &[u16]) -> io::Result<()> {
        self.write_rows(rows, samples.len() * 2, |range| samples[range.start / 2..range.end / 2].iter().flat_map(|x| x.to_le_bytes()).collect())
    }

    /// `bytes_of(range)` yields the little-endian bytes of that byte range of
    /// the band; each strip is predicted, compressed and queued in order.
    fn write_rows(&mut self, rows: u32, total_bytes: usize, bytes_of: impl Fn(std::ops::Range<usize>) -> Vec<u8> + Sync) -> io::Result<()> {
        let row_bytes = self.width as usize * self.channels as usize * (self.depth as usize / 8);
        if total_bytes != rows as usize * row_bytes || self.rows_written + rows > self.height {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "rows do not match the image"));
        }
        // Strip boundaries within this band.
        let strips: Vec<(u32, std::ops::Range<usize>)> = (0..rows)
            .step_by(self.rows_per_strip as usize)
            .map(|r| (self.rows_per_strip.min(rows - r), r as usize * row_bytes..(r + self.rows_per_strip.min(rows - r)) as usize * row_bytes))
            .collect();

        let (compression, step, row_len, depth) = (self.compression, self.channels as usize, self.width as usize * self.channels as usize, self.depth);
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).min(strips.len()).max(1);
        let next = std::sync::atomic::AtomicUsize::new(0);
        let mut done: Vec<Option<io::Result<Vec<u8>>>> = (0..strips.len()).map(|_| None).collect();
        let (send, recv) = std::sync::mpsc::channel::<(usize, io::Result<Vec<u8>>)>();
        std::thread::scope(|scope| {
            for _ in 0..threads {
                let (send, next, strips, bytes_of) = (send.clone(), &next, &strips, &bytes_of);
                scope.spawn(move || loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if i >= strips.len() {
                        break;
                    }
                    let _ = send.send((i, compress(bytes_of(strips[i].1.clone()), compression, depth, row_len, step)));
                });
            }
            drop(send);
            for (i, r) in recv {
                done[i] = Some(r);
            }
        });

        for ((strip_rows, _), result) in strips.iter().zip(done) {
            let bytes = result.ok_or_else(|| io::Error::other("a strip was not compressed"))??;
            let at = self.out.stream_position()?;
            let off = u32::try_from(at).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "TIFF over 4 GB"))?;
            self.out.write_all(&bytes)?;
            // Keep strips word-aligned; the byte count excludes the pad.
            if bytes.len() % 2 == 1 {
                self.out.write_all(&[0])?;
            }
            self.strips.push((off, bytes.len() as u32));
            self.rows_written += strip_rows;
        }
        Ok(())
    }

    /// Write the directory and finish. Fails if rows are missing.
    pub fn finish(mut self) -> io::Result<()> {
        if self.rows_written != self.height {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "fewer rows than the image has"));
        }
        let mut entries: Vec<(u16, u16, u32, Vec<u8>)> = Vec::new();
        let short = |v: u16| v.to_le_bytes().to_vec();
        let shorts = |v: &[u16]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
        let longs = |v: &[u32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();

        entries.push((256, LONG, 1, longs(&[self.width])));
        entries.push((257, LONG, 1, longs(&[self.height])));
        entries.push((258, SHORT, self.channels as u32, shorts(&vec![self.depth; self.channels as usize])));
        entries.push((259, SHORT, 1, short(match self.compression {
            Compression::None => 1,
            Compression::Lzw => 5,
            Compression::Deflate => 8,
        })));
        entries.push((262, SHORT, 1, short(if self.channels == 1 { 1 } else { 2 })));
        let n = self.strips.len() as u32;
        entries.push((273, LONG, n, longs(&self.strips.iter().map(|s| s.0).collect::<Vec<_>>())));
        entries.push((277, SHORT, 1, short(self.channels)));
        entries.push((278, LONG, 1, longs(&[self.rows_per_strip])));
        entries.push((279, LONG, n, longs(&self.strips.iter().map(|s| s.1).collect::<Vec<_>>())));
        entries.push((284, SHORT, 1, short(1)));
        let software = concat!("Negative Vibes ", env!("CARGO_PKG_VERSION"), "\0");
        entries.push((305, ASCII, software.len() as u32, software.as_bytes().to_vec()));
        if self.compression != Compression::None {
            entries.push((317, SHORT, 1, short(2))); // horizontal differencing
        }
        if self.channels == 4 {
            // The fourth sample is infrared, which has no TIFF name: "unspecified".
            entries.push((338, SHORT, 1, short(0)));
        }
        if let Some(p) = &self.profile {
            entries.push((34675, UNDEFINED, p.len() as u32, p.clone()));
        }
        entries.sort_by_key(|e| e.0);

        // Values over 4 bytes go after the directory's own entries.
        let ifd_at = self.out.stream_position()?;
        let ifd_at = ifd_at + ifd_at % 2;
        let table_len = 2 + 12 * entries.len() as u64 + 4;
        let mut extra_at = ifd_at + table_len;
        let mut table = Vec::new();
        let mut extra: Vec<u8> = Vec::new();
        table.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for (tag, typ, count, value) in &entries {
            table.extend_from_slice(&tag.to_le_bytes());
            table.extend_from_slice(&typ.to_le_bytes());
            table.extend_from_slice(&count.to_le_bytes());
            if value.len() <= 4 {
                let mut inline = value.clone();
                inline.resize(4, 0);
                table.extend_from_slice(&inline);
            } else {
                let off = u32::try_from(extra_at).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "TIFF over 4 GB"))?;
                table.extend_from_slice(&off.to_le_bytes());
                extra.extend_from_slice(value);
                if value.len() % 2 == 1 {
                    extra.push(0);
                }
                extra_at = ifd_at + table_len + extra.len() as u64;
            }
        }
        table.extend_from_slice(&0u32.to_le_bytes()); // no further directory

        if ifd_at > self.out.stream_position()? {
            self.out.write_all(&[0])?;
        }
        self.out.write_all(&table)?;
        self.out.write_all(&extra)?;
        let ifd = u32::try_from(ifd_at).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "TIFF over 4 GB"))?;
        self.out.seek(SeekFrom::Start(4))?;
        self.out.write_all(&ifd.to_le_bytes())?;
        self.out.flush()
    }
}

/// Predict (for compressed output) and compress one strip of little-endian bytes.
fn compress(mut data: Vec<u8>, compression: Compression, depth: u16, row_len: usize, step: usize) -> io::Result<Vec<u8>> {
    if compression == Compression::None {
        return Ok(data);
    }
    if depth == 16 {
        let mut v: Vec<u16> = data.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
        predict_u16(&mut v, row_len, step);
        data = v.iter().flat_map(|x| x.to_le_bytes()).collect();
    } else {
        predict_u8(&mut data, row_len, step);
    }
    match compression {
        Compression::None => Ok(data),
        Compression::Deflate => {
            let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
            z.write_all(&data)?;
            z.finish()
        }
        Compression::Lzw => weezl::encode::Encoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8)
            .encode(&data)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string())),
    }
}

/// Horizontal differencing: each sample minus the one `step` before it in
/// its row, which makes smooth gradients compress far better.
fn predict_u8(data: &mut [u8], row_len: usize, step: usize) {
    for row in data.chunks_mut(row_len) {
        for i in (step..row.len()).rev() {
            row[i] = row[i].wrapping_sub(row[i - step]);
        }
    }
}

fn predict_u16(data: &mut [u16], row_len: usize, step: usize) {
    for row in data.chunks_mut(row_len) {
        for i in (step..row.len()).rev() {
            row[i] = row[i].wrapping_sub(row[i - step]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn differencing_is_undone_by_a_running_sum() {
        let original: Vec<u16> = vec![10, 20, 30, 12, 24, 36, 14, 29, 40, 100, 200, 300];
        let mut d = original.clone();
        predict_u16(&mut d, 6, 3);
        assert_eq!(&d[..3], &[10, 20, 30], "first pixel of a row is kept");
        assert_eq!(&d[3..6], &[2, 4, 6]);
        // Undo like a TIFF reader: running sum along the row.
        for row in d.chunks_mut(6) {
            for i in 3..row.len() {
                row[i] = row[i].wrapping_add(row[i - 3]);
            }
        }
        assert_eq!(d, original);
    }

    #[test]
    fn eight_bit_differencing_wraps() {
        let mut d = vec![250u8, 3];
        predict_u8(&mut d, 2, 1);
        assert_eq!(d, vec![250, 9]); // 3 - 250 mod 256
    }
}
