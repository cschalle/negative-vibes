/// Reverse-DNS application ID, set at build time by Meson (or defaulted by build.rs).
pub const APP_ID: &str = env!("APP_ID");

/// Installed data directory for this package (GSettings schemas, film profiles, …).
pub const PKGDATADIR: &str = env!("PKGDATADIR");
