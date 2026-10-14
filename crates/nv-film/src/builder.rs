//! Create Film Profile (DESIGN §7.4): a profile measured from a scan of the
//! unexposed film base and a step wedge exposed on the same film. The wedge
//! gives the film's response step by step; the channels' responses against
//! one another give the dye-gamma alignment, the densest step the maximum
//! density, and the shape of the green response the toe and shoulder.
//!
//! The wedge only needs to be grey and evenly stepped; its absolute densities
//! do not matter. Crosstalk between the dyes cannot be told from a grey wedge
//! and stays at identity.

use crate::library::id_for;
use crate::{FilmKind, FilmProfile, MeasuredOn};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum BuildError {
    #[error("the wedge needs at least 6 steps; this one has {0}")]
    TooFewSteps(usize),
    #[error("the film base is too dark or too light to measure")]
    BadBase,
    #[error("only {0} wedge steps are clearly denser than the base and not black; the scan or the wedge is unsuitable")]
    TooFewUsable(usize),
    #[error("the steps do not get denser one after the other; check the wedge is in the region and the right way round")]
    NotMonotonic,
    #[error("the profile needs a name")]
    NoName,
}

/// What was measured: the base's transmission and each step's, darkest last
/// or first in any order of the wedge (they are sorted by density).
#[derive(Debug, Clone)]
pub struct Measurement {
    pub base: [f32; 3],
    pub steps: Vec<[f32; 3]>,
}

#[derive(Debug, Clone)]
pub struct Info {
    pub vendor: String,
    pub name: String,
    pub iso: u32,
    pub kind: FilmKind,
    pub device: String,
    pub date: String,
}

/// What the build learned, for showing the person.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub steps_used: usize,
    /// How well each of red and blue follows green on a log-log line (1 is perfect).
    pub fit: [f32; 2],
}

/// The median of each channel over the central part of one strip.
fn centre_median(px: &[f32], w: usize, x0: usize, y0: usize, x1: usize, y1: usize) -> [f32; 3] {
    let (sx, sy) = ((x1 - x0) / 4, (y1 - y0) / 4);
    let (x0, x1, y0, y1) = (x0 + sx, (x1 - sx).max(x0 + sx + 1), y0 + sy, (y1 - sy).max(y0 + sy + 1));
    let mut out = [0f32; 3];
    for (c, o) in out.iter_mut().enumerate() {
        let mut v: Vec<f32> = (y0..y1).flat_map(|y| (x0..x1).map(move |x| px[(y * w + x) * 3 + c])).collect();
        v.sort_by(|a, b| a.total_cmp(b));
        *o = v[v.len() / 2];
    }
    out
}

/// Cut a rectangle `[x0, y0, x1, y1]` (pixels) of an RGB picture into `n`
/// equal strips along its long side and measure each one.
pub fn measure_wedge(px: &[f32], w: usize, h: usize, rect: [usize; 4], n: usize) -> Vec<[f32; 3]> {
    let [x0, y0, x1, y1] = [rect[0].min(w), rect[1].min(h), rect[2].min(w), rect[3].min(h)];
    if n == 0 || x1 <= x0 + n || y1 <= y0 + 1 && x1 - x0 < 2 || px.len() < w * h * 3 {
        return Vec::new();
    }
    let horizontal = x1 - x0 >= y1 - y0;
    (0..n)
        .filter_map(|i| {
            let r = if horizontal {
                let (a, b) = (x0 + (x1 - x0) * i / n, x0 + (x1 - x0) * (i + 1) / n);
                (a, y0, b, y1)
            } else {
                let (a, b) = (y0 + (y1 - y0) * i / n, y0 + (y1 - y0) * (i + 1) / n);
                (x0, a, x1, b)
            };
            (r.2 > r.0 && r.3 > r.1).then(|| centre_median(px, w, r.0, r.1, r.2, r.3))
        })
        .collect()
}

/// Least-squares line through `(x, y)`: slope, intercept and correlation.
fn fit(xs: &[f64], ys: &[f64]) -> (f64, f64, f64) {
    let n = xs.len() as f64;
    let (mx, my) = (xs.iter().sum::<f64>() / n, ys.iter().sum::<f64>() / n);
    let sxx: f64 = xs.iter().map(|x| (x - mx).powi(2)).sum();
    let syy: f64 = ys.iter().map(|y| (y - my).powi(2)).sum();
    let sxy: f64 = xs.iter().zip(ys).map(|(x, y)| (x - mx) * (y - my)).sum();
    if sxx < 1e-12 || syy < 1e-12 {
        return (1.0, 0.0, 0.0);
    }
    let k = sxy / sxx;
    (k, my - k * mx, sxy / (sxx * syy).sqrt())
}

/// Where a response starts to climb and where it stops: the fraction of the
/// wedge (by step) before the slope reaches half its steepest, and after it
/// falls back below it. These are the toe and shoulder.
fn toe_and_shoulder(d: &[f64]) -> (f32, f32) {
    let n = d.len();
    let slopes: Vec<f64> = d.windows(2).map(|p| p[1] - p[0]).collect();
    let peak = slopes.iter().cloned().fold(0.0, f64::max);
    if peak <= 0.0 {
        return (0.12, 0.18);
    }
    let first = slopes.iter().position(|s| *s >= 0.5 * peak).unwrap_or(0);
    let last = slopes.iter().rposition(|s| *s >= 0.5 * peak).unwrap_or(slopes.len() - 1);
    let toe = first as f64 / (n - 1) as f64;
    let shoulder = (slopes.len() - 1 - last) as f64 / (n - 1) as f64;
    // The pipeline's curve cannot use more than a half either side.
    (toe.clamp(0.02, 0.4) as f32, shoulder.clamp(0.02, 0.4) as f32)
}

pub fn build(info: &Info, m: &Measurement) -> Result<(FilmProfile, Report), BuildError> {
    if info.name.trim().is_empty() {
        return Err(BuildError::NoName);
    }
    if m.steps.len() < 6 {
        return Err(BuildError::TooFewSteps(m.steps.len()));
    }
    if m.base.iter().any(|b| !(0.01..=1.0).contains(b)) {
        return Err(BuildError::BadBase);
    }
    // Density above base for each step, then order from least to most dense.
    let mut dens: Vec<[f64; 3]> = m
        .steps
        .iter()
        .map(|t| {
            let mut d = [0f64; 3];
            for c in 0..3 {
                d[c] = (m.base[c] as f64 / (t[c] as f64).max(1e-4)).log10().max(0.0);
            }
            d
        })
        .collect();
    dens.sort_by(|a, b| a[1].total_cmp(&b[1]));
    // The ends of the wedge: clear film and black are no use for fitting.
    let usable: Vec<[f64; 3]> = dens.iter().cloned().filter(|d| d[1] > 0.05 && d.iter().all(|v| *v < 3.5)).collect();
    if usable.len() < 4 {
        return Err(BuildError::TooFewUsable(usable.len()));
    }
    // A wedge must climb: after sorting by green, red and blue climb with it.
    let climbs = |c: usize| usable.windows(2).filter(|p| p[1][c] > p[0][c] - 0.02).count() as f64 / (usable.len() - 1) as f64;
    if climbs(0) < 0.85 || climbs(2) < 0.85 {
        return Err(BuildError::NotMonotonic);
    }

    // log D_c = k · log D_g + b, so (D_c)^(1/k) follows D_g.
    let lg: Vec<f64> = usable.iter().map(|d| d[1].ln()).collect();
    let mut gamma = [1.0f32; 3];
    let mut fit_q = [0f32; 2];
    for (slot, c) in [(0usize, 0usize), (1, 2)] {
        let lc: Vec<f64> = usable.iter().map(|d| d[c].max(1e-4).ln()).collect();
        let (k, _, r) = fit(&lg, &lc);
        gamma[c] = (1.0 / k).clamp(0.5, 2.0) as f32;
        fit_q[slot] = r as f32;
    }
    let dmax = |c: usize| usable.iter().map(|d| d[c]).fold(0.0, f64::max) as f32;
    // Toe and shoulder from the whole wedge, so they include the clear end.
    let greens: Vec<f64> = dens.iter().map(|d| d[1]).collect();
    let (toe, shoulder) = toe_and_shoulder(&greens);

    let profile = FilmProfile {
        schema: 1,
        id: id_for(&info.vendor, &info.name),
        vendor: info.vendor.trim().to_owned(),
        name: info.name.trim().to_owned(),
        iso: info.iso,
        kind: info.kind.clone(),
        base_rgb: m.base,
        gamma,
        density_max: [dmax(0), dmax(1), dmax(2)],
        crosstalk: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        curve_toe: toe,
        curve_shoulder: shoulder,
        source: "user".into(),
        measured_on: Some(MeasuredOn { device: info.device.clone(), date: info.date.clone(), method: format!("unexposed base + grey step wedge, {} steps", m.steps.len()) }),
    };
    Ok((profile, Report { steps_used: usable.len(), fit: fit_q }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wedge from a known film: S-shaped green density, red and blue as powers of it.
    fn wedge(n: usize, gamma: [f32; 3], base: [f32; 3]) -> Measurement {
        let steps = (0..n)
            .map(|i| {
                let x = i as f64 / (n - 1) as f64;
                // Logistic response, 0.05 to 2.4 in density.
                let dg = 0.05 + 2.35 / (1.0 + (-(x - 0.5) * 9.0).exp());
                let mut t = [0f32; 3];
                for c in 0..3 {
                    // Pipeline raises density to gamma; invert that: D_c = dg ^ (1/gamma_c) scaled.
                    let d = dg.powf(1.0 / gamma[c] as f64);
                    t[c] = base[c] * 10f32.powf(-(d as f32));
                }
                t
            })
            .collect();
        Measurement { base, steps }
    }

    fn info() -> Info {
        Info { vendor: "Test".into(), name: "Wedge 100".into(), iso: 100, kind: FilmKind::ColourNegative, device: "Plustek OpticFilm 8200i".into(), date: "2026-10-08".into() }
    }

    #[test]
    fn a_known_film_is_recovered() {
        let base = [0.62, 0.31, 0.17];
        let m = wedge(21, [0.92, 1.0, 1.08], base);
        let (p, report) = build(&info(), &m).unwrap();
        assert_eq!(p.id, "test-wedge-100");
        assert_eq!(p.base_rgb, base);
        assert!((p.gamma[0] - 0.92).abs() < 0.03 && (p.gamma[2] - 1.08).abs() < 0.03 && p.gamma[1] == 1.0, "{:?}", p.gamma);
        assert!(report.fit[0] > 0.99 && report.fit[1] > 0.99, "{report:?}");
        assert!(p.density_max.iter().all(|d| *d > 1.8 && *d < 3.0), "{:?}", p.density_max);
        assert!((0.02..=0.4).contains(&p.curve_toe) && (0.02..=0.4).contains(&p.curve_shoulder));
        assert_eq!(p.source, "user");
        assert!(p.measured_on.as_ref().unwrap().method.contains("21 steps"));
        crate::library::validate(&p).expect("a built profile must be a valid one");
    }

    #[test]
    fn the_order_of_the_steps_does_not_matter() {
        let mut m = wedge(21, [0.95, 1.0, 1.05], [0.6, 0.3, 0.15]);
        let (a, _) = build(&info(), &m).unwrap();
        m.steps.reverse();
        let (b, _) = build(&info(), &m).unwrap();
        assert_eq!(a.gamma, b.gamma);
        assert_eq!(a.density_max, b.density_max);
    }

    #[test]
    fn a_longer_toe_gives_a_bigger_toe() {
        // Same wedge but with the lower half flattened: the response starts later.
        let mut m = wedge(21, [1.0; 3], [0.6, 0.3, 0.15]);
        let (plain, _) = build(&info(), &m).unwrap();
        for t in m.steps.iter_mut().take(8) {
            for (v, base) in t.iter_mut().zip(m.base) {
                *v = base * 0.92;
            }
        }
        let (flat, _) = build(&info(), &m).unwrap();
        assert!(flat.curve_toe > plain.curve_toe, "{} vs {}", flat.curve_toe, plain.curve_toe);
    }

    #[test]
    fn unsuitable_scans_are_explained() {
        let good = wedge(21, [1.0; 3], [0.6, 0.3, 0.15]);
        assert_eq!(build(&info(), &Measurement { steps: good.steps[..4].to_vec(), ..good.clone() }).unwrap_err(), BuildError::TooFewSteps(4));
        assert_eq!(build(&info(), &Measurement { base: [0.0, 0.3, 0.2], ..good.clone() }).unwrap_err(), BuildError::BadBase);
        // Everything is the same as the base.
        let blank = Measurement { base: good.base, steps: vec![good.base; 10] };
        assert!(matches!(build(&info(), &blank).unwrap_err(), BuildError::TooFewUsable(0)));
        // Red goes the opposite way to green.
        let mut wrong = good.clone();
        for (i, t) in wrong.steps.iter_mut().enumerate() {
            t[0] = good.base[0] * 10f32.powf(-(2.4 - 0.1 * i as f32).max(0.1));
        }
        assert_eq!(build(&info(), &wrong).unwrap_err(), BuildError::NotMonotonic);
        assert_eq!(build(&Info { name: "  ".into(), ..info() }, &good).unwrap_err(), BuildError::NoName);
    }

    #[test]
    fn a_wedge_is_cut_into_strips_and_measured_by_median() {
        // 40 × 10 picture: four vertical bands of different grey, with a dust speck in one.
        let (w, h) = (40usize, 10usize);
        let mut px = vec![0f32; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                let v = 0.2 * (x / 10 + 1) as f32;
                px[(y * w + x) * 3..(y * w + x) * 3 + 3].fill(v);
            }
        }
        px[(5 * w + 15) * 3..(5 * w + 15) * 3 + 3].fill(1.0);
        let m = measure_wedge(&px, w, h, [0, 0, 40, 10], 4);
        assert_eq!(m.len(), 4);
        for (i, s) in m.iter().enumerate() {
            assert!((s[0] - 0.2 * (i + 1) as f32).abs() < 1e-6, "strip {i}: {s:?}");
        }
        // Tall rectangles are cut top to bottom.
        let tall = measure_wedge(&px, w, h, [0, 0, 4, 10], 2);
        assert_eq!(tall.len(), 2);
        assert!(measure_wedge(&px, w, h, [0, 0, 3, 3], 8).is_empty());
    }
}
