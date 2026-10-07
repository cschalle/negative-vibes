use std::{collections::HashMap, path::Path};
use serde::Deserialize;
use crate::capabilities::{HolderKind, IrStyle};

/// A parsed `devices.toml` file.
#[derive(Debug, Default, Deserialize)]
pub struct Quirks {
    #[serde(default, rename = "device")]
    pub entries: Vec<QuirkEntry>,
}

#[derive(Debug, Deserialize)]
pub struct QuirkEntry {
    #[serde(rename = "match")]
    pub matcher: QuirkMatcher,
    #[serde(flatten)]
    pub quirks: DeviceQuirks,
}

#[derive(Debug, Deserialize)]
pub struct QuirkMatcher {
    pub backend: Option<String>,
    pub model: Option<String>,
}

/// The actual quirk values for a matched device.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct DeviceQuirks {
    /// [width, height] of the scanner's film window in mm.
    #[serde(rename = "film-window-mm")]
    pub film_window_mm: Option<[f64; 2]>,

    pub holder: Option<HolderKind>,

    #[serde(rename = "optical-dpi")]
    pub optical_dpi: Option<Vec<u32>>,

    #[serde(rename = "recommended-dpi")]
    pub recommended_dpi: Option<u32>,

    pub ir: Option<IrStyle>,

    #[serde(rename = "ir-registered", default)]
    pub ir_registered: bool,

    #[serde(rename = "exposure-option")]
    pub exposure_option: Option<String>,

    /// Option names to disable before scanning (set to false / 0 / off).
    #[serde(default)]
    pub neutralise: HashMap<String, bool>,

    #[serde(rename = "preview-dpi")]
    pub preview_dpi: Option<u32>,

    #[serde(rename = "streaming-progress", default)]
    pub streaming_progress: bool,
}

impl Quirks {
    /// Load from a TOML file path.  Missing file is not an error.
    pub fn load(path: &Path) -> Self {
        let Ok(contents) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        toml::from_str(&contents).unwrap_or_else(|e| {
            eprintln!("nv-device: could not parse quirks file {}: {e}", path.display());
            Self::default()
        })
    }

    /// Find the first quirks entry matching `backend` and `model`.
    /// Both patterns are treated as case-insensitive substrings.
    pub fn find(&self, backend: &str, model: &str) -> Option<&DeviceQuirks> {
        self.entries.iter().find(|e| {
            let bk_ok = e
                .matcher
                .backend
                .as_deref()
                .map_or(true, |pat| backend.to_lowercase().contains(&pat.to_lowercase()));
            let mo_ok = e
                .matcher
                .model
                .as_deref()
                .map_or(true, |pat| model.to_lowercase().contains(&pat.to_lowercase()));
            bk_ok && mo_ok
        })
        .map(|e| &e.quirks)
    }
}

/// Extract the backend name from a SANE device name string.
/// e.g. `"plustek8200i:libusb:003:007"` → `"plustek8200i"`
pub fn backend_name(device_name: &str) -> &str {
    device_name.split(':').next().unwrap_or(device_name)
}
