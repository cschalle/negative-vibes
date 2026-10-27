//! Scanner access through the proposed `org.freedesktop.portal.Scanner` portal.
//!
//! [`PortalBackend`] implements `nv_sane::ScannerBackend` over D-Bus, so the acquisition
//! worker uses it the same way it uses libsane. The interface is a draft: see
//! `flatpak-scanner-portal/docs/`. Blocking: calls wait on D-Bus, so use it from the worker
//! thread, never from a UI thread.

mod backend;
pub mod proxy;

pub use backend::{PortalBackend, PortalHandle};

/// Where the real portal lives. The prototype serves the same path under its own bus name.
pub const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
pub const SCANNER_INTERFACE: &str = "org.freedesktop.portal.Scanner";
pub const REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";
pub const SESSION_INTERFACE: &str = "org.freedesktop.portal.Session";

/// Bus name the prototype owns. The real portal's `org.freedesktop.portal.Desktop` is
/// taken on a normal session, so the prototype never uses it.
pub const DEFAULT_BUS_NAME: &str = "org.freedesktop.NvPortalProto";

/// Where the portal frontend lives on a desktop that has it.
pub const PORTAL_BUS_NAME: &str = "org.freedesktop.portal.Desktop";

/// `:1.42` becomes `1_42`, as in the portal handle-path convention.
pub fn sender_path_element(unique_name: &str) -> String {
    unique_name.trim_start_matches(':').replace('.', "_")
}

/// Object path of a Request for `sender` and `token`. Clients compute it before the call
/// so they can subscribe to `Response` without a race.
pub fn request_path(unique_name: &str, token: &str) -> String {
    format!("{PORTAL_PATH}/request/{}/{token}", sender_path_element(unique_name))
}

pub fn session_path(unique_name: &str, token: &str) -> String {
    format!("{PORTAL_PATH}/session/{}/{token}", sender_path_element(unique_name))
}
