pub mod params;
mod conversion;
mod tone;

pub use params::{ImageType, ProcessParams};

use nv_image::FrameBuf;
use nv_film::FilmProfile;

/// Run the M3 pipeline on a raw scan buffer and return display-ready u8 sRGB pixels.
///
/// Stages run: (3 identity) → 4 Conversion → 5 Tone → (8 gamma-2.2 output).
/// Returns a flat `w × h × 3` Vec<u8> in R8G8B8 order.
pub fn process(raw: &FrameBuf, params: &ProcessParams) -> Vec<u8> {
    let w = raw.width as usize;
    let h = raw.effective_height() as usize;
    if w == 0 || h == 0 {
        return Vec::new();
    }

    // 1. Decode raw → linear f32 [0,1] pixels.
    let linear = decode_raw(raw, w, h);

    // 2. Stage 4: conversion (negative inversion, or positive pass-through).
    let converted = match params.image_type {
        ImageType::ColourNegative | ImageType::BwNegative => {
            conversion::convert_negative(&linear, w, h, params)
        }
        ImageType::Positive => {
            conversion::normalise_positive(&linear, params)
        }
    };

    // 3. Stage 5: tone adjustments.
    let toned = tone::apply_tone(&converted, &params.tone);

    // 4. Stage 8: gamma 2.2 → u8 sRGB.
    encode_output(&toned, params.image_type == ImageType::BwNegative)
}

/// Estimate film base (the unexposed rebate) from a scan buffer.
/// Samples the image borders (outer 8% on each side) and returns the
/// per-channel 98th-percentile transmission value.
pub fn estimate_film_base(raw: &FrameBuf) -> [f32; 3] {
    let w = raw.width as usize;
    let h = raw.effective_height() as usize;
    if w == 0 || h == 0 {
        return [0.6, 0.3, 0.18];
    }
    let pixels = decode_raw(raw, w, h);
    let border_w = (w / 12).max(4);
    let border_h = (h / 12).max(4);

    let mut r_vals: Vec<f32> = Vec::new();
    let mut g_vals: Vec<f32> = Vec::new();
    let mut b_vals: Vec<f32> = Vec::new();

    for y in 0..h {
        for x in 0..w {
            if x < border_w || x >= w - border_w || y < border_h || y >= h - border_h {
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

/// Decode a raw SANE buffer to linear f32 [0,1].
fn decode_raw(raw: &FrameBuf, w: usize, h: usize) -> Vec<f32> {
    let bps = raw.depth as usize / 8;
    let ch = raw.channels as usize;
    let bpr = raw.bytes_per_row;
    let src = raw.as_bytes();
    let scale = if bps == 2 { 65535.0f32 } else { 255.0f32 };

    let mut out = vec![0.0f32; w * h * 3];
    for y in 0..h {
        for x in 0..w {
            let src_off = y * bpr + x * ch * bps;
            let dst_off = (y * w + x) * 3;

            let r = nv_image::read_sample(src, src_off, bps) as f32 / scale;
            let g = if ch >= 2 { nv_image::read_sample(src, src_off + bps, bps) as f32 / scale } else { r };
            let b = if ch >= 3 { nv_image::read_sample(src, src_off + 2 * bps, bps) as f32 / scale } else { r };

            out[dst_off] = r;
            out[dst_off + 1] = g;
            out[dst_off + 2] = b;
        }
    }
    out
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
