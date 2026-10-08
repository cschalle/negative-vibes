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
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Tiff => "tif",
            ExportFormat::Jpeg => "jpg",
            ExportFormat::Png => "png",
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
    /// One grey channel instead of three (B&W film).
    pub grey: bool,
    pub embed_profile: bool,
    /// Where files go; `None` is `~/Pictures/Scans/<roll>`.
    pub folder: Option<std::path::PathBuf>,
    /// See [`crate::naming`].
    pub template: String,
    /// Export automatically after each scan.
    pub after_scan: bool,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            format: ExportFormat::Tiff,
            bit_depth: 16,
            compression: Compression::Deflate,
            jpeg_quality: 92,
            colour_space: ColourSpace::Srgb,
            grey: false,
            embed_profile: true,
            folder: None,
            template: "{roll}-{frame:02}".to_owned(),
            after_scan: true,
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

    /// The three purposes of DESIGN.md §8.5.
    pub fn presets() -> Vec<(&'static str, ExportSettings)> {
        vec![
            ("Archive", ExportSettings { format: ExportFormat::Tiff, bit_depth: 16, compression: Compression::Deflate, colour_space: ColourSpace::AdobeRgb, ..Default::default() }),
            ("Share", ExportSettings { format: ExportFormat::Jpeg, bit_depth: 8, jpeg_quality: 90, colour_space: ColourSpace::Srgb, ..Default::default() }),
            ("Print", ExportSettings { format: ExportFormat::Tiff, bit_depth: 8, compression: Compression::Lzw, colour_space: ColourSpace::AdobeRgb, ..Default::default() }),
        ]
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
    fn presets_have_distinct_purposes() {
        let p = ExportSettings::presets();
        assert_eq!(p.len(), 3);
        assert!(p.iter().any(|(n, s)| *n == "Share" && s.format == ExportFormat::Jpeg));
    }
}
