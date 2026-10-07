use std::path::{Path, PathBuf};
use crate::{model::Roll, Result};

const ROLL_JSON: &str = "roll.json";

/// Manages a roll folder on disk.
pub struct RollStore {
    root: PathBuf,
    pub roll: Roll,
}

impl RollStore {
    /// Create a new roll folder at `parent/<name>.nvroll/`.
    pub fn create(parent: &Path, name: &str) -> Result<Self> {
        let dir_name = sanitise(name);
        let root = parent.join(format!("{dir_name}.nvroll"));
        std::fs::create_dir_all(&root)?;
        std::fs::create_dir_all(root.join("raw"))?;
        std::fs::create_dir_all(root.join("thumbs"))?;

        let mut roll = Roll::new(name);
        roll.name = name.to_owned();

        let store = Self { root, roll };
        store.save()?;
        Ok(store)
    }

    /// Open an existing roll folder.
    pub fn open(root: &Path) -> Result<Self> {
        let json_path = root.join(ROLL_JSON);
        let json = std::fs::read_to_string(&json_path)?;
        let roll: Roll = serde_json::from_str(&json)?;
        Ok(Self { root: root.to_owned(), roll })
    }

    /// Atomically write `roll.json`.
    pub fn save(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(&self.roll)?;
        let tmp = self.root.join(".roll.json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, self.root.join(ROLL_JSON))?;
        Ok(())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn raw_dir(&self) -> PathBuf {
        self.root.join("raw")
    }

    pub fn thumbs_dir(&self) -> PathBuf {
        self.root.join("thumbs")
    }

    /// Path to a raw file by name.
    pub fn raw_path(&self, filename: &str) -> PathBuf {
        self.raw_dir().join(filename)
    }

    /// Path to a preview scan for a frame.
    pub fn preview_path(&self, frame_index: u32) -> PathBuf {
        self.raw_dir().join(format!("frame-{frame_index:02}.prev.tif"))
    }

    /// Path to the full raw scan for a frame.
    pub fn scan_path(&self, frame_index: u32) -> PathBuf {
        self.raw_dir().join(format!("frame-{frame_index:02}.tif"))
    }

    /// Path to a thumbnail for a frame.
    pub fn thumb_path(&self, frame_index: u32) -> PathBuf {
        self.thumbs_dir().join(format!("frame-{frame_index:02}.jpg"))
    }
}

fn sanitise(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' })
        .collect::<String>()
        .replace(' ', "_")
}
