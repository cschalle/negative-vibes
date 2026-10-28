//! Client proxies for the draft interface.

use std::collections::HashMap;
use zbus::zvariant::{ObjectPath, OwnedFd, OwnedObjectPath, OwnedValue, Value};

pub type Vardict<'a> = HashMap<&'a str, Value<'a>>;
pub type ScannerInfo = (String, HashMap<String, OwnedValue>);

#[zbus::proxy(interface = "org.freedesktop.portal.Scanner", default_path = "/org/freedesktop/portal/desktop")]
pub trait Scanner {
    fn select_scanners(&self, parent_window: &str, options: Vardict<'_>) -> zbus::Result<OwnedObjectPath>;
    fn list_scanners(&self, options: Vardict<'_>) -> zbus::Result<Vec<ScannerInfo>>;
    fn open_scanner(&self, scanner_id: &str, options: Vardict<'_>) -> zbus::Result<OwnedObjectPath>;
    fn get_options(&self, session_handle: &ObjectPath<'_>, options: Vardict<'_>) -> zbus::Result<Vec<HashMap<String, OwnedValue>>>;
    fn get_option_values(&self, session_handle: &ObjectPath<'_>, options: Vardict<'_>) -> zbus::Result<HashMap<String, OwnedValue>>;
    fn set_options(&self, session_handle: &ObjectPath<'_>, values: Vec<(&str, Value<'_>)>, options: Vardict<'_>) -> zbus::Result<(HashMap<String, OwnedValue>, Vec<String>)>;
    fn get_frame_parameters(&self, session_handle: &ObjectPath<'_>, options: Vardict<'_>) -> zbus::Result<HashMap<String, OwnedValue>>;
    fn start_frame(&self, session_handle: &ObjectPath<'_>, options: Vardict<'_>) -> zbus::Result<(OwnedFd, u32)>;
    fn cancel(&self, session_handle: &ObjectPath<'_>, frame: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    fn frame_event(&self, session_handle: ObjectPath<'_>, state: &str, details: HashMap<&str, Value<'_>>) -> zbus::Result<()>;

    #[zbus(signal)]
    fn button_event(&self, session_handle: ObjectPath<'_>, scanner_id: &str, button: &str, details: HashMap<&str, Value<'_>>) -> zbus::Result<()>;

    #[zbus(property, name = "version")]
    fn version(&self) -> zbus::Result<u32>;
}

#[zbus::proxy(interface = "org.freedesktop.portal.Request")]
pub trait Request {
    fn close(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn response(&self, response: u32, results: HashMap<&str, Value<'_>>) -> zbus::Result<()>;
}

#[zbus::proxy(interface = "org.freedesktop.portal.Session")]
pub trait Session {
    fn close(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn closed(&self, details: HashMap<&str, Value<'_>>) -> zbus::Result<()>;
}
