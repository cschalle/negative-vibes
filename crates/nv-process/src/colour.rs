//! Stage 6, colour (DESIGN.md §7.1; F§5.2, 5.5, 5.6 ACR, F§3 SC2G): saturation,
//! a colour-balance wheel for shadows, midtones and highlights, restoration of
//! faded colour, and colour-to-grey with the channel weights of the black and
//! white filters photographers put on a lens.
//!
//! The corrections work on the sRGB-encoded picture, where a given amount
//! looks the same in shadows and highlights, and take and give linear light.

use serde::{Deserialize, Serialize};

/// Channel weights for turning colour into grey, as a B&W filter would.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GreyMix {
    pub enabled: bool,
    /// Red, green, blue weights on linear light; they sum to one.
    pub weights: [f32; 3],
}

impl Default for GreyMix {
    fn default() -> Self {
        Self { enabled: false, weights: LUMINANCE }
    }
}

/// How bright each colour is to the eye (Rec. 709): the neutral grey.
pub const LUMINANCE: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// Named filters. A red filter lightens reds and darkens skies and foliage; a
/// green one does the reverse for foliage; blue lightens skies. These are
/// ordinary photographic choices, not anyone's proprietary table.
pub const GREY_PRESETS: [(&str, [f32; 3]); 7] = [
    ("Neutral", LUMINANCE),
    ("Red filter", [0.90, 0.10, 0.00]),
    ("Orange filter", [0.65, 0.33, 0.02]),
    ("Yellow filter", [0.45, 0.50, 0.05]),
    ("Green filter", [0.10, 0.80, 0.10]),
    ("Blue filter", [0.00, 0.15, 0.85]),
    ("Equal", [1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0]),
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColourParams {
    /// -1 (grey) to +1 (twice as colourful).
    pub saturation: f32,
    /// Adaptive colour restoration, 0–100: how much the dull colours of a
    /// faded picture are brought back, leaving vivid ones alone.
    pub acr: f32,
    /// Colour-wheel offsets for shadows, midtones and highlights: `[x, y]` on
    /// the unit disc, the angle the hue (red at 0°, yellow 60°, green 120°,
    /// cyan 180°, blue 240°, magenta 300°) and the distance the amount.
    pub wheel: [[f32; 2]; 3],
    pub grey: GreyMix,
}

impl Default for ColourParams {
    fn default() -> Self {
        Self { saturation: 0.0, acr: 0.0, wheel: [[0.0; 2]; 3], grey: GreyMix::default() }
    }
}

impl ColourParams {
    /// Nothing for stage 6 to do (grey is applied separately).
    pub fn is_neutral(&self) -> bool {
        self.saturation == 0.0 && self.acr <= 0.0 && self.wheel.iter().all(|w| *w == [0.0; 2])
    }
}

/// The strongest offset a full wheel gives, in encoded units.
const WHEEL_STRENGTH: f32 = 0.20;

/// `(shadows, midtones, highlights)` weights for encoded brightness `l`: they
/// always add up to one, with the midtones strongest in the middle.
pub fn region_weights(l: f32) -> [f32; 3] {
    let l = l.clamp(0.0, 1.0);
    let (s, h) = ((1.0 - l) * (1.0 - l), l * l);
    [s, 1.0 - s - h, h]
}

/// The colour a wheel angle points at, as a zero-sum push on red, green, blue.
pub fn hue_direction(angle: f32) -> [f32; 3] {
    let t = std::f32::consts::TAU / 3.0;
    [angle.cos(), (angle - t).cos(), (angle + t).cos()]
}

fn encode(v: f32) -> f32 {
    crate::srgb_oetf(v)
}

fn decode(e: f32) -> f32 {
    let e = e.clamp(0.0, 1.0);
    if e <= 0.040_45 { e / 12.92 } else { ((e + 0.055) / 1.055).powf(2.4) }
}

/// Apply the colour corrections to a toned picture (linear RGB, three floats
/// per pixel). Grey conversion is not done here; see [`to_grey`].
pub fn apply(toned: &mut [f32], p: &ColourParams) {
    if p.is_neutral() {
        return;
    }
    let sat = 1.0 + p.saturation.clamp(-1.0, 1.0);
    let acr = (p.acr / 100.0).clamp(0.0, 1.0);
    // Pre-compute the push each wheel makes.
    let pushes: Vec<[f32; 3]> = p
        .wheel
        .iter()
        .map(|w| {
            let amount = w[0].hypot(w[1]).min(1.0) * WHEEL_STRENGTH;
            let d = hue_direction(w[1].atan2(w[0]));
            [d[0] * amount, d[1] * amount, d[2] * amount]
        })
        .collect();
    let wheel_on = p.wheel.iter().any(|w| *w != [0.0; 2]);

    for px in toned.chunks_exact_mut(3) {
        let mut e = [encode(px[0]), encode(px[1]), encode(px[2])];
        if wheel_on {
            let l = 0.2126 * e[0] + 0.7152 * e[1] + 0.0722 * e[2];
            let w = region_weights(l);
            for c in 0..3 {
                e[c] += w[0] * pushes[0][c] + w[1] * pushes[1][c] + w[2] * pushes[2][c];
            }
        }
        let l = 0.2126 * e[0] + 0.7152 * e[1] + 0.0722 * e[2];
        let mut gain = sat;
        if acr > 0.0 {
            // How dull the pixel is: little spread between its channels. Dull
            // colours get the lift, vivid ones almost none.
            let chroma = e[0].max(e[1]).max(e[2]) - e[0].min(e[1]).min(e[2]);
            let dull = (1.0 - (chroma / 0.5).min(1.0)).powi(2);
            gain *= 1.0 + acr * 1.2 * dull;
        }
        for c in 0..3 {
            px[c] = decode(l + (e[c] - l) * gain);
        }
    }
}

/// Collapse a picture to grey with channel weights, on linear light.
pub fn to_grey(toned: &mut [f32], weights: [f32; 3]) {
    let sum = (weights[0] + weights[1] + weights[2]).max(1e-6);
    for px in toned.chunks_exact_mut(3) {
        let g = (weights[0] * px[0] + weights[1] * px[1] + weights[2] * px[2]) / sum;
        px.fill(g);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(r: f32, g: f32, b: f32) -> Vec<f32> {
        vec![r, g, b]
    }

    fn chroma(p: &[f32]) -> f32 {
        let e: Vec<f32> = p.iter().map(|v| encode(*v)).collect();
        e.iter().cloned().fold(0.0, f32::max) - e.iter().cloned().fold(1.0, f32::min)
    }

    #[test]
    fn neutral_parameters_change_nothing_and_say_so() {
        let mut p = px(0.3, 0.2, 0.1);
        let before = p.clone();
        assert!(ColourParams::default().is_neutral());
        apply(&mut p, &ColourParams::default());
        assert_eq!(p, before);
        assert!(!ColourParams { saturation: 0.1, ..Default::default() }.is_neutral());
        assert!(!ColourParams { wheel: [[0.0; 2], [0.0; 2], [0.1, 0.0]], ..Default::default() }.is_neutral());
    }

    #[test]
    fn saturation_runs_from_grey_to_more_colour_and_keeps_brightness() {
        let colour = px(0.5, 0.2, 0.1);
        let lum = |p: &[f32]| 0.2126 * encode(p[0]) + 0.7152 * encode(p[1]) + 0.0722 * encode(p[2]);
        let mut grey = colour.clone();
        apply(&mut grey, &ColourParams { saturation: -1.0, ..Default::default() });
        assert!(chroma(&grey) < 1e-4, "{grey:?}");
        assert!((lum(&grey) - lum(&colour)).abs() < 1e-3, "grey is the same brightness");
        let mut more = colour.clone();
        apply(&mut more, &ColourParams { saturation: 0.5, ..Default::default() });
        assert!(chroma(&more) > chroma(&colour) * 1.4);
        assert!((lum(&more) - lum(&colour)).abs() < 1e-3);
        // A neutral stays neutral whatever the saturation.
        let mut n = px(0.3, 0.3, 0.3);
        apply(&mut n, &ColourParams { saturation: 1.0, ..Default::default() });
        assert!((n[0] - n[1]).abs() < 1e-5 && (n[1] - n[2]).abs() < 1e-5);
    }

    #[test]
    fn wheel_directions_point_at_their_hues() {
        let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-5);
        assert!(near(hue_direction(0.0), [1.0, -0.5, -0.5])); // red
        assert!(near(hue_direction(std::f32::consts::FRAC_PI_3), [0.5, 0.5, -1.0])); // yellow
        assert!(near(hue_direction(std::f32::consts::PI), [-1.0, 0.5, 0.5])); // cyan
        for a in [0.3f32, 1.7, 4.0] {
            assert!(hue_direction(a).iter().sum::<f32>().abs() < 1e-5, "a push must not change brightness much");
        }
    }

    #[test]
    fn region_weights_always_add_up_to_one_and_follow_brightness() {
        for i in 0..=20 {
            let w = region_weights(i as f32 / 20.0);
            assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-6 && w.iter().all(|v| *v >= -1e-6));
        }
        assert!(region_weights(0.05)[0] > 0.85 && region_weights(0.95)[2] > 0.85 && region_weights(0.5)[1] > 0.45);
    }

    #[test]
    fn a_wheel_pushes_its_own_region_towards_its_hue() {
        let dark = px(0.02, 0.02, 0.02);
        let bright = px(0.8, 0.8, 0.8);
        let red_shadows = ColourParams { wheel: [[1.0, 0.0], [0.0; 2], [0.0; 2]], ..Default::default() };
        let (mut d, mut b) = (dark.clone(), bright.clone());
        apply(&mut d, &red_shadows);
        apply(&mut b, &red_shadows);
        assert!(d[0] > d[1] * 1.5 && d[0] > d[2] * 1.5, "shadows went red: {d:?}");
        assert!((b[0] - b[1]).abs() < 0.02, "highlights should hardly move: {b:?}");
        // The same wheel on the highlights does the opposite.
        let red_highlights = ColourParams { wheel: [[0.0; 2], [0.0; 2], [1.0, 0.0]], ..Default::default() };
        let (mut d, mut b) = (dark.clone(), bright.clone());
        apply(&mut d, &red_highlights);
        apply(&mut b, &red_highlights);
        assert!(b[0] > b[1] + 0.05 && (d[0] - d[1]).abs() < 0.01);
        // A bigger turn of the wheel pushes harder.
        let (mut small, mut big) = (bright.clone(), bright.clone());
        apply(&mut small, &ColourParams { wheel: [[0.0; 2], [0.0; 2], [0.3, 0.0]], ..Default::default() });
        apply(&mut big, &ColourParams { wheel: [[0.0; 2], [0.0; 2], [0.9, 0.0]], ..Default::default() });
        assert!(big[0] - big[1] > small[0] - small[1]);
    }

    #[test]
    fn restoration_lifts_dull_colour_and_leaves_vivid_colour_alone() {
        let acr = ColourParams { acr: 100.0, ..Default::default() };
        let (mut dull, mut vivid) = (px(0.30, 0.26, 0.24), px(0.9, 0.05, 0.05));
        let (dull0, vivid0) = (dull.clone(), vivid.clone());
        apply(&mut dull, &acr);
        apply(&mut vivid, &acr);
        let (gain_dull, gain_vivid) = (chroma(&dull) / chroma(&dull0), chroma(&vivid) / chroma(&vivid0));
        assert!(gain_dull > 1.5, "dull colour only gained {gain_dull}");
        assert!(gain_vivid < 1.1, "vivid colour gained {gain_vivid}");
        // Grey stays grey.
        let mut g = px(0.4, 0.4, 0.4);
        apply(&mut g, &acr);
        assert!((g[0] - g[2]).abs() < 1e-5);
    }

    #[test]
    fn grey_filters_lighten_their_own_colour() {
        let preset = |name: &str| GREY_PRESETS.iter().find(|(n, _)| *n == name).unwrap().1;
        let grey_of = |c: [f32; 3], name: &str| { let mut p = c.to_vec(); to_grey(&mut p, preset(name)); p[0] };
        let (red, green, blue) = ([0.8, 0.05, 0.05], [0.05, 0.8, 0.05], [0.05, 0.05, 0.8]);
        assert!(grey_of(red, "Red filter") > grey_of(red, "Green filter") + 0.4);
        assert!(grey_of(green, "Green filter") > grey_of(green, "Red filter") + 0.4);
        assert!(grey_of(blue, "Blue filter") > grey_of(blue, "Red filter") + 0.4);
        // A neutral is the same through every filter.
        for (name, _) in GREY_PRESETS {
            assert!((grey_of([0.4, 0.4, 0.4], name) - 0.4).abs() < 1e-5, "{name}");
        }
        for (name, w) in GREY_PRESETS {
            assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-5, "{name} weights must sum to one");
        }
        // The output has three equal channels.
        let mut p = vec![0.7, 0.2, 0.1];
        to_grey(&mut p, preset("Orange filter"));
        assert!(p[0] == p[1] && p[1] == p[2]);
    }

    #[test]
    fn output_stays_in_range_and_finite() {
        let mut p: Vec<f32> = (0..300).map(|i| (i % 11) as f32 / 10.0).collect();
        apply(&mut p, &ColourParams { saturation: 1.0, acr: 100.0, wheel: [[1.0, 0.3], [-0.7, 0.7], [0.2, -1.0]], ..Default::default() });
        assert!(p.iter().all(|v| v.is_finite() && (0.0..=1.0001).contains(v)), "out of range");
    }
}
