//! Roll analysis (DESIGN.md §7.4 step 8): measure each frame's film base,
//! density range and colour axis, and fix the median over the roll, so every
//! frame is converted the same way. A frame brighter or darker than its
//! neighbours then looks brighter or darker, instead of being stretched to
//! look like them.

use crate::{Proxy, conversion, estimate_film_base_proxy, params::ProcessParams, settings::Settings};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameAnalysis {
    pub film_base: [f32; 3],
    pub d_max: [f32; 3],
    pub cast_fit: [[f32; 2]; 3],
}

/// Measure one frame. `proxy` is the whole preview or scan, whose border
/// holds the rebate; `region` is the frame's rectangle within it, if the
/// image holds more than one frame. `params` supply the profile and the
/// cast strength; its roll-wide values are ignored.
pub fn analyse_frame(proxy: &Proxy, region: Option<[f32; 4]>, params: &ProcessParams) -> Option<FrameAnalysis> {
    if proxy.is_empty() {
        return None;
    }
    let film_base = estimate_film_base_proxy(proxy);
    let mut p = params.clone();
    p.conversion.film_base = Some(film_base);
    p.conversion.region = region;
    let m = conversion::measure_negative(&proxy.pixels, proxy.width, proxy.height, &p);
    Some(FrameAnalysis { film_base, d_max: m.d_max, cast_fit: m.cast_fit })
}

fn median(mut v: Vec<f32>) -> f32 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 }
}

/// The per-component median of several frames' analyses.
pub fn roll_analysis(frames: &[FrameAnalysis]) -> Option<FrameAnalysis> {
    if frames.is_empty() {
        return None;
    }
    let m3 = |f: &dyn Fn(&FrameAnalysis) -> [f32; 3]| -> [f32; 3] {
        let mut out = [0.0; 3];
        for c in 0..3 {
            out[c] = median(frames.iter().map(|a| f(a)[c]).collect());
        }
        out
    };
    let mut cast_fit = [[0.0f32; 2]; 3];
    for c in 0..3 {
        for k in 0..2 {
            cast_fit[c][k] = median(frames.iter().map(|a| a.cast_fit[c][k]).collect());
        }
    }
    Some(FrameAnalysis { film_base: m3(&|a| a.film_base), d_max: m3(&|a| a.d_max), cast_fit })
}

impl FrameAnalysis {
    /// The roll-wide settings this analysis fixes.
    pub fn settings(&self) -> Settings {
        Settings { film_base: Some(self.film_base), d_max: Some(self.d_max), cast_fit: Some(self.cast_fit), ..Default::default() }
    }
}

/// The settings a roll analysis sets, as a mask for clearing them again.
pub fn analysis_fields() -> Settings {
    FrameAnalysis { film_base: [0.0; 3], d_max: [0.0; 3], cast_fit: [[0.0; 2]; 3] }.settings()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{default_params_for, render};
    use nv_film::builtin_profiles;

    /// A synthetic negative frame: a ramp of green density from `lo` to `hi`
    /// inside a bright rebate border, as transmission.
    fn frame(lo: f32, hi: f32) -> Proxy {
        let (w, h) = (240usize, 120usize);
        let base = [0.6f32, 0.3, 0.18];
        let mut pixels = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let border = x < 20 || x >= w - 20 || y < 10 || y >= h - 10;
                let dg = if border { 0.0 } else { lo + (hi - lo) * (x - 20) as f32 / (w - 40) as f32 };
                let d = [0.05 + 1.1 * dg, dg, 0.03 + 0.9 * dg];
                for c in 0..3 {
                    pixels.push(base[c] * 10f32.powf(-d[c]));
                }
            }
        }
        Proxy::new(w, h, 1, pixels)
    }

    #[test]
    fn analysis_finds_the_rebate_and_the_range() {
        let p = default_params_for(&builtin_profiles()[0]);
        let a = analyse_frame(&frame(0.2, 1.4), None, &p).unwrap();
        assert!((a.film_base[1] - 0.3).abs() < 0.02, "{:?}", a.film_base);
        assert!(a.d_max[1] > 1.2 && a.d_max[1] < 1.5, "{:?}", a.d_max);
    }

    #[test]
    fn the_median_ignores_one_odd_frame() {
        let p = default_params_for(&builtin_profiles()[0]);
        let a: Vec<_> = [(0.2, 1.4), (0.2, 1.45), (0.1, 2.6)].iter().map(|r| analyse_frame(&frame(r.0, r.1), None, &p).unwrap()).collect();
        let roll = roll_analysis(&a).unwrap();
        assert!(roll.d_max[1] < 1.6, "outlier pulled the roll: {:?}", roll.d_max);
    }

    /// The point of roll analysis: with the roll's range fixed, a thin frame
    /// stays thin instead of being stretched to full range like a dense one.
    #[test]
    fn roll_range_keeps_a_thin_frame_thin() {
        let mut p = default_params_for(&builtin_profiles()[0]);
        let thin = frame(0.2, 0.7);
        let dense = frame(0.2, 1.6);
        let brightest = |rgb: &[u8]| -> u8 {
            let mut g: Vec<u8> = rgb.chunks(3).map(|p| p[1]).collect();
            g.sort_unstable();
            g[g.len() * 99 / 100]
        };

        p.conversion.film_base = Some(estimate_film_base_proxy(&thin));
        let own = brightest(&render(&thin, &p));

        let roll = roll_analysis(&[analyse_frame(&thin, None, &p).unwrap(), analyse_frame(&dense, None, &p).unwrap(), analyse_frame(&dense, None, &p).unwrap()]).unwrap();
        roll.settings().apply_to(&mut p);
        let fixed = brightest(&render(&thin, &p));
        assert!(own > fixed + 25, "per-frame range {own} vs roll range {fixed}");
    }

    #[test]
    fn region_limits_what_is_measured() {
        let p = default_params_for(&builtin_profiles()[0]);
        // Left half thin, right half dense: measuring only the left half
        // gives a smaller range than the whole.
        let f = frame(0.2, 1.6);
        let whole = analyse_frame(&f, None, &p).unwrap();
        let left = analyse_frame(&f, Some([0.0, 0.0, 0.5, 1.0]), &p).unwrap();
        assert!(left.d_max[1] < whole.d_max[1] - 0.2, "{} vs {}", left.d_max[1], whole.d_max[1]);
    }
}
