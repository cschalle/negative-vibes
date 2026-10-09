use serde::{Deserialize, Serialize};

/// Schema version for roll.json forward-compatibility.
const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Roll {
    #[serde(default = "schema_version")]
    pub schema: u32,

    pub id: String,
    pub name: String,
    pub created: String, // ISO 8601

    pub device_id: Option<String>, // "vendor model backend"
    pub film_stock: Option<String>,
    pub notes: Option<String>,

    #[serde(default)]
    pub settings: RollSettings,

    #[serde(default)]
    pub frames: Vec<Frame>,
}

fn schema_version() -> u32 {
    SCHEMA_VERSION
}

impl Roll {
    pub fn new(name: &str) -> Self {
        Self {
            schema: SCHEMA_VERSION,
            id: new_id(),
            name: name.to_owned(),
            created: now_iso(),
            device_id: None,
            film_stock: None,
            notes: None,
            settings: RollSettings::default(),
            frames: Vec::new(),
        }
    }

    /// Append an empty frame numbered after the highest one in the roll, so
    /// numbers stay unique (and file names with them) after deletions.
    pub fn add_frame(&mut self) -> &Frame {
        let idx = self.frames.iter().map(|f| f.index).max().unwrap_or(0) + 1;
        self.frames.push(Frame::new(idx));
        self.frames.last().unwrap()
    }

    /// The frames "Scan All" scans: previewed, and not scanned yet.
    pub fn frames_to_scan(&self) -> Vec<u32> {
        self.frames.iter().filter(|f| f.preview.is_some() && f.raw.is_none()).map(|f| f.index).collect()
    }

    pub fn frame(&self, index: u32) -> Option<&Frame> {
        self.frames.iter().find(|f| f.index == index)
    }

    pub fn frame_mut(&mut self, index: u32) -> Option<&mut Frame> {
        self.frames.iter_mut().find(|f| f.index == index)
    }

    /// Remove a frame. Returns it so the caller can delete its files.
    pub fn delete_frame(&mut self, index: u32) -> Option<Frame> {
        let pos = self.frames.iter().position(|f| f.index == index)?;
        Some(self.frames.remove(pos))
    }

    /// Insert a copy of a frame right after it: same crop, orientation and
    /// preview, but no raw scan (that is what the copy is for: scanning it
    /// again with other settings). Returns the new frame's index.
    pub fn duplicate_frame(&mut self, index: u32) -> Option<u32> {
        let pos = self.frames.iter().position(|f| f.index == index)?;
        let new_index = self.frames.iter().map(|f| f.index).max().unwrap_or(0) + 1;
        let src = &self.frames[pos];
        let mut copy = Frame::new(new_index);
        copy.label = src.label.clone();
        copy.geometry = src.geometry.clone();
        copy.preview = src.preview.clone();
        copy.preview_size = src.preview_size;
        copy.overrides = src.overrides.clone();
        copy.state = if copy.preview.is_some() { FrameState::Previewed } else { FrameState::Empty };
        self.frames.insert(pos + 1, copy);
        Some(new_index)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RollSettings {
    pub resolution_dpi: Option<u32>,
    pub with_ir: Option<bool>,
    /// Passes averaged for each exposure (F§2 multi-sample), and a longer
    /// exposure merged in (multi-exposure). Absent: one pass.
    #[serde(default)]
    pub samples: Option<u8>,
    #[serde(default)]
    pub long_exposure: Option<f32>,
    pub image_type: Option<ImageType>,
    /// Processing defaults for every frame of the roll (conversion, tone,
    /// and what roll analysis fixed).
    #[serde(default)]
    pub process: nv_process::Settings,
    /// How this roll's frames are exported (format, profile, folder, naming).
    #[serde(default)]
    pub export: nv_export::ExportSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ImageType {
    ColourNegative,
    BwNegative,
    Slide,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frame {
    pub id: String,
    pub index: u32,
    pub label: Option<String>,
    pub rating: Option<i8>, // -1 reject, 0 unrated, 1–5 stars
    pub state: FrameState,
    pub geometry: Geometry,
    /// Preview image in the roll's `raw/` folder. Several frames found in one
    /// preview share it, each with its own crop.
    #[serde(default)]
    pub preview: Option<String>,
    /// Pixel size `[w, h]` of that preview: `geometry.crop` is in its pixels,
    /// so a frame can be scanned without loading the preview.
    #[serde(default)]
    pub preview_size: Option<[u32; 2]>,
    pub raw: Option<RawRef>,
    /// Processing settings this frame has set for itself; everything else
    /// follows the roll.
    #[serde(default)]
    pub overrides: nv_process::Settings,
}

impl Frame {
    pub fn new(index: u32) -> Self {
        Self {
            id: new_id(),
            index,
            label: None,
            rating: None,
            state: FrameState::Empty,
            geometry: Geometry::default(),
            preview: None,
            preview_size: None,
            raw: None,
            overrides: nv_process::Settings::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum FrameState {
    #[default]
    Empty,
    Previewed,
    Scanned,
    NeedsRescan,
    Exported,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Geometry {
    /// Crop rectangle in raw pixel space [x, y, w, h]. None = full frame.
    pub crop: Option<[u32; 4]>,
    /// Rotation in 90° steps (0, 1, 2, 3 → 0°, 90°, 180°, 270°).
    pub rotation: u8,
    pub flip_h: bool,
    pub flip_v: bool,
    pub angle: f32, // fine rotation, degrees
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawRef {
    /// Filename relative to the roll folder's `raw/` directory.
    pub filename: String,
    pub scan: ScanSettings,
    pub has_ir: bool,
    pub width_px: u32,
    pub height_px: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanSettings {
    pub resolution_dpi: u32,
    pub depth: u8,
    pub channels: u8,
    pub scanned_at: String, // ISO 8601
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn new_id() -> String {
    // Simple u64 hex ID — replace with UUID in a later milestone.
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    format!("{t:032x}")
}

fn now_iso() -> String {
    // chrono is not in the workspace yet; use a simple UTC approximation.
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    format!("{secs}") // will improve to full ISO string in M4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_numbers_stay_unique_after_a_delete() {
        let mut roll = Roll::new("t");
        for _ in 0..3 {
            roll.add_frame();
        }
        roll.delete_frame(2).unwrap();
        assert_eq!(roll.add_frame().index, 4);
        let idx: Vec<_> = roll.frames.iter().map(|f| f.index).collect();
        assert_eq!(idx, [1, 3, 4]);
    }

    #[test]
    fn duplicate_copies_geometry_not_the_scan() {
        let mut roll = Roll::new("t");
        roll.add_frame();
        roll.add_frame();
        {
            let f = roll.frame_mut(1).unwrap();
            f.geometry.crop = Some([1, 2, 3, 4]);
            f.preview = Some("frame-01.prev.tif".into());
            f.state = FrameState::Scanned;
        }
        let n = roll.duplicate_frame(1).unwrap();
        assert_eq!(n, 3);
        // Inserted right after the original.
        assert_eq!(roll.frames.iter().map(|f| f.index).collect::<Vec<_>>(), [1, 3, 2]);
        let copy = roll.frame(3).unwrap();
        assert_eq!(copy.geometry.crop, Some([1, 2, 3, 4]));
        assert_eq!(copy.preview.as_deref(), Some("frame-01.prev.tif"));
        assert!(copy.raw.is_none());
        assert_eq!(copy.state, FrameState::Previewed);
    }

    #[test]
    fn export_settings_survive_and_old_rolls_get_defaults() {
        let mut roll = Roll::new("t");
        roll.settings.export.format = nv_export::ExportFormat::Jpeg;
        roll.settings.export.template = "{label}".into();
        let back: Roll = serde_json::from_str(&serde_json::to_string(&roll).unwrap()).unwrap();
        assert_eq!(back.settings.export.format, nv_export::ExportFormat::Jpeg);
        assert_eq!(back.settings.export.template, "{label}");
        // A roll written before export settings existed.
        let old: Roll = serde_json::from_str(r#"{"schema":1,"id":"x","name":"n","created":"0","settings":{},"frames":[]}"#).unwrap();
        assert_eq!(old.settings.export, nv_export::ExportSettings::default());
    }

    #[test]
    fn scan_all_takes_previewed_frames_that_have_no_scan_yet() {
        let mut roll = Roll::new("t");
        for _ in 0..4 {
            roll.add_frame();
        }
        roll.frame_mut(1).unwrap().preview = Some("p.tif".into());
        roll.frame_mut(2).unwrap().preview = Some("p.tif".into());
        roll.frame_mut(2).unwrap().raw = Some(RawRef {
            filename: "frame-02.tif".into(),
            scan: ScanSettings { resolution_dpi: 600, depth: 16, channels: 3, scanned_at: String::new() },
            has_ir: false,
            width_px: 1,
            height_px: 1,
        });
        roll.frame_mut(4).unwrap().preview = Some("p.tif".into());
        assert_eq!(roll.frames_to_scan(), [1, 4]);
    }

    #[test]
    fn settings_survive_a_save_and_load() {
        let mut roll = Roll::new("t");
        roll.add_frame();
        roll.settings.process.exposure = Some(0.5);
        roll.settings.process.film_base = Some([0.6, 0.3, 0.2]);
        roll.frame_mut(1).unwrap().overrides.black_point = Some(0.07);
        let back: Roll = serde_json::from_str(&serde_json::to_string(&roll).unwrap()).unwrap();
        assert_eq!(back.settings.process.exposure, Some(0.5));
        assert_eq!(back.settings.process.film_base, Some([0.6, 0.3, 0.2]));
        assert_eq!(back.frames[0].overrides.black_point, Some(0.07));
    }

    #[test]
    fn rolls_from_before_settings_still_load() {
        let json = r#"{"schema":1,"id":"x","name":"n","created":"0","settings":{},
            "frames":[{"id":"f","index":1,"label":null,"rating":null,"state":"empty",
            "geometry":{"crop":null,"rotation":0,"flip_h":false,"flip_v":false,"angle":0.0},"raw":null}]}"#;
        let roll: Roll = serde_json::from_str(json).unwrap();
        assert!(roll.settings.process.is_empty() && roll.frames[0].overrides.is_empty());
    }

    #[test]
    fn old_roll_json_without_preview_still_loads() {
        let mut roll = Roll::new("t");
        roll.add_frame();
        let mut v = serde_json::to_value(&roll).unwrap();
        v["frames"][0].as_object_mut().unwrap().remove("preview");
        let back: Roll = serde_json::from_value(v).unwrap();
        assert!(back.frames[0].preview.is_none());
    }
}
