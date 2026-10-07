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

    pub fn add_frame(&mut self) -> &Frame {
        let idx = self.frames.len() as u32 + 1;
        self.frames.push(Frame::new(idx));
        self.frames.last().unwrap()
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
