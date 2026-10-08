pub mod model;
pub mod store;
pub mod raw_io;

pub use model::{Frame, FrameState, Geometry, RawRef, Roll};
pub use store::RollStore;
pub use raw_io::{RawReader, RawWriter};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("TIFF error: {0}")]
    Tiff(#[from] tiff::TiffError),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pnm_round_trip() {
        let _buf = nv_image::FrameBuf::new(4, Some(3), 8, 3);
        let dir = tempdir();
        let path = dir.join("test.pnm");
        // Write via RawWriter isn't tested here — just test RawReader on a known PNM.
        let data = b"P6\n4 3\n255\n\x00\xff\x00\xff\x00\x00\x00\x00\xff\xff\xff\xff\
                      \xff\x00\x00\x00\xff\x00\x00\x00\x00\xff\x00\xff\xff\
                      \xaa\xbb\xcc\xdd\xee\xff\x11\x22\x33\x44\x55\x66";
        std::fs::write(&path, &data[..]).unwrap();
        let loaded = RawReader::read(&path).unwrap();
        assert_eq!(loaded.width, 4);
        assert_eq!(loaded.effective_height(), 3);
    }

    fn tempdir() -> std::path::PathBuf {
        let p = std::env::temp_dir().join("nv-store-test");
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
