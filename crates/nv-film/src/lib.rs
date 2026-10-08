pub mod profile;
pub mod builtin;
pub mod detect;

pub use profile::{FilmKind, FilmProfile};
pub use builtin::builtin_profiles;
pub use detect::{detect_frames, DetectOptions};
