use nv_device::DeviceCapabilities;

/// Commands sent from the UI thread to the worker.
#[derive(Debug)]
pub enum WorkerCmd {
    /// Probe for devices.  The worker will reply with `ScanEvent::DeviceList`.
    Probe,
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
    /// Shut the worker thread down.
    Quit,
}

/// Events sent from the worker thread back to the UI thread.
#[derive(Debug)]
pub enum ScanEvent {
    /// A new device list (result of Probe or reconnect attempt).
    DeviceList(Vec<DeviceCapabilities>),

    /// The selected device has been opened and options read.
    DeviceReady {
        caps: Box<DeviceCapabilities>,
    },

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
    IrFrameStarted {
        width_px: u32,
        height_px: Option<u32>,
        bytes_per_line: usize,
        depth: u8,
    },

    /// A pass of a multi-pass acquisition begins (`index` counts from 0).
    PassStarted {
        index: u32,
        total: u32,
        /// The exposure factor of this pass.
        exposure: f32,
    },

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
}
