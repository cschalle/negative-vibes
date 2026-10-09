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
mod resize;
pub mod settings;
mod tiff;

pub use colour::ColourSpace;
pub use encode::{ExportError, ExportReport, copy_raw, export_image, export_image_full, export_image_straightened, export_image_with, write_scan_tiff};
pub use naming::{NameContext, NameError, render_template, unique_path};
pub use settings::{Compression, ExportFormat, ExportSettings, OutputSize, Purpose};
