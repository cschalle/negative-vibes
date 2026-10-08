//! Getting the picture out: colour spaces and ICC profiles, naming
//! templates, export settings, and encoders that stream a frame of any size
//! to TIFF, JPEG or PNG a band at a time.
//!
//! The raw 16-bit linear TIFF that `nv-store` writes into the roll doubles as
//! the HDR RAW export (DESIGN.md §6.3): [`copy_raw`] hands it over unchanged.

pub mod colour;
pub mod dng;
pub mod encode;
pub mod icc;
pub mod metadata;
pub mod naming;
pub mod settings;
mod resize;
mod tiff;

pub use colour::ColourSpace;
pub use encode::{copy_raw, export_image, export_image_straightened, export_image_with, write_scan_tiff, ExportError, ExportReport};
pub use naming::{render_template, unique_path, NameContext, NameError};
pub use settings::{Compression, ExportFormat, ExportSettings, OutputSize};
