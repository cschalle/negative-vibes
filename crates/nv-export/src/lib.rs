//! Output encoders and metadata embedding.
//!
//! Supports TIFF (8/16-bit, ICC tag 34675, XMP tag 700, tiled Deflate),
//! JPEG (with ICC), PNG (16-bit, iCCP), and eventually DNG.
//!
//! The raw 16-bit linear TIFF written by `nv-store` doubles as the HDR RAW
//! and HDRi RAW export format (§6.3), so no separate "save raw" path exists.

// TODO (M4): implement TiffEncoder, JpegEncoder, PngEncoder.
