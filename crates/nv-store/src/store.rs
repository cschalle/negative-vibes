use std::path::{Path, PathBuf};
use crate::{model::Roll, Result};

const ROLL_JSON: &str = "roll.json";
const EXPORTS_JSON: &str = "exports.json";

/// One finished export.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExportRecord {
    pub frame: u32,
    /// Full path of the file written.
    pub file: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    /// ISO 8601, UTC.
    pub at: String,
    pub settings: nv_export::ExportSettings,
}

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

    /// Wrap a roll that is already in memory with the folder it lives in.
    pub fn from_parts(root: PathBuf, roll: Roll) -> Self {
        Self { root, roll }
    }

    /// Atomically write `roll.json`.
    pub fn save(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(&self.roll)?;
        let tmp = self.root.join(".roll.json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, self.root.join(ROLL_JSON))?;
        Ok(())
    }

    /// Add an entry to `exports.json`: what was exported where, so a roll
    /// remembers its output. The file is a JSON array, replaced atomically.
    pub fn record_export(&self, record: &ExportRecord) -> Result<()> {
        let path = self.root.join(EXPORTS_JSON);
        let mut all: Vec<ExportRecord> = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        all.push(record.clone());
        let tmp = self.root.join(".exports.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&all)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Everything exported so far, oldest first.
    pub fn exports(&self) -> Vec<ExportRecord> {
        std::fs::read(self.root.join(EXPORTS_JSON))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_are_remembered_in_order() {
        let dir = std::env::temp_dir().join(format!("nv-store-exports-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = RollStore::create(&dir, "r").unwrap();
        assert!(store.exports().is_empty());
        let rec = |frame, file: &str| ExportRecord {
            frame,
            file: file.into(),
            width: 10,
            height: 5,
            bytes: 99,
            at: "2026-10-08T12:00:00Z".into(),
            settings: nv_export::ExportSettings::default(),
        };
        store.record_export(&rec(1, "/out/a.tif")).unwrap();
        store.record_export(&rec(2, "/out/b.tif")).unwrap();
        let back = RollStore::open(store.root()).unwrap().exports();
        assert_eq!(back.iter().map(|r| r.file.as_str()).collect::<Vec<_>>(), ["/out/a.tif", "/out/b.tif"]);
        assert_eq!(back[1], rec(2, "/out/b.tif"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
