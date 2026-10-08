//! Reading linear DNG files: the "raw" format SilverFast, VueScan and others
//! save film scans in. A DNG is a TIFF whose first directory is a small
//! preview and whose real picture sits in a sub-directory (`SubIFDs`), which
//! the general TIFF reader does not follow.
//!
//! Only what a film scanner writes is read: uncompressed, strip-based, 8 or
//! 16 bits, one or three samples per pixel, photometric BlackIsZero, RGB or
//! LinearRaw. Mosaic (camera) raws, compression and tiles are refused with
//! a message. Files come from anywhere, so every offset and size is checked
//! before use.

use nv_image::FrameBuf;

use crate::{Result, StoreError};

const TAG_WIDTH: u16 = 256;
const TAG_HEIGHT: u16 = 257;
const TAG_BITS: u16 = 258;
const TAG_COMPRESSION: u16 = 259;
const TAG_PHOTOMETRIC: u16 = 262;
const TAG_STRIP_OFFSETS: u16 = 273;
const TAG_SAMPLES: u16 = 277;
const TAG_ROWS_PER_STRIP: u16 = 278;
const TAG_STRIP_COUNTS: u16 = 279;
const TAG_PLANAR: u16 = 284;
const TAG_SUB_IFDS: u16 = 330;
const TAG_TILE_WIDTH: u16 = 322;
const TAG_DNG_VERSION: u16 = 50706;
const TAG_BLACK_LEVEL: u16 = 50714;
const TAG_WHITE_LEVEL: u16 = 50717;

const PHOTOMETRIC_BLACK_IS_ZERO: u32 = 1;
const PHOTOMETRIC_RGB: u32 = 2;
const PHOTOMETRIC_LINEAR_RAW: u32 = 34892;

/// Largest picture accepted: matches the TIFF reader's ceiling.
const MAX_BYTES: u64 = 4 << 30;
/// Most directories followed, so a loop in the file cannot run forever.
const MAX_IFDS: usize = 16;

fn err(msg: &str) -> StoreError {
    StoreError::Other(format!("DNG: {msg}"))
}

struct Bytes<'a> {
    d: &'a [u8],
    little: bool,
}

#[derive(Clone, Copy)]
struct Entry {
    tag: u16,
    typ: u16,
    count: u32,
    /// Where the value bytes are: inline in the entry, or at its offset.
    at: usize,
}

impl Bytes<'_> {
    fn u16(&self, o: usize) -> Option<u16> {
        let b: [u8; 2] = self.d.get(o..o.checked_add(2)?)?.try_into().ok()?;
        Some(if self.little { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    }

    fn u32(&self, o: usize) -> Option<u32> {
        let b: [u8; 4] = self.d.get(o..o.checked_add(4)?)?.try_into().ok()?;
        Some(if self.little { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    }

    fn ifd(&self, off: usize) -> Option<Vec<Entry>> {
        let n = self.u16(off)? as usize;
        (0..n)
            .map(|i| {
                let p = off.checked_add(2 + 12 * i)?;
                let (typ, count) = (self.u16(p + 2)?, self.u32(p + 4)?);
                let size = match typ {
                    1 | 2 | 6 | 7 => 1usize,
                    3 | 8 => 2,
                    4 | 9 | 11 => 4,
                    5 | 10 | 12 => 8,
                    _ => 1,
                };
                let bytes = size.checked_mul(count as usize)?;
                let at = if bytes <= 4 { p + 8 } else { self.u32(p + 8)? as usize };
                Some(Entry { tag: self.u16(p)?, typ, count, at })
            })
            .collect()
    }

    /// The first value of an integer entry.
    fn first(&self, e: &Entry) -> Option<u32> {
        self.nth(e, 0)
    }

    fn nth(&self, e: &Entry, i: usize) -> Option<u32> {
        if i >= e.count as usize {
            return None;
        }
        match e.typ {
            3 => self.u16(e.at.checked_add(2 * i)?).map(u32::from),
            4 => self.u32(e.at.checked_add(4 * i)?),
            1 => self.d.get(e.at.checked_add(i)?).map(|b| *b as u32),
            _ => None,
        }
    }

    fn all(&self, e: &Entry) -> Option<Vec<u32>> {
        // The directory sits in the file, so a count beyond what the file
        // could hold is a lie, not a reason to allocate.
        if e.count as usize > self.d.len() {
            return None;
        }
        (0..e.count as usize).map(|i| self.nth(e, i)).collect()
    }
}

fn find(entries: &[Entry], tag: u16) -> Option<&Entry> {
    entries.iter().find(|e| e.tag == tag)
}

/// Whether `data` is a DNG: a TIFF whose first directory carries `DNGVersion`.
pub fn is_dng(data: &[u8]) -> bool {
    let little = match data.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return false,
    };
    let b = Bytes { d: data, little };
    b.u16(2) == Some(42)
        && b.u32(4)
            .and_then(|o| b.ifd(o as usize))
            .is_some_and(|e| find(&e, TAG_DNG_VERSION).is_some())
}

/// Read the largest picture in a linear DNG.
pub fn read_dng(data: &[u8]) -> Result<FrameBuf> {
    let little = match data.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return Err(err("not a TIFF file")),
    };
    let b = Bytes { d: data, little };
    if b.u16(2) != Some(42) {
        return Err(err("not a TIFF file"));
    }

    // Every directory: the chain from the first, and each one's SubIFDs.
    let mut todo = vec![b.u32(4).ok_or_else(|| err("truncated header"))? as usize];
    let mut seen = Vec::new();
    let mut ifds: Vec<Vec<Entry>> = Vec::new();
    while let Some(off) = todo.pop() {
        if off == 0 || seen.contains(&off) || seen.len() >= MAX_IFDS {
            continue;
        }
        seen.push(off);
        let entries = b.ifd(off).ok_or_else(|| err("damaged directory"))?;
        if let Some(subs) = find(&entries, TAG_SUB_IFDS).and_then(|e| b.all(e)) {
            todo.extend(subs.into_iter().map(|o| o as usize));
        }
        // The next directory in the chain follows the entries.
        let n = b.u16(off).unwrap_or(0) as usize;
        if let Some(next) = b.u32(off + 2 + 12 * n) {
            todo.push(next as usize);
        }
        ifds.push(entries);
    }

    let area = |e: &Vec<Entry>| -> u64 {
        let get = |t| find(e, t).and_then(|x| b.first(x)).unwrap_or(0) as u64;
        get(TAG_WIDTH) * get(TAG_HEIGHT)
    };
    let main = ifds.iter().max_by_key(|e| area(e)).filter(|e| area(e) > 0).ok_or_else(|| err("no image found"))?;

    let get = |tag| find(main, tag).and_then(|e| b.first(e));
    let (w, h) = (get(TAG_WIDTH).ok_or_else(|| err("no width"))?, get(TAG_HEIGHT).ok_or_else(|| err("no height"))?);
    let bits = get(TAG_BITS).unwrap_or(1);
    let spp = get(TAG_SAMPLES).unwrap_or(1);
    let photometric = get(TAG_PHOTOMETRIC).unwrap_or(PHOTOMETRIC_BLACK_IS_ZERO);

    if get(TAG_COMPRESSION).unwrap_or(1) != 1 {
        return Err(err("compressed DNG is not supported"));
    }
    if find(main, TAG_TILE_WIDTH).is_some() {
        return Err(err("tiled DNG is not supported"));
    }
    if get(TAG_PLANAR).unwrap_or(1) != 1 {
        return Err(err("planar layout is not supported"));
    }
    if !matches!(photometric, PHOTOMETRIC_BLACK_IS_ZERO | PHOTOMETRIC_RGB | PHOTOMETRIC_LINEAR_RAW) {
        return Err(err("camera mosaic raw is not supported; this reads film-scanner DNGs"));
    }
    if !matches!(bits, 8 | 16) || !matches!(spp, 1 | 3) {
        return Err(err(&format!("{spp} samples of {bits} bits is not supported")));
    }
    if w == 0 || h == 0 || w > 1 << 20 || h > 1 << 20 {
        return Err(err("implausible image size"));
    }
    let bps = bits as usize / 8;
    let row_bytes = w as usize * spp as usize * bps;
    let total = row_bytes as u64 * h as u64;
    if total > MAX_BYTES {
        return Err(err(&format!("{w}×{h} is too large to open")));
    }

    let offsets = find(main, TAG_STRIP_OFFSETS).and_then(|e| b.all(e)).ok_or_else(|| err("no strip offsets"))?;
    let counts = find(main, TAG_STRIP_COUNTS).and_then(|e| b.all(e)).ok_or_else(|| err("no strip sizes"))?;
    if offsets.len() != counts.len() || offsets.is_empty() {
        return Err(err("strip tables disagree"));
    }
    let rows_per_strip = get(TAG_ROWS_PER_STRIP).unwrap_or(h).max(1) as usize;

    // DNG's black and white levels: scale so that white is full scale.
    let level = |tag| find(main, tag).and_then(|e| b.first(e));
    let black = level(TAG_BLACK_LEVEL).unwrap_or(0);
    let full = (1u32 << bits) - 1;
    let white = level(TAG_WHITE_LEVEL).filter(|w| *w > black).unwrap_or(full);
    let rescale = black != 0 || white != full;

    let mut buf = FrameBuf::new(w, Some(h), bits as u8, spp as u8);
    let mut rows_done = 0usize;
    let mut chunk = Vec::new();
    for (off, count) in offsets.iter().zip(&counts) {
        if rows_done >= h as usize {
            break;
        }
        let rows = rows_per_strip.min(h as usize - rows_done);
        let want = rows * row_bytes;
        if (*count as usize) < want {
            return Err(err("strip is shorter than its rows"));
        }
        let strip = data
            .get(*off as usize..(*off as usize).checked_add(want).ok_or_else(|| err("bad strip offset"))?)
            .ok_or_else(|| err("strip lies outside the file"))?;
        chunk.clear();
        if bps == 1 {
            chunk.extend_from_slice(strip);
        } else {
            // Samples in the host's order, as SANE hands them over.
            for p in strip.chunks_exact(2) {
                let mut v = if little { u16::from_le_bytes([p[0], p[1]]) } else { u16::from_be_bytes([p[0], p[1]]) } as u32;
                if rescale {
                    v = (v.saturating_sub(black) as u64 * full as u64 / (white - black) as u64).min(full as u64) as u32;
                }
                chunk.extend_from_slice(&(v as u16).to_ne_bytes());
            }
        }
        buf.append(&chunk);
        rows_done += rows;
    }
    if rows_done < h as usize {
        return Err(err("fewer rows than the image has"));
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nv_image::read_sample;

    const TAG_NEW_SUBFILE: u16 = 254;

    /// Build a tiny DNG: IFD0 is a 2×1 thumbnail with `DNGVersion` and a
    /// SubIFD; the SubIFD is the real `w × h` picture.
    struct Dng {
        little: bool,
        w: u32,
        h: u32,
        bits: u16,
        spp: u16,
        photometric: u16,
        compression: u16,
        rows_per_strip: u32,
        extra: Vec<(u16, u16, u32)>, // tag, type (3 = short), value
        samples: Vec<u16>,
    }

    impl Dng {
        fn grey16(little: bool, w: u32, h: u32, samples: Vec<u16>) -> Self {
            Dng { little, w, h, bits: 16, spp: 1, photometric: 34892, compression: 1, rows_per_strip: 1, extra: vec![], samples }
        }

        fn put16(&self, v: &mut Vec<u8>, x: u16) {
            v.extend_from_slice(&if self.little { x.to_le_bytes() } else { x.to_be_bytes() });
        }

        fn put32(&self, v: &mut Vec<u8>, x: u32) {
            v.extend_from_slice(&if self.little { x.to_le_bytes() } else { x.to_be_bytes() });
        }

        fn entry(&self, v: &mut Vec<u8>, tag: u16, typ: u16, count: u32, val: u32) {
            self.put16(v, tag);
            self.put16(v, typ);
            self.put32(v, count);
            // Inline short values sit in the first two bytes of the field.
            if typ == 3 && count == 1 {
                self.put16(v, val as u16);
                self.put16(v, 0);
            } else {
                self.put32(v, val);
            }
        }

        fn bytes(&self) -> Vec<u8> {
            let mut v = Vec::new();
            v.extend_from_slice(if self.little { b"II" } else { b"MM" });
            self.put16(&mut v, 42);
            self.put32(&mut v, 8);

            // IFD0: 5 entries = 2 + 60 + 4 bytes, at 8..74. Thumbnail at 74.
            let ifd0 = 8;
            let sub_at = ifd0 + 2 + 5 * 12 + 4 + 2; // after the 2-byte thumbnail pixels
            self.put16(&mut v, 5);
            self.entry(&mut v, TAG_NEW_SUBFILE, 4, 1, 1);
            self.entry(&mut v, TAG_WIDTH, 3, 1, 1);
            self.entry(&mut v, TAG_HEIGHT, 3, 1, 1);
            self.entry(&mut v, TAG_DNG_VERSION, 1, 4, 0x0004_0001);
            self.entry(&mut v, TAG_SUB_IFDS, 4, 1, sub_at as u32);
            self.put32(&mut v, 0);
            self.put16(&mut v, 0); // thumbnail pixels (unused)
            assert_eq!(v.len(), sub_at);

            // The SubIFD.
            let n_strips = self.h.div_ceil(self.rows_per_strip);
            let n_entries = 9 + self.extra.len();
            let table_at = sub_at + 2 + n_entries * 12 + 4;
            let (offs_at, counts_at) = (table_at, table_at + 4 * n_strips as usize);
            // One strip keeps its offset and size inline: no tables to skip.
            let data_at = if n_strips > 1 { counts_at + 4 * n_strips as usize } else { table_at };
            self.put16(&mut v, n_entries as u16);
            self.entry(&mut v, TAG_WIDTH, 4, 1, self.w);
            self.entry(&mut v, TAG_HEIGHT, 4, 1, self.h);
            self.entry(&mut v, TAG_BITS, 3, 1, self.bits as u32);
            self.entry(&mut v, TAG_COMPRESSION, 3, 1, self.compression as u32);
            self.entry(&mut v, TAG_PHOTOMETRIC, 3, 1, self.photometric as u32);
            self.entry(&mut v, TAG_STRIP_OFFSETS, 4, n_strips, if n_strips == 1 { data_at as u32 } else { offs_at as u32 });
            self.entry(&mut v, TAG_SAMPLES, 3, 1, self.spp as u32);
            self.entry(&mut v, TAG_ROWS_PER_STRIP, 4, 1, self.rows_per_strip);
            let row_bytes = self.w * self.spp as u32 * (self.bits as u32 / 8);
            self.entry(&mut v, TAG_STRIP_COUNTS, 4, n_strips, if n_strips == 1 { row_bytes * self.h } else { counts_at as u32 });
            for (tag, typ, val) in &self.extra {
                self.entry(&mut v, *tag, *typ, 1, *val);
            }
            self.put32(&mut v, 0);
            if n_strips > 1 {
                for s in 0..n_strips {
                    self.put32(&mut v, data_at as u32 + s * self.rows_per_strip * row_bytes);
                }
                for _ in 0..n_strips {
                    self.put32(&mut v, self.rows_per_strip * row_bytes);
                }
            }
            for s in &self.samples {
                if self.bits == 16 {
                    self.put16(&mut v, *s);
                } else {
                    v.push(*s as u8);
                }
            }
            v
        }
    }

    fn samples_of(b: &FrameBuf) -> Vec<u32> {
        let bps = b.depth as usize / 8;
        (0..b.as_bytes().len() / bps).map(|i| read_sample(b.as_bytes(), i * bps, bps)).collect()
    }

    #[test]
    fn reads_the_subifd_picture_not_the_thumbnail_in_either_byte_order() {
        for little in [true, false] {
            let dng = Dng::grey16(little, 3, 2, vec![100, 200, 300, 40000, 50000, 65535]);
            let bytes = dng.bytes();
            assert!(is_dng(&bytes));
            let buf = read_dng(&bytes).unwrap();
            assert_eq!((buf.width, buf.effective_height(), buf.depth, buf.channels), (3, 2, 16, 1));
            assert_eq!(samples_of(&buf), [100, 200, 300, 40000, 50000, 65535], "little={little}");
        }
    }

    #[test]
    fn several_strips_join_in_order() {
        let mut dng = Dng::grey16(false, 2, 5, (1..=10).collect());
        dng.rows_per_strip = 2; // 3 strips: 2, 2 and 1 rows
        assert_eq!(samples_of(&read_dng(&dng.bytes()).unwrap()), (1..=10).collect::<Vec<u32>>());
    }

    #[test]
    fn eight_bit_rgb_is_read_as_it_is() {
        let mut dng = Dng::grey16(true, 2, 1, vec![1, 2, 3, 4, 5, 6]);
        dng.bits = 8;
        dng.spp = 3;
        dng.photometric = 2;
        let buf = read_dng(&dng.bytes()).unwrap();
        assert_eq!((buf.depth, buf.channels), (8, 3));
        assert_eq!(buf.as_bytes(), &[1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn white_and_black_levels_scale_to_full_range() {
        let mut dng = Dng::grey16(true, 3, 1, vec![100, 600, 1100]);
        dng.extra = vec![(TAG_BLACK_LEVEL, 3, 100), (TAG_WHITE_LEVEL, 3, 1100)];
        assert_eq!(samples_of(&read_dng(&dng.bytes()).unwrap()), [0, 32767, 65535]);
    }

    #[test]
    fn what_it_cannot_read_is_refused_with_a_reason() {
        let mut compressed = Dng::grey16(true, 2, 1, vec![1, 2]);
        compressed.compression = 7;
        assert!(read_dng(&compressed.bytes()).err().unwrap().to_string().contains("compressed"));

        let mut mosaic = Dng::grey16(true, 2, 1, vec![1, 2]);
        mosaic.photometric = 32803;
        assert!(read_dng(&mosaic.bytes()).err().unwrap().to_string().contains("mosaic"));

        let mut tiled = Dng::grey16(true, 2, 1, vec![1, 2]);
        tiled.extra = vec![(TAG_TILE_WIDTH, 3, 16)];
        assert!(read_dng(&tiled.bytes()).err().unwrap().to_string().contains("tiled"));
    }

    /// Files come from the internet: nothing in them may panic the reader.
    #[test]
    fn damaged_files_are_errors_not_panics() {
        let good = Dng::grey16(false, 3, 2, vec![1, 2, 3, 4, 5, 6]).bytes();
        // Truncated at every length.
        for n in 0..good.len() {
            let _ = read_dng(&good[..n]);
        }
        // Every single byte flipped to 0xFF in turn.
        for i in 0..good.len() {
            let mut bad = good.clone();
            bad[i] = 0xFF;
            let _ = read_dng(&bad);
        }
        assert!(read_dng(b"").is_err() && read_dng(b"II*\0").is_err() && read_dng(&[0u8; 64]).is_err());
    }

    #[test]
    fn a_plain_tiff_is_not_a_dng() {
        let mut v = b"II*\0\x08\0\0\0".to_vec();
        v.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        assert!(!is_dng(&v));
    }
}
