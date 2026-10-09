use crate::capabilities::{DeviceCapabilities, IrStyle};

/// What kind of acquisition to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanKind {
    /// Single colour (or grey) scan.
    Plain,
    /// Colour + IR in one acquisition (multi-frame or RGBI).
    RgbPlusIr,
    /// N identical scans averaged (noise ∝ 1/√N).
    MultiSample { count: u8 },
    /// Two scans: normal then long exposure, merged in linear space.
    MultiExposure,
}

/// A resolved plan for one acquisition.
#[derive(Debug, Clone)]
pub struct AcquisitionPlan {
    pub kind: PlanKind,
    pub resolution_dpi: u32,
    pub depth: u8,
    /// The SANE `mode` value to set, e.g. `"Color"` or `"Color+Infrared"`.
    pub mode: String,
    /// SANE option name→value pairs to set before starting.
    pub options: Vec<(String, OptionSetting)>,
    /// Options to neutralise (set to off/0/false) — from quirks.
    pub neutralise: Vec<String>,
    /// Use the `preview` option if available.
    pub preview_mode: bool,
}

#[derive(Debug, Clone)]
pub enum OptionSetting {
    Bool(bool),
    Int(i32),
    Fixed(f64),
    String(String),
}

impl AcquisitionPlan {
    /// Build a preview plan at the device's recommended preview resolution.
    pub fn preview(caps: &DeviceCapabilities) -> Self {
        let resolution_dpi = caps.recommended_dpi.or_else(|| caps.resolutions.iter().copied().find(|&r| r >= 300)).unwrap_or(300);
        let mode = preview_mode_string(caps);
        Self {
            kind: PlanKind::Plain,
            resolution_dpi,
            depth: *caps.depths.iter().max().unwrap_or(&8),
            mode,
            options: vec![],
            neutralise: caps.neutralise.clone(),
            preview_mode: caps.has_preview_option,
        }
    }

    /// Build a full-resolution scan plan.
    pub fn scan(caps: &DeviceCapabilities, resolution_dpi: u32, with_ir: bool) -> Self {
        let mode = if with_ir && caps.ir != IrStyle::None { ir_mode_string(caps) } else { "Color".to_owned() };
        let kind = if with_ir && caps.ir != IrStyle::None { PlanKind::RgbPlusIr } else { PlanKind::Plain };
        Self { kind, resolution_dpi, depth: *caps.depths.iter().max().unwrap_or(&8), mode, options: vec![], neutralise: caps.neutralise.clone(), preview_mode: false }
    }
}

fn preview_mode_string(_caps: &DeviceCapabilities) -> String {
    "Color".to_owned()
}

fn ir_mode_string(caps: &DeviceCapabilities) -> String {
    match caps.ir {
        IrStyle::MultiFrame => "Color+Infrared".to_owned(),
        IrStyle::Rgbi => "Color+Infrared".to_owned(),
        IrStyle::SeparateScan => "Color".to_owned(),
        IrStyle::None => "Color".to_owned(),
    }
}
