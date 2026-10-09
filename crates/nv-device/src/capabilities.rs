use crate::quirks::DeviceQuirks;
use nv_sane::{Handle, OptionDescriptor, OptionKind};

/// How the device delivers infrared.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IrStyle {
    /// The `Color+Infrared` mode delivers RGB frame (last_frame=false) then
    /// a GRAY IR frame — our plustek8200i behaviour.
    MultiFrame,
    /// The backend returns a single RGBI frame (unofficial SANE_FRAME_RGBI=0x10).
    Rgbi,
    /// Infrared requires a completely separate scan in "Infrared" mode.
    SeparateScan,
    /// No infrared capability.
    None,
}

/// How the film holder advances between frames.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HolderKind {
    /// One frame window; user pushes the holder between frames.
    ManualAdvance,
    /// Multi-frame preview strip (flatbeds with film guide, motorised feeders).
    MultiFrame,
}

/// Everything the app needs to know about a SANE device before scanning.
#[derive(Debug, Clone)]
pub struct DeviceCapabilities {
    /// The SANE device name string.
    pub device_name: String,
    pub vendor: String,
    pub model: String,

    /// Device can scan film at all.
    pub has_film_source: bool,
    /// Name of the `source` option value to use for film (or None if the device
    /// is film-only and has no source option).
    pub film_source_value: Option<String>,

    pub ir: IrStyle,
    /// True when IR and colour passes share geometry (no alignment needed).
    pub ir_registered: bool,

    /// Available bit depths; 16 preferred.
    pub depths: Vec<u8>,
    /// Available resolutions in DPI (optical steps from word-list, or powers of
    /// two of the maximum for range constraints).
    pub resolutions: Vec<u32>,

    pub scan_area_mm: Option<[f64; 4]>,   // [tl-x, tl-y, br-x, br-y]
    pub film_window_mm: Option<[f64; 2]>, // [width, height]

    pub has_preview_option: bool,
    /// Name of the exposure option (factor), if present.
    pub exposure_option: Option<String>,

    pub holder: HolderKind,
    pub streaming_progress: bool,
    pub recommended_dpi: Option<u32>,

    /// Options the app must neutralise before scanning (set to false / 0 / off).
    pub neutralise: Vec<String>,
}

impl DeviceCapabilities {
    /// Build from SANE option descriptors and optional quirks.
    pub fn detect(device_name: &str, vendor: &str, model: &str, handle: &Handle, quirks: Option<&DeviceQuirks>) -> nv_sane::Result<Self> {
        let opts = handle.all_options()?;

        let film_source = detect_film_source(&opts, handle);
        let has_film_source = film_source.is_some() || quirks.map_or(false, |q| q.film_window_mm.is_some());

        let depths = detect_depths(&opts);
        let resolutions = detect_resolutions(&opts);
        let scan_area_mm = detect_scan_area_mm(&opts);
        let has_preview_option = opts.iter().any(|o| o.name == "preview");
        let exposure_option = detect_exposure_option(&opts);

        // Quirks override / augment what we detect from options.
        let ir = quirks.and_then(|q| q.ir.clone()).unwrap_or_else(|| detect_ir_from_options(&opts));
        let ir_registered = quirks.map_or(false, |q| q.ir_registered);
        let holder = quirks.and_then(|q| q.holder.clone()).unwrap_or_else(|| guess_holder(&scan_area_mm));
        let streaming_progress = quirks.map_or(false, |q| q.streaming_progress);
        let recommended_dpi = quirks.and_then(|q| q.recommended_dpi);
        let neutralise = quirks.map(|q| q.neutralise.keys().cloned().collect()).unwrap_or_default();
        let film_window_mm = quirks.and_then(|q| q.film_window_mm);

        Ok(Self {
            device_name: device_name.to_owned(),
            vendor: vendor.to_owned(),
            model: model.to_owned(),
            has_film_source,
            film_source_value: film_source,
            ir,
            ir_registered,
            depths,
            resolutions,
            scan_area_mm,
            film_window_mm,
            has_preview_option,
            exposure_option,
            holder,
            streaming_progress,
            recommended_dpi,
            neutralise,
        })
    }
}

// ── Detection helpers ─────────────────────────────────────────────────────────

fn detect_film_source(opts: &[OptionDescriptor], handle: &Handle) -> Option<String> {
    let opt = opts.iter().find(|o| o.name == "source")?;
    let nv_sane::OptionConstraint::StringList(ref values) = opt.constraint else {
        return None;
    };
    let film_keywords = ["transparen", "film", "slide", "negative", "TPU", "TMA"];
    values
        .iter()
        .find(|v| film_keywords.iter().any(|kw| v.to_lowercase().contains(&kw.to_lowercase())))
        .cloned()
        // If none of the keywords match, try reading the current value.
        .or_else(|| handle.get_option(opt).ok().and_then(|v| if let nv_sane::OptionValue::String(s) = v { Some(s) } else { None }))
}

fn detect_ir_from_options(opts: &[OptionDescriptor]) -> IrStyle {
    // Mode option with Color+Infrared / Infrared values
    if let Some(mode_opt) = opts.iter().find(|o| o.name == "mode") {
        if let nv_sane::OptionConstraint::StringList(ref values) = mode_opt.constraint {
            if values.iter().any(|v| v.contains("Infrared") && v.contains("Color")) {
                return IrStyle::MultiFrame;
            }
            if values.iter().any(|v| v == "Infrared") {
                return IrStyle::SeparateScan;
            }
        }
    }
    // Bool infrared option
    if opts.iter().any(|o| o.name == "infrared" && o.kind == OptionKind::Bool) {
        return IrStyle::MultiFrame;
    }
    IrStyle::None
}

fn detect_depths(opts: &[OptionDescriptor]) -> Vec<u8> {
    let opt = opts.iter().find(|o| o.name == "depth");
    match opt.and_then(|o| match &o.constraint {
        nv_sane::OptionConstraint::WordList(v) => Some(v.clone()),
        _ => None,
    }) {
        Some(list) => list.iter().map(|&d| d as u8).collect(),
        None => vec![8],
    }
}

fn detect_resolutions(opts: &[OptionDescriptor]) -> Vec<u32> {
    let opt = opts.iter().find(|o| o.name == "resolution" || o.name == "x-resolution");
    match opt.map(|o| &o.constraint) {
        Some(nv_sane::OptionConstraint::WordList(v)) => v.iter().map(|&r| r as u32).collect(),
        Some(nv_sane::OptionConstraint::Range { max, .. }) => {
            // Generate power-of-two steps up to max.
            let mut v = Vec::new();
            let mut r = 75u32;
            while r <= *max as u32 {
                v.push(r);
                r *= 2;
            }
            v
        }
        _ => vec![],
    }
}

fn detect_scan_area_mm(opts: &[OptionDescriptor]) -> Option<[f64; 4]> {
    let by_name = |name: &str| opts.iter().find(|o| o.name == name).and_then(|o| if let nv_sane::OptionConstraint::Range { max, .. } = o.constraint { Some(max) } else { None });
    Some([0.0, 0.0, by_name("br-x")?, by_name("br-y")?])
}

fn detect_exposure_option(opts: &[OptionDescriptor]) -> Option<String> {
    opts.iter().find(|o| o.name.starts_with("exposure") || o.name.starts_with("analog-gamma") || o.name.starts_with("gain")).map(|o| o.name.clone())
}

fn guess_holder(area: &Option<[f64; 4]>) -> HolderKind {
    match area {
        Some([_, _, w, h]) if *w < 40.0 || *h < 30.0 => HolderKind::ManualAdvance,
        _ => HolderKind::MultiFrame,
    }
}
