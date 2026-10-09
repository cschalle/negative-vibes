//! A small EXIF block for JPEG and PNG files: when the photograph was taken,
//! and a description. Photo services (Google Photos among them) place a
//! picture on their timeline by `DateTimeOriginal`; without it a scan of a
//! 1987 holiday lands on the day it was scanned.

use crate::dng::exif_date;
use crate::metadata::Metadata;

/// `YYYY:MM:DD HH:MM:SS` for any date the metadata allows. A whole date with no
/// time gets noon (so no time zone moves it to another day); a month alone
/// the 15th; a year alone the middle of the year.
pub fn exif_date_loose(iso: &str) -> Option<String> {
    let s = iso.trim();
    if let Some(d) = exif_date(s) {
        // `exif_date` gives midnight for a date with no time; use noon instead.
        let has_time = s.contains(['T', ' ']);
        return Some(if has_time { d } else { format!("{} 12:00:00", &d[..10]) });
    }
    let digits = |p: &str, n: usize| p.len() == n && p.bytes().all(|b| b.is_ascii_digit());
    let parts: Vec<&str> = s.split('-').collect();
    match parts.as_slice() {
        [y] if digits(y, 4) => Some(format!("{y}:07:01 12:00:00")),
        [y, m] if digits(y, 4) && digits(m, 2) && (1..=12).contains(&m.parse::<u32>().unwrap_or(0)) => Some(format!("{y}:{m}:15 12:00:00")),
        _ => None,
    }
}

const ASCII: u16 = 2;
const UNDEFINED: u16 = 7;
const LONG: u16 = 4;

struct Entry {
    tag: u16,
    typ: u16,
    count: u32,
    data: Vec<u8>,
}

fn ascii(tag: u16, s: &str) -> Entry {
    // EXIF ASCII is 7-bit and NUL-terminated; keep what fits.
    let mut data: Vec<u8> = s.chars().map(|c| if c.is_ascii() && c != '\0' { c as u8 } else { b'?' }).collect();
    data.push(0);
    Entry { tag, typ: ASCII, count: data.len() as u32, data }
}

/// Lay out one IFD at `at` (an offset from the start of the TIFF header). Returns the bytes of the
/// IFD followed by its out-of-line values. Entries must be in tag order.
fn ifd(entries: &[Entry], at: u32) -> Vec<u8> {
    let table = 2 + entries.len() as u32 * 12 + 4;
    let mut head = Vec::new();
    let mut values = Vec::new();
    head.extend((entries.len() as u16).to_le_bytes());
    for e in entries {
        head.extend(e.tag.to_le_bytes());
        head.extend(e.typ.to_le_bytes());
        head.extend(e.count.to_le_bytes());
        if e.data.len() <= 4 {
            let mut v = e.data.clone();
            v.resize(4, 0);
            head.extend(v);
        } else {
            head.extend((at + table + values.len() as u32).to_le_bytes());
            values.extend(&e.data);
            if values.len() % 2 == 1 {
                values.push(0); // values start on a word boundary
            }
        }
    }
    head.extend(0u32.to_le_bytes()); // no next IFD
    head.extend(values);
    head
}

/// The EXIF data to embed (the TIFF structure, without the `Exif\0\0` that a
/// JPEG segment puts in front), or `None` when there is nothing worth saying.
pub fn exif_block(meta: &Metadata) -> Option<Vec<u8>> {
    let date = exif_date_loose(&meta.date_taken);
    let description = [meta.title.trim(), meta.caption.trim()].into_iter().find(|s| !s.is_empty());
    if date.is_none() && description.is_none() {
        return None;
    }
    let mut exif = vec![Entry { tag: 0x9000, typ: UNDEFINED, count: 4, data: b"0230".to_vec() }];
    if let Some(d) = &date {
        exif.push(ascii(0x9003, d)); // DateTimeOriginal
        exif.push(ascii(0x9004, d)); // DateTimeDigitized: Google reads the one it finds first
    }
    let mut ifd0 = Vec::new();
    if let Some(text) = description {
        ifd0.push(ascii(0x010E, text));
    }
    ifd0.push(Entry { tag: 0x8769, typ: LONG, count: 1, data: vec![0; 4] }); // patched below

    // IFD0 sits right after the 8-byte header; the EXIF IFD follows it.
    let ifd0_len = ifd(&ifd0, 8).len() as u32;
    let exif_at = 8 + ifd0_len + (ifd0_len % 2);
    let last = ifd0.last_mut()?;
    last.data = exif_at.to_le_bytes().to_vec();

    let mut out = b"II*\0".to_vec();
    out.extend(8u32.to_le_bytes());
    out.extend(ifd(&ifd0, 8));
    if out.len() % 2 == 1 {
        out.push(0);
    }
    debug_assert_eq!(out.len() as u32, exif_at);
    out.extend(ifd(&exif, exif_at));
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16_at(b: &[u8], o: usize) -> u16 {
        u16::from_le_bytes([b[o], b[o + 1]])
    }
    fn u32_at(b: &[u8], o: usize) -> u32 {
        u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
    }

    /// Find a tag in the IFD at `at` and return its value bytes.
    fn find(b: &[u8], at: usize, tag: u16) -> Option<Vec<u8>> {
        let n = u16_at(b, at) as usize;
        for i in 0..n {
            let e = at + 2 + i * 12;
            if u16_at(b, e) == tag {
                let count = u32_at(b, e + 4) as usize;
                let size = match u16_at(b, e + 2) {
                    ASCII | UNDEFINED => count,
                    LONG => count * 4,
                    _ => return None,
                };
                return Some(if size <= 4 { b[e + 8..e + 8 + size].to_vec() } else { b[u32_at(b, e + 8) as usize..][..size].to_vec() });
            }
        }
        None
    }

    #[test]
    fn partial_dates_become_usable_timestamps() {
        assert_eq!(exif_date_loose("1987").as_deref(), Some("1987:07:01 12:00:00"));
        assert_eq!(exif_date_loose("1987-06").as_deref(), Some("1987:06:15 12:00:00"));
        assert_eq!(exif_date_loose("1987-06-15").as_deref(), Some("1987:06:15 12:00:00"));
        assert_eq!(exif_date_loose("1987-06-15T14:30").as_deref(), Some("1987:06:15 14:30:00"));
        for no in ["", "June", "1987-13", "87"] {
            assert_eq!(exif_date_loose(no), None, "{no}");
        }
    }

    #[test]
    fn nothing_to_say_gives_no_block() {
        assert_eq!(exif_block(&Metadata::default()), None);
    }

    #[test]
    fn the_block_is_readable_and_carries_the_date_and_description() {
        let m = Metadata { date_taken: "1987-06-15".into(), caption: "Beach at Skagen".into(), ..Default::default() };
        let b = exif_block(&m).unwrap();
        assert_eq!(&b[..4], b"II*\0");
        let ifd0 = u32_at(&b, 4) as usize;
        assert_eq!(find(&b, ifd0, 0x010E).unwrap(), b"Beach at Skagen\0");
        let exif_at = u32_at(&find(&b, ifd0, 0x8769).unwrap(), 0) as usize;
        assert_eq!(find(&b, exif_at, 0x9003).unwrap(), b"1987:06:15 12:00:00\0");
        assert_eq!(find(&b, exif_at, 0x9000).unwrap(), b"0230");
    }

    #[test]
    fn a_date_alone_works_and_odd_text_is_kept_ascii() {
        let b = exif_block(&Metadata { date_taken: "1999".into(), ..Default::default() }).unwrap();
        let ifd0 = u32_at(&b, 4) as usize;
        assert!(find(&b, ifd0, 0x010E).is_none());
        let b = exif_block(&Metadata { title: "Café".into(), ..Default::default() }).unwrap();
        assert_eq!(find(&b, u32_at(&b, 4) as usize, 0x010E).unwrap(), b"Caf?\0");
    }
}
