/// Reverse-DNS application ID, set at build time by Meson (or defaulted by build.rs).
pub const APP_ID: &str = env!("APP_ID");

/// Installed data directory for this package (GSettings schemas, film profiles, …).
pub const PKGDATADIR: &str = env!("PKGDATADIR");

/// The Google OAuth client this build signs in with, from the environment when it
/// was built (`NV_GOOGLE_CLIENT_ID`, `NV_GOOGLE_CLIENT_SECRET`). Empty in a build
/// that has none; Preferences can then be given one.
pub const GOOGLE_CLIENT_ID: &str = match option_env!("NV_GOOGLE_CLIENT_ID") {
    Some(s) => s,
    None => "",
};
pub const GOOGLE_CLIENT_SECRET: &str = match option_env!("NV_GOOGLE_CLIENT_SECRET") {
    Some(s) => s,
    None => "",
};
