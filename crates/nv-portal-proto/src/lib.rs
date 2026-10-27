//! Prototype of the proposed `org.freedesktop.portal.Scanner` interface.
//!
//! The service binary (`nv-portal-proto`) is a test double for the real portal: it
//! implements the draft interface in `flatpak-scanner-portal/docs/` over libsane, with the
//! permission store and the selection dialog stubbed. The client side, which the app uses,
//! is in `nv-sane-portal`.
//!
//! The real thing belongs in xdg-desktop-portal, in C.

pub use nv_sane_portal::proxy;
pub use nv_sane_portal::{DEFAULT_BUS_NAME, PORTAL_PATH, REQUEST_INTERFACE, SCANNER_INTERFACE, SESSION_INTERFACE, request_path, sender_path_element, session_path};

pub mod convert;
pub mod curate;
pub mod sane_thread;
pub mod service;
