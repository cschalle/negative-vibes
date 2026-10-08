use serde::{Deserialize, Serialize};

/// Schema version for roll.json forward-compatibility.
const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Roll {
    #[serde(default = "schema_version")]
    pub schema: u32,

    pub id: String,
    pub name: String,
    pub created: String,           // ISO 8601

    pub device_id: Option<String>, // "vendor model backend"
    pub film_stock: Option<String>,
    pub notes: Option<String>,

    #[serde(default)]
    pub settings: RollSettings,

    #[serde(default)]
    pub frames: Vec<Frame>,
}

fn schema_version() -> u32 { SCHEMA_VERSION }

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
        copy.state = if copy.preview.is_some() { FrameState::Previewed } else { FrameState::Empty };
        self.frames.insert(pos + 1, copy);
        Some(new_index)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RollSettings {
    pub resolution_dpi: Option<u32>,
    pub with_ir: Option<bool>,
    pub image_type: Option<ImageType>,
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
    pub rating: Option<i8>,        // -1 reject, 0 unrated, 1–5 stars
    pub state: FrameState,
    pub geometry: Geometry,
    /// Preview image in the roll's `raw/` folder. Several frames found in one
    /// preview share it, each with its own crop.
    #[serde(default)]
    pub preview: Option<String>,
    pub raw: Option<RawRef>,
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
            raw: None,
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
    pub angle: f32,  // fine rotation, degrees
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
    pub scanned_at: String,  // ISO 8601
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn new_id() -> String {
    // Simple u64 hex ID — replace with UUID in a later milestone.
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{t:032x}")
}

fn now_iso() -> String {
    // chrono is not in the workspace yet; use a simple UTC approximation.
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{secs}")   // will improve to full ISO string in M4
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
    fn old_roll_json_without_preview_still_loads() {
        let mut roll = Roll::new("t");
        roll.add_frame();
        let mut v = serde_json::to_value(&roll).unwrap();
        v["frames"][0].as_object_mut().unwrap().remove("preview");
        let back: Roll = serde_json::from_value(v).unwrap();
        assert!(back.frames[0].preview.is_none());
    }
}
