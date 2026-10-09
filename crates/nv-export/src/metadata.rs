//! Metadata for exported pictures (F§10): the IPTC-style fields a person
//! fills in (byline, caption, keywords, place, copyright, …) plus what the
//! app knows about the frame (film, roll, scanner, resolution, scan date),
//! written as one XMP packet and embedded in TIFF, JPEG, PNG and DNG files.
//! XMP is what current photo software reads; the legacy IPTC-IIM block is
//! not written.
//!
//! Field sets can be saved as named presets, one JSON file each in
//! `$XDG_DATA_HOME/negative-vibes/metadata/`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What the person writes: the same for every frame of a roll, unless they
/// change it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Metadata {
    /// Who made the picture (IPTC byline).
    pub creator: String,
    pub title: String,
    pub headline: String,
    pub caption: String,
    pub keywords: Vec<String>,
    pub copyright: String,
    pub credit: String,
    pub source: String,
    pub instructions: String,
    pub city: String,
    pub province: String,
    pub country: String,
    /// When the photograph was taken: `2024`, `2024-06`, `2024-06-15` or with a time.
    pub date_taken: String,
    /// The original camera and lens, and the film.
    pub camera: String,
    pub lens: String,
    pub film_stock: String,
}

impl Metadata {
    pub fn is_empty(&self) -> bool {
        *self == Metadata::default()
    }

    /// Fields that are filled in, with leading and trailing space removed.
    pub fn cleaned(&self) -> Metadata {
        let t = |s: &str| s.trim().to_owned();
        Metadata {
            creator: t(&self.creator),
            title: t(&self.title),
            headline: t(&self.headline),
            caption: t(&self.caption),
            keywords: self.keywords.iter().map(|k| k.trim().to_owned()).filter(|k| !k.is_empty()).collect(),
            copyright: t(&self.copyright),
            credit: t(&self.credit),
            source: t(&self.source),
            instructions: t(&self.instructions),
            city: t(&self.city),
            province: t(&self.province),
            country: t(&self.country),
            date_taken: t(&self.date_taken),
            camera: t(&self.camera),
            lens: t(&self.lens),
            film_stock: t(&self.film_stock),
        }
    }

    /// Keywords as typed, separated by commas or semicolons.
    pub fn parse_keywords(text: &str) -> Vec<String> {
        text.split([',', ';', '\n']).map(|k| k.trim().to_owned()).filter(|k| !k.is_empty()).collect()
    }
}

/// `YYYY`, `YYYY-MM`, `YYYY-MM-DD`, or either of the last with `Thh:mm[:ss]`.
pub fn valid_date(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return true;
    }
    let (date, time) = match s.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (s, None),
    };
    let parts: Vec<&str> = date.split('-').collect();
    let num = |p: &str, len: usize| p.len() == len && p.bytes().all(|b| b.is_ascii_digit());
    let ok_date = match parts.as_slice() {
        [y] => num(y, 4),
        [y, m] => num(y, 4) && num(m, 2) && (1..=12).contains(&m.parse::<u32>().unwrap_or(0)),
        [y, m, d] => num(y, 4) && num(m, 2) && num(d, 2) && (1..=12).contains(&m.parse::<u32>().unwrap_or(0)) && (1..=31).contains(&d.parse::<u32>().unwrap_or(0)),
        _ => false,
    };
    let ok_time = match time {
        None => true,
        Some(t) => {
            parts.len() == 3 && {
                let hms: Vec<&str> = t.split(':').collect();
                (hms.len() == 2 || hms.len() == 3)
                    && hms.iter().all(|p| num(p, 2))
                    && hms[0].parse::<u32>().unwrap_or(99) < 24
                    && hms[1].parse::<u32>().unwrap_or(99) < 60
                    && hms.get(2).is_none_or(|s| s.parse::<u32>().unwrap_or(99) < 60)
            }
        }
    };
    ok_date && ok_time
}

/// What the app knows about one frame being exported.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrameInfo {
    pub roll: String,
    pub frame_number: Option<u32>,
    /// The frame's own label, used as the title when none was written.
    pub label: String,
    pub scanner: String,
    pub resolution_dpi: Option<u32>,
    /// ISO 8601.
    pub scan_date: String,
    pub film_profile: String,
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            c if (c as u32) < 0x20 && !matches!(c, '\n' | '\t' | '\r') => {} // not allowed in XML
            c => o.push(c),
        }
    }
    o
}

fn alt(tag: &str, text: &str) -> String {
    format!("<{tag}><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></{tag}>", esc(text))
}

fn list(tag: &str, kind: &str, items: &[String]) -> String {
    let lis: String = items.iter().map(|i| format!("<rdf:li>{}</rdf:li>", esc(i))).collect();
    format!("<{tag}><rdf:{kind}>{lis}</rdf:{kind}></{tag}>")
}

/// The XMP packet for a frame: the person's fields, then what the app knows.
pub fn xmp_packet(meta: &Metadata, frame: &FrameInfo) -> Vec<u8> {
    let m = meta.cleaned();
    let mut p = String::new();
    let mut put = |s: String| p.push_str(&s);
    // Dublin Core.
    let title = if m.title.is_empty() { frame.label.trim().to_owned() } else { m.title.clone() };
    if !title.is_empty() {
        put(alt("dc:title", &title));
    }
    if !m.creator.is_empty() {
        put(list("dc:creator", "Seq", std::slice::from_ref(&m.creator)));
    }
    if !m.caption.is_empty() {
        put(alt("dc:description", &m.caption));
    }
    if !m.copyright.is_empty() {
        put(alt("dc:rights", &m.copyright));
    }
    if !m.keywords.is_empty() {
        put(list("dc:subject", "Bag", &m.keywords));
    }
    // Photoshop / IPTC core.
    for (tag, v) in [
        ("photoshop:Headline", &m.headline),
        ("photoshop:Credit", &m.credit),
        ("photoshop:Source", &m.source),
        ("photoshop:Instructions", &m.instructions),
        ("photoshop:City", &m.city),
        ("photoshop:State", &m.province),
        ("photoshop:Country", &m.country),
        ("photoshop:DateCreated", &m.date_taken),
    ] {
        if !v.is_empty() {
            put(format!("<{tag}>{}</{tag}>", esc(v)));
        }
    }
    if !m.date_taken.is_empty() {
        put(format!("<exif:DateTimeOriginal>{}</exif:DateTimeOriginal>", esc(&m.date_taken)));
    }
    if !m.copyright.is_empty() {
        put("<xmpRights:Marked>True</xmpRights:Marked>".to_owned());
    }
    // The original camera, if the person said.
    if !m.camera.is_empty() {
        put(format!("<nv:OriginalCamera>{}</nv:OriginalCamera>", esc(&m.camera)));
    }
    if !m.lens.is_empty() {
        put(format!("<nv:OriginalLens>{}</nv:OriginalLens>", esc(&m.lens)));
    }
    let film = if m.film_stock.is_empty() { frame.film_profile.trim().to_owned() } else { m.film_stock.clone() };
    if !film.is_empty() {
        put(format!("<nv:FilmStock>{}</nv:FilmStock>", esc(&film)));
    }
    // What the app knows.
    put(format!("<xmp:CreatorTool>Negative Vibes {}</xmp:CreatorTool>", env!("CARGO_PKG_VERSION")));
    if !frame.scan_date.is_empty() {
        put(format!("<xmp:CreateDate>{}</xmp:CreateDate>", esc(&frame.scan_date)));
    }
    if !frame.scanner.is_empty() {
        put(format!("<nv:Scanner>{}</nv:Scanner>", esc(&frame.scanner)));
    }
    if let Some(dpi) = frame.resolution_dpi {
        put(format!("<nv:ScanResolution>{dpi}</nv:ScanResolution>"));
    }
    if !frame.roll.is_empty() {
        put(format!("<nv:Roll>{}</nv:Roll>", esc(&frame.roll)));
    }
    if let Some(n) = frame.frame_number {
        put(format!("<nv:Frame>{n}</nv:Frame>"));
    }

    format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n\
<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
<rdf:Description rdf:about=\"\" \
xmlns:dc=\"http://purl.org/dc/elements/1.1/\" \
xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" \
xmlns:xmpRights=\"http://ns.adobe.com/xap/1.0/rights/\" \
xmlns:photoshop=\"http://ns.adobe.com/photoshop/1.0/\" \
xmlns:exif=\"http://ns.adobe.com/exif/1.0/\" \
xmlns:nv=\"{NV_NAMESPACE}\">\n{p}\n</rdf:Description>\n\
</rdf:RDF>\n\
</x:xmpmeta>\n\
<?xpacket end=\"w\"?>"
    )
    .into_bytes()
}

/// The app's own XMP namespace.
pub const NV_NAMESPACE: &str = "https://negative-vibes.app/ns/1.0/";

// ── Presets ───────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum PresetError {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Parse(#[from] serde_json::Error),
}

pub fn presets_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("negative-vibes/metadata")
}

/// File name stem for a preset name: letters, digits, dashes.
fn stem(name: &str) -> String {
    let mut s = String::new();
    for c in name.trim().chars() {
        if c.is_alphanumeric() {
            s.extend(c.to_lowercase());
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    s.trim_end_matches('-').to_owned()
}

#[derive(Serialize, Deserialize)]
struct PresetFile {
    name: String,
    #[serde(flatten)]
    meta: Metadata,
}

pub fn save_preset_in(dir: &Path, name: &str, meta: &Metadata) -> Result<PathBuf, PresetError> {
    let name = name.trim();
    let file = stem(name);
    if name.is_empty() || file.is_empty() {
        return Err(PresetError::Invalid("the preset needs a name".into()));
    }
    if !valid_date(&meta.date_taken) {
        return Err(PresetError::Invalid("the date must look like 2024-06-15".into()));
    }
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{file}.json"));
    std::fs::write(&path, serde_json::to_vec_pretty(&PresetFile { name: name.to_owned(), meta: meta.clone() })?)?;
    Ok(path)
}

pub fn save_preset(name: &str, meta: &Metadata) -> Result<PathBuf, PresetError> {
    save_preset_in(&presets_dir(), name, meta)
}

/// The presets in `dir`, by name.
pub fn list_presets_in(dir: &Path) -> Vec<(String, Metadata)> {
    let mut out: Vec<(String, Metadata)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_slice::<PresetFile>(&std::fs::read(e.path()).ok()?).ok())
        .map(|p| (p.name, p.meta))
        .collect();
    out.sort_by_key(|a| a.0.to_lowercase());
    out
}

pub fn list_presets() -> Vec<(String, Metadata)> {
    list_presets_in(&presets_dir())
}

pub fn delete_preset_in(dir: &Path, name: &str) -> Result<(), PresetError> {
    let file = stem(name);
    if file.is_empty() {
        return Err(PresetError::Invalid("no such preset".into()));
    }
    std::fs::remove_file(dir.join(format!("{file}.json")))?;
    Ok(())
}

pub fn delete_preset(name: &str) -> Result<(), PresetError> {
    delete_preset_in(&presets_dir(), name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full() -> Metadata {
        Metadata {
            creator: "Ann <Photo> & Co".into(),
            title: "Harbour".into(),
            headline: "Morning".into(),
            caption: "Boats at \"dawn\"".into(),
            keywords: vec!["boats".into(), " harbour ".into(), "".into()],
            copyright: "© 1987 Ann".into(),
            credit: "Ann".into(),
            source: "Family archive".into(),
            instructions: "Do not crop".into(),
            city: "Oslo".into(),
            province: "Oslo".into(),
            country: "Norway".into(),
            date_taken: "1987-06-15".into(),
            camera: "Nikon FM2".into(),
            lens: "50mm f/1.8".into(),
            film_stock: "Kodak Portra 400".into(),
        }
    }

    fn frame() -> FrameInfo {
        FrameInfo {
            roll: "Roll 7".into(),
            frame_number: Some(12),
            label: "".into(),
            scanner: "Plustek OpticFilm 8200i".into(),
            resolution_dpi: Some(3600),
            scan_date: "2026-10-08T12:00:00Z".into(),
            film_profile: "Portra 400".into(),
        }
    }

    #[test]
    fn the_packet_is_well_formed_escaped_xml_with_every_field() {
        let x = String::from_utf8(xmp_packet(&full(), &frame())).unwrap();
        assert!(x.starts_with("<?xpacket begin=") && x.trim_end().ends_with("<?xpacket end=\"w\"?>"));
        for needle in [
            "<dc:creator><rdf:Seq><rdf:li>Ann &lt;Photo&gt; &amp; Co</rdf:li>",
            "Boats at &quot;dawn&quot;",
            "<rdf:li>boats</rdf:li><rdf:li>harbour</rdf:li></rdf:Bag>",
            "<photoshop:City>Oslo</photoshop:City>",
            "<photoshop:DateCreated>1987-06-15</photoshop:DateCreated>",
            "<nv:FilmStock>Kodak Portra 400</nv:FilmStock>",
            "<nv:ScanResolution>3600</nv:ScanResolution>",
            "<nv:Frame>12</nv:Frame>",
            "<xmp:CreateDate>2026-10-08T12:00:00Z</xmp:CreateDate>",
            "© 1987 Ann",
        ] {
            assert!(x.contains(needle), "missing {needle}\n{x}");
        }
        // Tags balance (a cheap well-formedness check).
        let opens = x.matches('<').count();
        let closes = x.matches('>').count();
        assert_eq!(opens, closes);
    }

    #[test]
    fn empty_fields_are_left_out_and_the_frame_label_is_the_fallback_title() {
        let x = String::from_utf8(xmp_packet(&Metadata::default(), &FrameInfo { label: "Quay".into(), ..Default::default() })).unwrap();
        assert!(x.contains("<dc:title>") && x.contains("Quay"));
        for absent in ["<dc:creator", "<photoshop:", "<dc:subject", "xmpRights:Marked", "<nv:Roll", "<nv:Frame"] {
            assert!(!x.contains(absent), "{absent} should be absent:\n{x}");
        }
        assert!(x.contains("xmp:CreatorTool"));
        // The film shown is the profile's unless the person named a stock.
        let x = String::from_utf8(xmp_packet(&Metadata::default(), &frame())).unwrap();
        assert!(x.contains("<nv:FilmStock>Portra 400"));
    }

    #[test]
    fn control_characters_cannot_break_the_xml() {
        let m = Metadata { caption: "a\u{0}b\u{1b}c\nd".into(), ..Default::default() };
        let x = String::from_utf8(xmp_packet(&m, &FrameInfo::default())).unwrap();
        assert!(x.contains("abc\nd"));
    }

    #[test]
    fn dates_are_checked() {
        for good in ["", "1987", "1987-06", "1987-06-15", "1987-06-15T14:30", "1987-06-15T14:30:59"] {
            assert!(valid_date(good), "{good}");
        }
        for bad in ["87", "1987-13", "1987-06-32", "June 1987", "1987-06-15T25:00", "1987-06T10:00", "1987-6-5"] {
            assert!(!valid_date(bad), "{bad}");
        }
    }

    #[test]
    fn keywords_split_on_commas_and_semicolons() {
        assert_eq!(Metadata::parse_keywords(" a, b;c ,, \n d"), ["a", "b", "c", "d"]);
        assert!(Metadata::default().is_empty() && !full().is_empty());
    }

    #[test]
    fn presets_save_list_and_delete() {
        let dir = std::env::temp_dir().join(format!("nv-meta-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        save_preset_in(&dir, "Family archive", &full()).unwrap();
        save_preset_in(&dir, "Client — Smith", &Metadata { creator: "Me".into(), ..Default::default() }).unwrap();
        let all = list_presets_in(&dir);
        assert_eq!(all.iter().map(|p| p.0.as_str()).collect::<Vec<_>>(), ["Client — Smith", "Family archive"]);
        assert_eq!(all[1].1, full());
        assert!(save_preset_in(&dir, "  ", &full()).is_err());
        assert!(save_preset_in(&dir, "Bad date", &Metadata { date_taken: "yesterday".into(), ..Default::default() }).is_err());
        delete_preset_in(&dir, "Family archive").unwrap();
        assert_eq!(list_presets_in(&dir).len(), 1);
        assert!(delete_preset_in(&dir, "../x").is_err() || !dir.join("../x.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_settings_without_metadata_still_load() {
        let m: Metadata = serde_json::from_str("{}").unwrap();
        assert!(m.is_empty());
        let m: Metadata = serde_json::from_str(r#"{"creator":"A","future_field":1}"#).unwrap();
        assert_eq!(m.creator, "A");
    }
}
