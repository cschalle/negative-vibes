//! What the rest of the app needs from a scanner source, independent of where the
//! scanner is driven from.
//!
//! [`Session`] and [`Handle`] implement these over libsane in this process. A scanner
//! portal client (see `flatpak-scanner-portal/docs/`) will implement them over D-Bus, so the
//! worker, capability detection and CLI need no change to use either.
//!
//! The traits mirror the SANE call sequence on purpose: the portal's
//! `GetOptions`/`SetOptions`/`StartFrame` model is SANE-shaped, so a one-to-one mapping
//! keeps the prototype honest about what the portal API can and cannot express.
//!
//! Neither trait requires `Send`: libsane objects must stay on the worker thread, and a
//! portal implementation is held to the same rule so the two behave alike.

use crate::{DeviceInfo, OptionDescriptor, OptionValue, Parameters, Result, SaneError, SetInfo};
use nv_sane_sys as sys;

/// A source of scanners: lists them and opens one.
pub trait ScannerBackend {
    /// All scanners visible to the app. `local_only` skips network scanners where the
    /// backend can tell the difference (faster, avoids mDNS timeouts).
    fn get_devices(&self, local_only: bool) -> Result<Vec<DeviceInfo>>;

    /// Let the user choose which scanners this app may use, and return what they chose.
    /// `parent_window` is a window identifier for the dialog (see the Print and USB
    /// portals; empty if there is none). With libsane there is nothing to choose, so this is
    /// just the device list; with the portal it shows the portal's dialog.
    fn select_devices(&self, _parent_window: &str) -> Result<Vec<DeviceInfo>> {
        self.get_devices(false)
    }

    /// Open a scanner by the `name` from [`DeviceInfo`]. Takes the scanner's lock until
    /// the returned handle is dropped.
    fn open(&self, name: &str) -> Result<Box<dyn ScannerHandle>>;
}

/// An open scanner: options and frame acquisition.
pub trait ScannerHandle {
    /// Every option descriptor, without the SANE slot-0 count pseudo-option.
    fn all_options(&self) -> Result<Vec<OptionDescriptor>>;

    fn get_option(&self, desc: &OptionDescriptor) -> Result<OptionValue>;

    fn set_option(&self, desc: &OptionDescriptor, value: &OptionValue) -> Result<SetInfo>;

    /// Set an option to the value the scanner picks (`SANE_ACTION_SET_AUTO`). Only for options
    /// with the automatic capability.
    fn set_option_auto(&self, _desc: &OptionDescriptor) -> Result<SetInfo> {
        Err(SaneError::Status { code: sys::SANE_STATUS_UNSUPPORTED, message: "automatic values are not supported".into() })
    }

    /// Set an option by name. `Ok(None)` if the scanner has no such option.
    fn set_option_by_name(&self, name: &str, value: &OptionValue) -> Result<Option<SetInfo>> {
        match self.all_options()?.into_iter().find(|d| d.name == name) {
            Some(desc) => self.set_option(&desc, value).map(Some),
            None => Ok(None),
        }
    }

    /// Frame parameters; final only after [`start`](Self::start).
    fn get_parameters(&self) -> Result<Parameters>;

    /// Begin one frame. Call again while the previous frame's `last_frame` was false.
    fn start(&self) -> Result<()>;

    /// Read frame data. `Ok(0)` is the end of the frame.
    fn read(&self, buf: &mut [u8]) -> Result<usize>;

    /// Cancel the frame in progress.
    fn cancel(&self);
}

impl<T: ScannerHandle + ?Sized> ScannerHandle for Box<T> {
    fn all_options(&self) -> Result<Vec<OptionDescriptor>> {
        (**self).all_options()
    }
    fn get_option(&self, desc: &OptionDescriptor) -> Result<OptionValue> {
        (**self).get_option(desc)
    }
    fn set_option(&self, desc: &OptionDescriptor, value: &OptionValue) -> Result<SetInfo> {
        (**self).set_option(desc, value)
    }
    fn set_option_auto(&self, desc: &OptionDescriptor) -> Result<SetInfo> {
        (**self).set_option_auto(desc)
    }
    fn set_option_by_name(&self, name: &str, value: &OptionValue) -> Result<Option<SetInfo>> {
        (**self).set_option_by_name(name, value)
    }
    fn get_parameters(&self) -> Result<Parameters> {
        (**self).get_parameters()
    }
    fn start(&self) -> Result<()> {
        (**self).start()
    }
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        (**self).read(buf)
    }
    fn cancel(&self) {
        (**self).cancel()
    }
}
