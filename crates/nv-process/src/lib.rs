pub mod analysis;
pub mod params;
mod conversion;
pub mod colour;
pub mod densitometer;
pub mod input;
pub mod curves;
pub mod detail;
pub mod dust;
pub mod orient;
pub mod settings;
pub mod tiles;
pub mod histogram;
pub mod proxy;
mod tone;

pub use histogram::Histogram;
pub use analysis::{analyse_frame, analysis_fields, roll_analysis, FrameAnalysis};
pub use params::{ImageType, ProcessParams};
pub use settings::Settings;
pub use params::ToneParams;
pub use curves::{Curve, Curves};
pub use input::InputProfile;
pub use colour::{ColourParams, GreyMix, GREY_PRESETS};
pub use detail::{AacoGrid, DetailParams, NoiseLevel, Placement, Quality, UsmParams};
pub use dust::{DefectType, DustMethod, DustMode, DustParams, Stroke};
pub use orient::{DisplayOp, Orientation};
pub use proxy::{Proxy, ProxyBuilder, DEFAULT_PROXY_PIXELS};

use nv_image::FrameBuf;
use nv_film::FilmProfile;

/// Run the pipeline on a raw scan at full resolution and return display-ready
/// u8 sRGB pixels (`w × h × 3`, R8G8B8).
pub fn process(raw: &FrameBuf, params: &ProcessParams) -> Vec<u8> {
    render(&Proxy::from_raw(raw, usize::MAX), params)
}

/// A render and the histogram of what went into the tone stage.
pub struct Rendered {
    /// Display-ready sRGB, `w × h × 3`.
    pub rgb: Vec<u8>,
    /// 256-bin histogram of the converted picture's luminance, in the
    /// perceptual encoding, before black and white points: the levels
    /// widget draws it with the points on top.
    pub levels: [u32; 256],
}

/// Run the pipeline on a proxy: 4 Conversion → 5 Tone → 8 gamma-2.2 output.
pub fn render(proxy: &Proxy, params: &ProcessParams) -> Vec<u8> {
    render_detailed(proxy, params).rgb
}

/// [`render`], plus the levels histogram, from the same conversion.
pub fn render_detailed(proxy: &Proxy, params: &ProcessParams) -> Rendered {
    render_cached(proxy, params, &RenderCache::default())
}

/// What stage 4 depends on. Tone, balance and curves are not in it.
#[derive(Clone, PartialEq)]
struct ConvKey {
    proxy: u64,
    image_type: ImageType,
    profile: FilmProfile,
    conversion: params::ConversionParams,
    dust: dust::DustParams,
}

/// Remembers the last converted picture, so a change that only touches tone
/// (the common case: a slider, a curve, a pipette) skips the conversion, which
/// is most of the work. Changing the picture, film base, exposure, cast or
/// anything else stage 4 reads converts again.
#[derive(Default)]
pub struct RenderCache {
    last: std::sync::Mutex<Option<(ConvKey, std::sync::Arc<Converted>)>>,
    conversions: std::sync::atomic::AtomicUsize,
}

impl RenderCache {
    /// How many times stage 4 actually ran.
    pub fn conversions(&self) -> usize {
        self.conversions.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// [`render_detailed`] through a [`RenderCache`].
pub fn render_cached(proxy: &Proxy, params: &ProcessParams, cache: &RenderCache) -> Rendered {
    if proxy.is_empty() {
        return Rendered { rgb: Vec::new(), levels: [0; 256] };
    }
    let key = ConvKey {
        proxy: proxy.id,
        image_type: params.image_type,
        profile: params.profile.clone(),
        conversion: params.conversion.clone(),
        dust: params.dust.clone(),
    };
    let converted = {
        let mut last = cache.last.lock().unwrap_or_else(|e| e.into_inner());
        match last.as_ref() {
            Some((k, c)) if *k == key => c.clone(),
            _ => {
                cache.conversions.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let (pixels, mask) = convert_stage(proxy, params);
                let c = std::sync::Arc::new(Converted { pixels, mask });
                *last = Some((key, c.clone()));
                c
            }
        }
    };

    let mut levels = [0u32; 256];
    let b = params.tone.balance;
    for px in converted.pixels.chunks_exact(3) {
        let l = (0.2126 * px[0] * b[0] + 0.7152 * px[1] * b[1] + 0.0722 * px[2] * b[2]).clamp(0.0, 1.0);
        levels[((l.powf(1.0 / 2.2) * 255.0 + 0.5) as usize).min(255)] += 1;
    }

    let mut toned = tone::apply_tone(&converted.pixels, &params.tone);
    colour::apply(&mut toned, &params.colour);
    detail::apply(&mut toned, proxy.width, proxy.height, &params.detail, proxy.factor, Quality::Preview, Placement { origin: proxy.origin, full: proxy.full });
    // B&W is one grey channel; collapse it before painting, or the marks go grey too.
    let grey = params.image_type == ImageType::BwNegative || params.colour.grey.enabled;
    if grey {
        grey_stage(&mut toned, params);
    }
    if let Some(mask) = &converted.mask {
        paint_marks(&mut toned, mask, params.dust.mark);
    }
    Rendered { rgb: encode_output(&toned, false), levels }
}

/// The stage 4 result and, in Mark mode, what dust detection found.
struct Converted {
    pixels: Vec<f32>,
    mask: Option<Vec<u8>>,
}

/// Stage 2, then stage 4: dust removed from the transmission, then the
/// negative converted. In Mark mode the pixels are untouched and the mask
/// comes back, to be painted on the finished picture.
fn convert_stage(proxy: &Proxy, params: &ProcessParams) -> (Vec<f32>, Option<Vec<u8>>) {
    // Infrared detection needs the plane; software detection needs only the picture.
    let usable = params.dust.mode != dust::DustMode::Off
        && (params.dust.method == dust::DustMethod::Software || proxy.ir.is_some() || !params.dust.include.is_empty());
    let (repaired, mask) = if usable {
        let (px, report) = dust::apply_placed(&proxy.pixels, proxy.ir.as_deref(), proxy.width, proxy.height, &params.dust, proxy.factor, proxy.origin, proxy.full);
        match params.dust.mode {
            dust::DustMode::Correct => (Some(px), None),
            _ => (None, report.map(|r| r.mask)),
        }
    } else {
        (None, None)
    };
    let src = repaired.as_deref().unwrap_or(&proxy.pixels);
    let out = match params.image_type {
        ImageType::ColourNegative | ImageType::BwNegative => conversion::convert_negative(src, proxy.width, proxy.height, params),
        ImageType::Positive => match &params.input {
            // The scanner's own colour, from its IT8 profile, before the picture is normalised.
            Some(profile) => {
                let mut matched = src.to_vec();
                profile.apply(&mut matched);
                conversion::normalise_positive(&matched, params)
            }
            None => conversion::normalise_positive(src, params),
        },
    };
    (out, mask)
}

/// Stage 4 only: the converted positive in linear light, before tone.
/// Auto levels measure this.
pub fn convert(proxy: &Proxy, params: &ProcessParams) -> Vec<f32> {
    convert_stage(proxy, params).0
}

/// Paint what dust detection found, for Mark mode, in the chosen colour
/// (bright magenta unless told otherwise).
fn paint_marks(toned: &mut [f32], mask: &[u8], colour: [f32; 3]) {
    for (px, m) in toned.chunks_exact_mut(3).zip(mask) {
        if *m != 0 {
            px.copy_from_slice(&colour);
        }
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
    let t = tone::auto_tone(&converted, proxy.width, proxy.height, params.conversion.region.or(Some([0.0, 0.0, 1.0, 1.0])), 0.02, 0.98, params.tone.balance);
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

/// B&W negatives are one density channel: collapse to green-weighted
/// luminance, replicated to all three channels so every consumer can keep
/// treating the picture as RGB.
/// Grey by the chosen filter weights; a B&W negative with no filter chosen keeps
/// its usual green-weighted collapse.
fn grey_stage(pixels: &mut [f32], params: &ProcessParams) {
    if params.colour.grey.enabled {
        colour::to_grey(pixels, params.colour.grey.weights);
    } else {
        collapse_to_grey(pixels);
    }
}

fn collapse_to_grey(pixels: &mut [f32]) {
    for px in pixels.chunks_exact_mut(3) {
        let lum = 0.2126 * px[0] + 0.7152 * px[1] + 0.0722 * px[2];
        px.fill(lum);
    }
}

/// The picture after stage 5, as linear light in sRGB primaries, `w × h × 3`
/// in `[0, 1]`: what the exporters turn into 8- or 16-bit files in the
/// colour space the user chose. B&W negatives come out grey.
pub fn render_toned(proxy: &Proxy, params: &ProcessParams) -> Vec<f32> {
    render_toned_q(proxy, params, Quality::Full)
}

/// [`render_toned`] at a chosen quality: `Preview` leaves out the noise
/// reduction, which is too slow for the editing preview.
pub fn render_toned_q(proxy: &Proxy, params: &ProcessParams, quality: Quality) -> Vec<f32> {
    if proxy.is_empty() {
        return Vec::new();
    }
    let (converted, mask) = convert_stage(proxy, params);
    let mut toned = tone::apply_tone(&converted, &params.tone);
    colour::apply(&mut toned, &params.colour);
    detail::apply(&mut toned, proxy.width, proxy.height, &params.detail, proxy.factor, quality, Placement { origin: proxy.origin, full: proxy.full });
    if params.image_type == ImageType::BwNegative || params.colour.grey.enabled {
        grey_stage(&mut toned, params);
    }
    if let Some(mask) = &mask {
        paint_marks(&mut toned, mask, params.dust.mark);
    }
    toned
}

/// The sRGB opto-electronic transfer function: linear light to encoded value.
#[inline]
pub fn srgb_oetf(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// Linear light to 8-bit sRGB, as the canvas shows it.
pub fn encode_srgb8(pixels: &[f32]) -> Vec<u8> {
    encode_output(pixels, false)
}

pub(crate) fn encode_output(pixels: &[f32], greyscale: bool) -> Vec<u8> {
    let mut linear;
    let pixels = if greyscale {
        linear = pixels.to_vec();
        collapse_to_grey(&mut linear);
        &linear[..]
    } else {
        pixels
    };
    pixels.iter().map(|v| (srgb_oetf(*v) * 255.0 + 0.5) as u8).collect()
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
            cast_removal: 1.0,
            d_max: None,
            cast_fit: None,
            region: None,
            positive_range: None,
        },
        tone: ToneParams::default(),
        // Silver black and white film blocks infrared: no dust removal by default.
        detail: DetailParams::default(),
        colour: ColourParams::default(),
        input: None,
        dust: params::DustParams {
            mode: if image_type == ImageType::BwNegative { dust::DustMode::Off } else { dust::DustMode::Correct },
            ..Default::default()
        },
    }
}

/// Average of the `(2·radius+1)²` proxy pixels around `(fx, fy)` (fractions of
/// the image), as raw linear transmission. For the film base pipette.
pub fn sample_proxy(proxy: &Proxy, fx: f64, fy: f64, radius: usize) -> Option<[f32; 3]> {
    sample(&proxy.pixels, proxy.width, proxy.height, fx, fy, radius)
}

/// The same on the converted positive (stage 4 output, before tone), for the
/// white, grey and black pipettes. The conversion measures the whole picture,
/// so this converts all of it.
pub fn sample_converted(proxy: &Proxy, params: &ProcessParams, fx: f64, fy: f64, radius: usize) -> Option<[f32; 3]> {
    if proxy.is_empty() {
        return None;
    }
    sample(&convert(proxy, params), proxy.width, proxy.height, fx, fy, radius)
}

fn sample(px: &[f32], w: usize, h: usize, fx: f64, fy: f64, radius: usize) -> Option<[f32; 3]> {
    if w == 0 || h == 0 || px.len() < w * h * 3 {
        return None;
    }
    let cx = ((fx.clamp(0.0, 1.0) * w as f64) as usize).min(w - 1);
    let cy = ((fy.clamp(0.0, 1.0) * h as f64) as usize).min(h - 1);
    let (x0, x1) = (cx.saturating_sub(radius), (cx + radius + 1).min(w));
    let (y0, y1) = (cy.saturating_sub(radius), (cy + radius + 1).min(h));
    let mut acc = [0f32; 3];
    for y in y0..y1 {
        for x in x0..x1 {
            for c in 0..3 {
                acc[c] += px[(y * w + x) * 3 + c];
            }
        }
    }
    let n = ((x1 - x0) * (y1 - y0)) as f32;
    Some([acc[0] / n, acc[1] / n, acc[2] / n])
}

/// What the pipettes change, as a new `ToneParams`.
pub mod pipette {
    use super::ToneParams;

    fn luma(rgb: [f32; 3], balance: [f32; 3]) -> f32 {
        0.2126 * rgb[0] * balance[0] + 0.7152 * rgb[1] * balance[1] + 0.0722 * rgb[2] * balance[2]
    }

    /// Grey pipette: gains that make `sample` neutral at the same luminance.
    pub fn grey(tone: &ToneParams, sample: [f32; 3]) -> ToneParams {
        let target = luma(sample, [1.0; 3]).max(1e-4);
        let gain = |c: usize| (target / sample[c].max(1e-4)).clamp(0.25, 4.0);
        ToneParams { balance: [gain(0), gain(1), gain(2)], ..tone.clone() }
    }

    /// White pipette: `sample` becomes white. Never at or below the black point.
    pub fn white(tone: &ToneParams, sample: [f32; 3]) -> ToneParams {
        let w = luma(sample, tone.balance).clamp(tone.black_point + 0.01, 1.0);
        ToneParams { white_point: w, ..tone.clone() }
    }

    /// Black pipette: `sample` becomes black. Never at or above the white point.
    pub fn black(tone: &ToneParams, sample: [f32; 3]) -> ToneParams {
        let b = luma(sample, tone.balance).clamp(0.0, tone.white_point - 0.01);
        ToneParams { black_point: b, ..tone.clone() }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn grey_makes_the_sample_neutral_and_keeps_its_brightness() {
            let t = grey(&ToneParams::default(), [0.6, 0.4, 0.2]);
            let out = [0.6 * t.balance[0], 0.4 * t.balance[1], 0.2 * t.balance[2]];
            assert!((out[0] - out[1]).abs() < 1e-3 && (out[1] - out[2]).abs() < 1e-3, "{out:?}");
            let before = 0.2126 * 0.6 + 0.7152 * 0.4 + 0.0722 * 0.2;
            assert!((out[1] - before).abs() < 0.01, "{} vs {before}", out[1]);
        }

        #[test]
        fn white_and_black_keep_the_points_ordered() {
            let t = ToneParams { black_point: 0.3, white_point: 0.8, ..ToneParams::default() };
            assert!(white(&t, [0.1, 0.1, 0.1]).white_point > 0.3);
            assert!(black(&t, [0.95, 0.95, 0.95]).black_point < 0.8);
            let w = white(&t, [0.7, 0.7, 0.7]);
            assert!((w.white_point - 0.7).abs() < 1e-3);
        }
    }
}
