pub mod params;
mod conversion;
pub mod histogram;
pub mod proxy;
mod tone;

pub use histogram::Histogram;
pub use params::{ImageType, ProcessParams};
pub use params::ToneParams;
pub use proxy::{Proxy, DEFAULT_PROXY_PIXELS};

use nv_image::FrameBuf;
use nv_film::FilmProfile;

/// Run the pipeline on a raw scan at full resolution and return display-ready
/// u8 sRGB pixels (`w × h × 3`, R8G8B8).
pub fn process(raw: &FrameBuf, params: &ProcessParams) -> Vec<u8> {
    render(&Proxy::from_raw(raw, usize::MAX), params)
}

/// Run the pipeline on a proxy: 4 Conversion → 5 Tone → 8 gamma-2.2 output.
pub fn render(proxy: &Proxy, params: &ProcessParams) -> Vec<u8> {
    if proxy.is_empty() {
        return Vec::new();
    }
    let toned = tone::apply_tone(&convert(proxy, params), &params.tone);
    encode_output(&toned, params.image_type == ImageType::BwNegative)
}

/// Stage 4 only: the converted positive in linear light, before tone.
/// Auto levels measure this.
pub fn convert(proxy: &Proxy, params: &ProcessParams) -> Vec<f32> {
    match params.image_type {
        ImageType::ColourNegative | ImageType::BwNegative => {
            conversion::convert_negative(&proxy.pixels, proxy.width, proxy.height, params)
        }
        ImageType::Positive => conversion::normalise_positive(&proxy.pixels, params),
    }
}

/// Auto levels for the frame (F§5.1: thresholds 0.02 / 0.98, 10% border
/// ignored): black and white points and the midtone that sets the median to
/// middle grey. Contrast is left as it was.
pub fn auto_tone(proxy: &Proxy, params: &ProcessParams) -> ToneParams {
    if proxy.is_empty() {
        return params.tone.clone();
    }
    let neutral = ProcessParams { tone: ToneParams::default(), ..params.clone() };
    let converted = convert(proxy, &neutral);
    let t = tone::auto_tone(&converted, proxy.width, proxy.height, 0.10, 0.02, 0.98);
    ToneParams { contrast: params.tone.contrast, ..t }
}

/// Estimate film base (the unexposed rebate) from a scan buffer.
pub fn estimate_film_base(raw: &FrameBuf) -> [f32; 3] {
    estimate_film_base_proxy(&Proxy::from_raw(raw, DEFAULT_PROXY_PIXELS))
}

/// Estimate film base from a proxy: samples the outer twelfth on each side
/// and returns the per-channel 98th-percentile transmission value.
pub fn estimate_film_base_proxy(proxy: &Proxy) -> [f32; 3] {
    let (w, h) = (proxy.width, proxy.height);
    if w == 0 || h == 0 {
        return [0.6, 0.3, 0.18];
    }
    let pixels = &proxy.pixels;
    let border_w = (w / 12).max(4);
    let border_h = (h / 12).max(4);

    let mut r_vals: Vec<f32> = Vec::new();
    let mut g_vals: Vec<f32> = Vec::new();
    let mut b_vals: Vec<f32> = Vec::new();

    for y in 0..h {
        for x in 0..w {
            if x < border_w || x >= w.saturating_sub(border_w) || y < border_h || y >= h.saturating_sub(border_h) {
                let off = (y * w + x) * 3;
                r_vals.push(pixels[off]);
                g_vals.push(pixels[off + 1]);
                b_vals.push(pixels[off + 2]);
            }
        }
    }

    [percentile_98(&mut r_vals), percentile_98(&mut g_vals), percentile_98(&mut b_vals)]
}

fn percentile_98(vals: &mut Vec<f32>) -> f32 {
    if vals.is_empty() { return 1.0; }
    vals.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
    let idx = ((vals.len() as f32 * 0.98) as usize).min(vals.len() - 1);
    vals[idx].max(0.01)
}

fn encode_output(pixels: &[f32], greyscale: bool) -> Vec<u8> {
    let n = pixels.len() / 3;
    let mut out = vec![0u8; n * 3];
    for i in 0..n {
        let r = pixels[i * 3];
        let g = pixels[i * 3 + 1];
        let b = pixels[i * 3 + 2];

        if greyscale {
            // Green-weighted luminance for B&W display.
            let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            let v = gamma22(lum);
            out[i * 3] = v;
            out[i * 3 + 1] = v;
            out[i * 3 + 2] = v;
        } else {
            out[i * 3] = gamma22(r);
            out[i * 3 + 1] = gamma22(g);
            out[i * 3 + 2] = gamma22(b);
        }
    }
    out
}

#[inline]
fn gamma22(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    (v.powf(1.0 / 2.2) * 255.0 + 0.5) as u8
}

/// Build default ProcessParams for a given film profile.
pub fn default_params_for(profile: &FilmProfile) -> ProcessParams {
    use params::{ConversionParams, ToneParams};
    let image_type = match profile.kind {
        nv_film::profile::FilmKind::BwNegative => ImageType::BwNegative,
        nv_film::profile::FilmKind::ColourNegative => ImageType::ColourNegative,
        _ => ImageType::Positive,
    };
    ProcessParams {
        image_type,
        profile: profile.clone(),
        conversion: ConversionParams {
            film_base: None,
            exposure: 0.0,
            tolerance: 0.0016,
            auto_ccr: true,
        },
        tone: ToneParams::default(),
    }
}
