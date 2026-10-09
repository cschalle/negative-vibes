//! Application preferences: what a new roll starts with. Stored as JSON in
//! the user's config folder, so they work without an installed GSettings
//! schema (a development build has none).

use std::path::{Path, PathBuf};

use nv_export::ExportSettings;
use serde::{Deserialize, Serialize};

/// Resolution used until someone picks one.
pub const DEFAULT_DPI: u32 = 1200;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Export settings a new roll starts with.
    pub export: ExportSettings,
    /// Scan resolution a new roll starts with.
    pub scan_dpi: u32,
    /// Colour dust is painted in by Mark mode, as sRGB values (F§6).
    pub mark_colour: [f32; 3],
    /// Open the Scan a Roll guide when the app starts (for newcomers).
    pub guided_on_start: bool,
    /// Open each exported picture in the default application.
    pub open_after_export: bool,
    /// Preview as soon as the scanner is ready.
    pub auto_preview: bool,
    /// A sound when a scan is finished.
    pub completion_sound: bool,
    /// Resolution of the quick preview (the nearest the scanner offers).
    pub preview_dpi: u32,
    /// Capture infrared by default, where the scanner can.
    pub scan_ir: bool,
    /// Passes averaged by default, and the longer exposure for multi-exposure.
    pub samples: u8,
    pub long_exposure: Option<f32>,
    /// How far the infrared, and separate passes, may be out of line (pixels).
    pub ir_align_limit: u32,
    pub pass_align_limit: u32,
    /// A scanner (IT8) profile new rolls start with.
    pub scanner_profile: Option<String>,
}

impl Prefs {
    /// Values brought into the ranges the program can use.
    pub fn normalised(mut self) -> Self {
        self.samples = self.samples.clamp(1, 8);
        self.long_exposure = self.long_exposure.map(|e| e.clamp(1.5, 4.0));
        self.ir_align_limit = self.ir_align_limit.clamp(0, 64);
        self.pass_align_limit = self.pass_align_limit.clamp(0, 64);
        self.preview_dpi = self.preview_dpi.clamp(75, 1200);
        self
    }
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            export: ExportSettings::default(),
            scan_dpi: DEFAULT_DPI,
            mark_colour: [1.0, 0.0, 1.0],
            guided_on_start: true,
            open_after_export: false,
            auto_preview: false,
            completion_sound: false,
            preview_dpi: 300,
            scan_ir: true,
            samples: 1,
            long_exposure: None,
            ir_align_limit: 20,
            pass_align_limit: 20,
            scanner_profile: None,
        }
    }
}

/// `~/.config/negative-vibes/preferences.json`.
pub fn path() -> PathBuf {
    glib::user_config_dir().join("negative-vibes").join("preferences.json")
}

impl Prefs {
    /// The saved preferences; defaults when there are none or the file is unreadable.
    pub fn load() -> Self {
        Self::load_from(&path())
    }

    pub fn load_from(path: &Path) -> Self {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<Prefs>(&b).ok()).unwrap_or_default().normalised()
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&path())
    }

    /// Written to a temporary file and renamed, so a crash never leaves half a file.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nv-prefs-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn preferences_survive_a_save_and_load() {
        let dir = scratch("roundtrip");
        let file = dir.join("deep/er/preferences.json"); // folders are made as needed
        let p = Prefs {
            export: ExportSettings { template: "{date}-{frame}".into(), after_scan: false, ..Default::default() },
            scan_dpi: 3600,
            mark_colour: [0.0, 0.8, 0.2],
            guided_on_start: false,
            ..Default::default()
        };
        p.save_to(&file).unwrap();
        assert_eq!(Prefs::load_from(&file), p);
        assert!(!file.with_extension("json.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_or_damaged_files_give_the_defaults() {
        let dir = scratch("damaged");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(Prefs::load_from(&dir.join("nope.json")), Prefs::default());
        std::fs::write(dir.join("bad.json"), b"{ not json").unwrap();
        assert_eq!(Prefs::load_from(&dir.join("bad.json")), Prefs::default());
        // Missing fields keep their defaults.
        std::fs::write(dir.join("part.json"), br#"{"scan_dpi": 600}"#).unwrap();
        let p = Prefs::load_from(&dir.join("part.json"));
        assert_eq!((p.scan_dpi, p.export), (600, ExportSettings::default()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_preference_files_get_the_new_defaults_and_wild_values_are_brought_into_range() {
        let old: Prefs = serde_json::from_str(r#"{"scan_dpi": 2400, "guided_on_start": false}"#).unwrap();
        assert_eq!((old.scan_dpi, old.preview_dpi, old.scan_ir, old.samples, old.ir_align_limit), (2400, 300, true, 1, 20));
        let wild = Prefs { samples: 99, long_exposure: Some(9.0), ir_align_limit: 5000, preview_dpi: 3, ..Default::default() }.normalised();
        assert_eq!((wild.samples, wild.long_exposure, wild.ir_align_limit, wild.preview_dpi), (8, Some(4.0), 64, 75));
        assert_eq!(Prefs::default().normalised(), Prefs::default());
    }
}
