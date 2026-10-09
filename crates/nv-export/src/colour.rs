//! Output colour spaces. The pipeline's picture is linear light in sRGB
//! primaries; exporting to another space is a 3 × 3 matrix on linear values
//! followed by that space's transfer curve (DESIGN.md §7.6).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColourSpace {
    #[default]
    Srgb,
    AdobeRgb,
}

type M3 = [[f64; 3]; 3];

fn mul(a: &M3, b: &M3) -> M3 {
    let mut o = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            o[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    o
}

/// Linear sRGB to CIE XYZ, D65 (IEC 61966-2-1).
const SRGB_TO_XYZ: M3 = [[0.412_456_4, 0.357_576_1, 0.180_437_5], [0.212_672_9, 0.715_152_2, 0.072_175_0], [0.019_333_9, 0.119_192_0, 0.950_304_1]];

/// CIE XYZ, D65, to linear Adobe RGB (1998).
const XYZ_TO_ADOBE: M3 = [[2.041_369_0, -0.564_946_4, -0.344_694_4], [-0.969_266_0, 1.876_010_8, 0.041_556_0], [0.013_447_4, -0.118_389_7, 1.015_409_6]];

impl ColourSpace {
    pub fn name(self) -> &'static str {
        match self {
            ColourSpace::Srgb => "sRGB",
            ColourSpace::AdobeRgb => "Adobe RGB (1998)",
        }
    }

    /// Linear light in sRGB primaries to linear light in this space.
    pub fn matrix(self) -> [[f32; 3]; 3] {
        let m = match self {
            ColourSpace::Srgb => [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            ColourSpace::AdobeRgb => mul(&XYZ_TO_ADOBE, &SRGB_TO_XYZ),
        };
        let mut o = [[0f32; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                o[i][j] = m[i][j] as f32;
            }
        }
        o
    }

    /// Linear light to this space's encoded value, both in `[0, 1]`.
    pub fn encode(self, v: f32) -> f32 {
        let v = v.clamp(0.0, 1.0);
        match self {
            ColourSpace::Srgb => nv_process::srgb_oetf(v),
            ColourSpace::AdobeRgb => v.powf(1.0 / ADOBE_GAMMA),
        }
    }

    /// The inverse of [`encode`](Self::encode).
    pub fn decode(self, e: f32) -> f32 {
        let e = e.clamp(0.0, 1.0);
        match self {
            ColourSpace::Srgb => {
                if e <= 0.040_45 {
                    e / 12.92
                } else {
                    ((e + 0.055) / 1.055).powf(2.4)
                }
            }
            ColourSpace::AdobeRgb => e.powf(ADOBE_GAMMA),
        }
    }
}

/// Adobe RGB's transfer curve: 563/256, as the specification gives it.
pub const ADOBE_GAMMA: f32 = 2.199_218_75;

/// The curve a grey output is encoded with: gamma 2.2 (the value in ICC's
/// 8.8 fixed point is 563/256, the same as Adobe RGB).
pub const GREY_GAMMA: f32 = ADOBE_GAMMA;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_is_the_identity_matrix_and_adobe_matches_the_published_one() {
        assert_eq!(ColourSpace::Srgb.matrix()[1], [0.0, 1.0, 0.0]);
        let m = ColourSpace::AdobeRgb.matrix();
        // sRGB -> Adobe RGB (D65), Lindbloom.
        let published = [[0.7152, 0.2848, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0412, 0.9588]];
        for i in 0..3 {
            for j in 0..3 {
                assert!((m[i][j] - published[i][j]).abs() < 2e-3, "[{i}][{j}] {} vs {}", m[i][j], published[i][j]);
            }
        }
        // White stays white: every row sums to 1.
        for row in m {
            assert!((row.iter().sum::<f32>() - 1.0).abs() < 1e-4);
        }
    }

    #[test]
    fn transfer_curves_round_trip_and_keep_the_ends() {
        for space in [ColourSpace::Srgb, ColourSpace::AdobeRgb] {
            assert_eq!(space.encode(0.0), 0.0);
            assert!((space.encode(1.0) - 1.0).abs() < 1e-6);
            for v in [0.001f32, 0.01, 0.18, 0.5, 0.9] {
                assert!((space.decode(space.encode(v)) - v).abs() < 1e-4, "{space:?} {v}");
            }
        }
        // Middle grey: sRGB encodes 18% to about 0.46.
        assert!((ColourSpace::Srgb.encode(0.18) - 0.461).abs() < 0.005);
    }
}
