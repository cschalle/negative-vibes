use nv_device::DeviceCapabilities;

/// Commands sent from the UI thread to the worker.
#[derive(Debug)]
pub enum WorkerCmd {
    /// Probe for devices.  The worker will reply with `ScanEvent::DeviceList`.
    Probe,
    /// Show the scanner selection dialog, if the backend has one, then probe.
    SelectScanners { parent_window: String },
    /// Begin acquisition with the given plan.
    StartScan {
        device_name: String,
        resolution_dpi: u32,
        with_ir: bool,
        /// Sub-area to scan as fractions `[x0, y0, x1, y1]` of the device's
        /// full scan window, or `None` for the whole window.
        area: Option<[f64; 4]>,
        /// How many passes, and at which exposures (see [`crate::passes`]).
        passes: crate::passes::PassPlan,
    },
    /// Ask the current scan to stop cleanly.
    Cancel,
    /// Start or stop watching the scanner's front-panel buttons.
    ///
    /// `Some(name)` holds that device open and polls its sensor options;
    /// `None` stops and lets the scanner go. Watching has to be given up
    /// around any other operation, because a SANE backend allows one open
    /// handle at a time -- the worker does that itself.
    WatchButtons { device_name: Option<String> },
    /// Shut the worker thread down.
    Quit,
}

/// A front-panel button the scanner reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanButton {
    /// The standard SANE "scan" sensor: the panel's scan button. On the
    /// OpticFilm 8200i this is the one marked QuickScan.
    Scan,
    /// The OpticFilm 8200i's second button, marked IntelliScan.
    IntelliScan,
}

impl ScanButton {
    /// The button a SANE sensor option name refers to, if we act on it.
    pub fn from_option_name(name: &str) -> Option<Self> {
        match name {
            "scan" => Some(Self::Scan),
            "intelliscan" => Some(Self::IntelliScan),
            _ => None,
        }
    }
}

/// Events sent from the worker thread back to the UI thread.
#[derive(Debug)]
pub enum ScanEvent {
    /// A new device list (result of Probe or reconnect attempt).
    DeviceList(Vec<DeviceCapabilities>),

    /// Scanners the user has allowed that could not be opened (switched off, unplugged, or
    /// resetting), by display name. Sent just before an empty `DeviceList`, and only then.
    Unreachable(Vec<String>),

    /// The selected device has been opened and options read.
    DeviceReady { caps: Box<DeviceCapabilities> },

    /// Scan parameters are finalised; height may be None.
    ScanStarted {
        width_px: u32,
        height_px: Option<u32>,
        /// Actual bytes per scanline as reported by sane_get_parameters().
        /// May be larger than width_px × channels × (depth/8) due to alignment padding.
        bytes_per_line: usize,
        depth: u8,
        channels: u8,
    },

    /// One chunk of raw scan data (interleaved RGB or grey, big-endian for 16-bit).
    DataChunk {
        /// Byte offset from the start of the full image data.
        offset: usize,
        data: Vec<u8>,
    },

    /// Progress update (0.0 – 1.0, only when the backend supports it).
    Progress(f32),

    /// The infrared frame of a multi-frame acquisition begins. Its
    /// `DataChunk`s follow (offsets restart at 0), one grey channel.
    IrFrameStarted { width_px: u32, height_px: Option<u32>, bytes_per_line: usize, depth: u8 },

    /// A pass of a multi-pass acquisition begins (`index` counts from 0).
    PassStarted {
        index: u32,
        total: u32,
        /// The exposure factor of this pass.
        exposure: f32,
    },

    /// The scanner is making its infrared pass before the colour data starts. Some backends
    /// (the plustek8200i) do this inside `start()`, which blocks and reports nothing until it
    /// is done, so there is no progress to show, only that it is happening.
    InfraredPass,

    /// The passes are being lined up and merged.
    Merging,

    /// The colour frame finished; IR data follows (multi-frame acquisition).
    ColorFrameDone,

    /// The IR frame finished.
    IrFrameDone,

    /// The full acquisition completed successfully.
    ScanDone,

    /// An error occurred.
    Error(String),

    /// The scan was cancelled by the user.
    Cancelled,

    /// A front-panel button was pressed. One event per press.
    ButtonPressed(ScanButton),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names the backend publishes, mapped to what the app does with them.
    #[test]
    fn maps_the_option_names_the_backend_publishes() {
        assert_eq!(ScanButton::from_option_name("scan"), Some(ScanButton::Scan));
        assert_eq!(ScanButton::from_option_name("intelliscan"), Some(ScanButton::IntelliScan));
    }

    /// A sensor we have no action for is not a button press. Acting on an
    /// unknown sensor would start scans from a lid switch.
    #[test]
    fn leaves_unknown_sensors_alone() {
        for name in ["email", "copy", "page-loaded", "bool-hard-select-soft-detect", ""] {
            assert_eq!(ScanButton::from_option_name(name), None, "{name} should not map to an action");
        }
    }
}
