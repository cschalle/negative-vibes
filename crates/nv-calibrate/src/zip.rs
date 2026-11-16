//! A reader for the ZIP files that makers of IT8 targets ship their data in:
//! stored and deflated entries, no encryption, no ZIP64. Nothing is written to
//! disk, and an entry is refused if it would unpack to more than `MAX_ENTRY`.

use std::io::Read;

/// Nothing a target's data needs is anywhere near this size.
const MAX_ENTRY: u64 = 32 << 20;

struct Entry {
    name: String,
    method: u16,
    compressed: usize,
    offset: usize,
}

pub struct Zip<'a> {
    data: &'a [u8],
    entries: Vec<Entry>,
}

fn u16le(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(o..o.checked_add(2)?)?.try_into().ok()?))
}

fn u32le(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(o..o.checked_add(4)?)?.try_into().ok()?))
}

impl<'a> Zip<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self, String> {
        let bad = || "this is not a readable ZIP file".to_owned();
        let last = data.len().checked_sub(22).ok_or_else(bad)?;
        let first = last.saturating_sub(65535);
        let eocd = (first..=last).rev().find(|&i| data[i..i + 4] == [0x50, 0x4b, 5, 6]).ok_or_else(bad)?;
        let count = u16le(data, eocd + 10).ok_or_else(bad)? as usize;
        let mut at = u32le(data, eocd + 16).ok_or_else(bad)? as usize;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            if u32le(data, at) != Some(0x0201_4b50) {
                return Err(bad());
            }
            let (method, compressed) = (u16le(data, at + 10).ok_or_else(bad)?, u32le(data, at + 20).ok_or_else(bad)? as usize);
            let (n, m, k) = (u16le(data, at + 28).ok_or_else(bad)? as usize, u16le(data, at + 30).ok_or_else(bad)? as usize, u16le(data, at + 32).ok_or_else(bad)? as usize);
            let offset = u32le(data, at + 42).ok_or_else(bad)? as usize;
            let name = String::from_utf8_lossy(data.get(at + 46..at + 46 + n).ok_or_else(bad)?).into_owned();
            entries.push(Entry { name, method, compressed, offset });
            at += 46 + n + m + k;
        }
        Ok(Self { data, entries })
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.name.as_str())
    }

    pub fn read(&self, name: &str) -> Result<Vec<u8>, String> {
        let bad = || format!("{name} is damaged inside the ZIP file");
        let e = self.entries.iter().find(|e| e.name == name).ok_or_else(|| format!("{name} is not in the ZIP file"))?;
        if u32le(self.data, e.offset) != Some(0x0403_4b50) {
            return Err(bad());
        }
        let (n, m) = (u16le(self.data, e.offset + 26).ok_or_else(bad)? as usize, u16le(self.data, e.offset + 28).ok_or_else(bad)? as usize);
        let start = e.offset + 30 + n + m;
        let raw = self.data.get(start..start.checked_add(e.compressed).ok_or_else(bad)?).ok_or_else(bad)?;
        let mut out = Vec::new();
        match e.method {
            0 => out.extend_from_slice(raw),
            8 => {
                flate2::read::DeflateDecoder::new(raw).take(MAX_ENTRY + 1).read_to_end(&mut out).map_err(|_| bad())?;
            }
            other => return Err(format!("{name} uses compression method {other}, which is not supported")),
        }
        if out.len() as u64 > MAX_ENTRY {
            return Err(format!("{name} is too large to be a data file"));
        }
        Ok(out)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    fn crc(_: &[u8]) -> u32 {
        0 // the reader does not check it
    }

    /// A ZIP of `(name, data, deflate)` entries, put together by hand.
    pub fn build(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data, deflate) in files {
            let body = if *deflate {
                let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(data).unwrap();
                e.finish().unwrap()
            } else {
                data.to_vec()
            };
            let method: u16 = if *deflate { 8 } else { 0 };
            let offset = out.len() as u32;
            out.extend(0x0403_4b50u32.to_le_bytes());
            out.extend([20, 0, 0, 0]);
            out.extend(method.to_le_bytes());
            out.extend([0, 0, 0, 0]);
            out.extend(crc(data).to_le_bytes());
            out.extend((body.len() as u32).to_le_bytes());
            out.extend((data.len() as u32).to_le_bytes());
            out.extend((name.len() as u16).to_le_bytes());
            out.extend([0, 0]);
            out.extend(name.as_bytes());
            out.extend(&body);
            central.extend(0x0201_4b50u32.to_le_bytes());
            central.extend([20, 0, 20, 0, 0, 0]);
            central.extend(method.to_le_bytes());
            central.extend([0, 0, 0, 0]);
            central.extend(crc(data).to_le_bytes());
            central.extend((body.len() as u32).to_le_bytes());
            central.extend((data.len() as u32).to_le_bytes());
            central.extend((name.len() as u16).to_le_bytes());
            central.extend([0; 12]);
            central.extend(offset.to_le_bytes());
            central.extend(name.as_bytes());
        }
        let cd_at = out.len() as u32;
        out.extend(&central);
        out.extend(0x0605_4b50u32.to_le_bytes());
        out.extend([0, 0, 0, 0]);
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((central.len() as u32).to_le_bytes());
        out.extend(cd_at.to_le_bytes());
        out.extend([0, 0]);
        out
    }

    #[test]
    fn stored_and_deflated_entries_are_read_back() {
        let text = "A1\t\t1.95\t2.2\t1.56\n".repeat(50);
        let zip = build(&[("a/plain.txt", b"hello", false), ("a/packed.txt", text.as_bytes(), true)]);
        let z = Zip::parse(&zip).unwrap();
        assert_eq!(z.names().collect::<Vec<_>>(), ["a/plain.txt", "a/packed.txt"]);
        assert_eq!(z.read("a/plain.txt").unwrap(), b"hello");
        assert_eq!(z.read("a/packed.txt").unwrap(), text.as_bytes());
        assert!(z.read("missing").is_err());
    }

    #[test]
    fn what_is_not_a_zip_is_refused_without_panicking() {
        assert!(Zip::parse(b"").is_err());
        assert!(Zip::parse(&[7u8; 500]).is_err());
        let mut zip = build(&[("x", b"data", false)]);
        zip.truncate(zip.len() - 10);
        assert!(Zip::parse(&zip).is_err());
        // An entry whose offsets point outside the file.
        let mut zip = build(&[("x", b"data", false)]);
        let n = zip.len();
        zip[n - 6..n - 2].copy_from_slice(&0xffff_fff0u32.to_le_bytes());
        assert!(Zip::parse(&zip).is_err());
    }
}
