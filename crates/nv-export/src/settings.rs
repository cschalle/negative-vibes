//! What an export does, in one serialisable value: stored with the roll and
//! as presets.

use serde::{Deserialize, Serialize};

use crate::colour::ColourSpace;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExportFormat {
    #[default]
    Tiff,
    Jpeg,
    Png,
    /// The scan itself as a linear DNG: no processing, no colour space.
    Dng,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Tiff => "tif",
            ExportFormat::Jpeg => "jpg",
            ExportFormat::Png => "png",
            ExportFormat::Dng => "dng",
        }
    }

    /// JPEG is 8 bit only.
    pub fn allows_16_bit(self) -> bool {
        self != ExportFormat::Jpeg
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Compression {
    #[default]
    None,
    Lzw,
    Deflate,
}

/// How big the exported picture is. It is never made larger than the scan:
/// enlarging adds no detail, and the scan can simply be made at a higher
/// resolution.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum OutputSize {
    /// Every pixel of the scan.
    #[default]
    Original,
    /// The longer side is this many pixels.
    LongEdge { pixels: u32 },
    /// As large as fits in this box, keeping the proportions.
    Fit { width: u32, height: u32 },
    /// This percentage of the scan's size.
    Percent { percent: f32 },
}

impl OutputSize {
    /// The picture's size for a `w × h` picture.
    pub fn dims(&self, w: usize, h: usize) -> (usize, usize) {
        let scale = match *self {
            OutputSize::Original => 1.0,
            OutputSize::LongEdge { pixels } => pixels.max(1) as f64 / w.max(h).max(1) as f64,
            OutputSize::Fit { width, height } => (width.max(1) as f64 / w.max(1) as f64).min(height.max(1) as f64 / h.max(1) as f64),
            OutputSize::Percent { percent } => (percent as f64).clamp(1.0, 100.0) / 100.0,
        }
        .min(1.0);
        if scale >= 1.0 {
            return (w, h);
        }
        let round = |v: usize| ((v as f64 * scale).round() as usize).clamp(1, v.max(1));
        (round(w), round(h))
    }

    /// The named sizes of F§9, as the dialog lists them.
    pub fn presets() -> Vec<(&'static str, OutputSize)> {
        vec![
            ("Original size", OutputSize::Original),
            ("Web, 2048 px", OutputSize::LongEdge { pixels: 2048 }),
            ("Social, 1080 px", OutputSize::LongEdge { pixels: 1080 }),
            ("Presentation, Full HD (1920 × 1080)", OutputSize::Fit { width: 1920, height: 1080 }),
            ("Presentation, HD ready (1280 × 720)", OutputSize::Fit { width: 1280, height: 720 }),
            ("Half size", OutputSize::Percent { percent: 50.0 }),
        ]
    }
}

/// What the scans are for (the guided flow's second step): each sets the
/// resolution and the export settings that suit it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// The best the scanner gives, kept as a 16-bit master.
    Archive,
    /// Small enough for the web and social media.
    Share,
    /// Enough for a large print.
    Print,
}

impl Purpose {
    pub const ALL: [Purpose; 3] = [Purpose::Archive, Purpose::Share, Purpose::Print];

    pub fn name(self) -> &'static str {
        match self {
            Purpose::Archive => "Archive",
            Purpose::Share => "Share",
            Purpose::Print => "Print",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Purpose::Archive => "Highest resolution, 16-bit TIFF in Adobe RGB, plus the raw scan",
            Purpose::Share => "JPEG in sRGB, 3000 pixels on the long side",
            Purpose::Print => "16-bit TIFF in Adobe RGB at 2400 dpi, enough for a large print",
        }
    }

    /// The scan resolution wanted, before it is fitted to what the scanner offers.
    pub fn dpi(self) -> u32 {
        match self {
            Purpose::Archive => 3600,
            Purpose::Share | Purpose::Print => 2400,
        }
    }

    /// Change `s` to suit this purpose; the folder, naming and metadata stay.
    pub fn apply(self, s: &mut ExportSettings) {
        let base = ExportSettings::default();
        let (format, bit_depth, compression, jpeg_quality, colour_space, size) = match self {
            Purpose::Archive => (ExportFormat::Tiff, 16, Compression::Deflate, base.jpeg_quality, ColourSpace::AdobeRgb, OutputSize::Original),
            Purpose::Share => (ExportFormat::Jpeg, 8, base.compression, 90, ColourSpace::Srgb, OutputSize::LongEdge { pixels: 3000 }),
            Purpose::Print => (ExportFormat::Tiff, 16, Compression::Lzw, base.jpeg_quality, ColourSpace::AdobeRgb, OutputSize::Original),
        };
        s.format = format;
        s.bit_depth = bit_depth;
        s.compression = compression;
        s.jpeg_quality = jpeg_quality;
        s.colour_space = colour_space;
        s.size = size;
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportSettings {
    pub format: ExportFormat,
    /// 8 or 16.
    pub bit_depth: u8,
    /// TIFF only.
    pub compression: Compression,
    /// JPEG only, 1–100.
    pub jpeg_quality: u8,
    pub colour_space: ColourSpace,
    pub size: OutputSize,
    /// One grey channel instead of three (B&W film).
    pub grey: bool,
    pub embed_profile: bool,
    /// Where files go; `None` is `~/Pictures/Scans/<roll>`.
    pub folder: Option<std::path::PathBuf>,
    /// See [`crate::naming`].
    pub template: String,
    /// Export automatically after each scan.
    pub after_scan: bool,
    /// What the person wrote for the roll (F§10), and whether it goes into the files.
    pub metadata: crate::metadata::Metadata,
    pub embed_metadata: bool,
    /// Send each exported picture to Google Photos once it is written.
    pub google_photos: bool,
    /// ... into an album named after the roll.
    pub google_album: bool,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            format: ExportFormat::Tiff,
            bit_depth: 16,
            compression: Compression::Deflate,
            jpeg_quality: 92,
            colour_space: ColourSpace::Srgb,
            size: OutputSize::Original,
            grey: false,
            embed_profile: true,
            folder: None,
            template: "{roll}-{frame:02}".to_owned(),
            after_scan: true,
            metadata: Default::default(),
            embed_metadata: true,
            google_photos: false,
            google_album: true,
        }
    }
}

impl ExportSettings {
    /// The settings as they will really be used: a JPEG is 8 bit, depths
    /// are 8 or 16, quality is 1–100.
    pub fn normalised(&self) -> Self {
        let mut s = self.clone();
        if !s.format.allows_16_bit() || s.bit_depth != 16 {
            s.bit_depth = 8;
        }
        s.jpeg_quality = s.jpeg_quality.clamp(1, 100);
        s
    }

    /// What suits Google Photos: an 8-bit sRGB JPEG, large enough to look good on any
    /// screen and small enough to upload quickly, sent on as soon as it is written.
    pub fn google_photos_preset() -> Self {
        ExportSettings {
            format: ExportFormat::Jpeg,
            bit_depth: 8,
            jpeg_quality: 92,
            colour_space: ColourSpace::Srgb,
            size: OutputSize::LongEdge { pixels: 4000 },
            google_photos: true,
            ..Default::default()
        }
    }

    /// The three purposes of DESIGN.md §8.5.
    pub fn presets() -> Vec<(&'static str, ExportSettings)> {
        vec![
            ("Archive", ExportSettings { format: ExportFormat::Tiff, bit_depth: 16, compression: Compression::Deflate, colour_space: ColourSpace::AdobeRgb, ..Default::default() }),
            ("Share", ExportSettings { format: ExportFormat::Jpeg, bit_depth: 8, jpeg_quality: 90, colour_space: ColourSpace::Srgb, ..Default::default() }),
            ("Print", ExportSettings { format: ExportFormat::Tiff, bit_depth: 8, compression: Compression::Lzw, colour_space: ColourSpace::AdobeRgb, ..Default::default() }),
            ("Google Photos", ExportSettings::google_photos_preset()),
        ]
    }
}

#[cfg(test)]
mod purpose_tests {
    use super::*;

    #[test]
    fn each_purpose_sets_a_distinct_valid_export_and_leaves_the_rest() {
        let mut seen = Vec::new();
        for p in Purpose::ALL {
            let mut s = ExportSettings { folder: Some("/x".into()), template: "{frame}".into(), embed_metadata: false, ..Default::default() };
            p.apply(&mut s);
            assert_eq!(s, s.normalised(), "{} must already be a valid setting", p.name());
            assert_eq!((s.folder.as_deref(), s.template.as_str(), s.embed_metadata), (Some(std::path::Path::new("/x")), "{frame}", false));
            assert!(p.dpi() >= 600 && !p.description().is_empty());
            seen.push((s.format, s.bit_depth, s.colour_space, s.size));
        }
        seen.dedup();
        assert_eq!(seen.len(), 3);
        let mut share = ExportSettings::default();
        Purpose::Share.apply(&mut share);
        assert_eq!((share.format, share.bit_depth, share.size.dims(5000, 3000)), (ExportFormat::Jpeg, 8, (3000, 1800)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_jpeg_is_eight_bit_whatever_was_asked() {
        let s = ExportSettings { format: ExportFormat::Jpeg, bit_depth: 16, jpeg_quality: 200, ..Default::default() }.normalised();
        assert_eq!((s.bit_depth, s.jpeg_quality), (8, 100));
        assert_eq!(ExportSettings { bit_depth: 12, ..Default::default() }.normalised().bit_depth, 8);
        assert_eq!(ExportSettings::default().normalised().bit_depth, 16);
    }

    #[test]
    fn settings_survive_json_and_missing_fields_take_defaults() {
        let s = ExportSettings { format: ExportFormat::Png, grey: true, colour_space: ColourSpace::AdobeRgb, ..Default::default() };
        let back: ExportSettings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        let partial: ExportSettings = serde_json::from_str(r#"{"format":"jpeg"}"#).unwrap();
        assert_eq!((partial.format, partial.bit_depth, partial.embed_profile), (ExportFormat::Jpeg, 16, true));
    }

    #[test]
    fn sizes_keep_the_proportions_and_never_enlarge() {
        assert_eq!(OutputSize::Original.dims(3000, 2000), (3000, 2000));
        assert_eq!(OutputSize::LongEdge { pixels: 1500 }.dims(3000, 2000), (1500, 1000));
        assert_eq!(OutputSize::LongEdge { pixels: 1500 }.dims(2000, 3000), (1000, 1500));
        assert_eq!(OutputSize::LongEdge { pixels: 9000 }.dims(3000, 2000), (3000, 2000), "no enlarging");
        assert_eq!(OutputSize::Fit { width: 1920, height: 1080 }.dims(3000, 2000), (1620, 1080));
        assert_eq!(OutputSize::Fit { width: 1920, height: 1080 }.dims(1000, 500), (1000, 500));
        assert_eq!(OutputSize::Percent { percent: 50.0 }.dims(3001, 2001), (1501, 1001));
        assert_eq!(OutputSize::Percent { percent: 0.0 }.dims(300, 200), (3, 2), "percent has a floor");
        assert_eq!(OutputSize::LongEdge { pixels: 0 }.dims(300, 200), (1, 1).max((1, 1)).min((300, 200)), "pixels have a floor");
        assert_eq!(OutputSize::presets().len(), 6);
    }

    #[test]
    fn sizes_survive_json_and_old_settings_mean_original() {
        for size in [OutputSize::Original, OutputSize::LongEdge { pixels: 2048 }, OutputSize::Fit { width: 10, height: 20 }, OutputSize::Percent { percent: 33.5 }] {
            let s = ExportSettings { size, ..Default::default() };
            let back: ExportSettings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
            assert_eq!(back.size, size);
        }
        assert_eq!(serde_json::from_str::<ExportSettings>(r#"{"format":"png"}"#).unwrap().size, OutputSize::Original);
    }

    #[test]
    fn presets_have_distinct_purposes() {
        let p = ExportSettings::presets();
        assert_eq!(p.len(), 4);
        assert!(p.iter().any(|(n, s)| *n == "Share" && s.format == ExportFormat::Jpeg));
        let g = p.iter().find(|(n, _)| *n == "Google Photos").map(|(_, s)| s.clone()).unwrap();
        assert!(g.google_photos && g.google_album && g.format == ExportFormat::Jpeg && g.bit_depth == 8);
        assert!(!ExportSettings::default().google_photos, "nothing is uploaded unless asked for");
        // Files written before the setting existed still load.
        let old: ExportSettings = serde_json::from_str(r#"{"format":"png"}"#).unwrap();
        assert!(!old.google_photos && old.google_album);
    }
}
