use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FilmKind {
    ColourNegative,
    BwNegative,
    ColourReversal,
    BwReversal,
}

/// Open film-profile format (§7.4 of DESIGN.md).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilmProfile {
    pub schema: u32,
    pub id: String,
    pub vendor: String,
    pub name: String,
    #[serde(default)]
    pub iso: u32,
    pub kind: FilmKind,
    /// Estimated film-base transmission [R, G, B] in [0,1], normalised so
    /// open-scanner-light = 1.0.  Used as the auto film-base seed.
    pub base_rgb: [f32; 3],
    /// Per-channel density gamma exponents for dye-gamma alignment.
    /// Default [1.0, 1.0, 1.0] = no alignment.
    pub gamma: [f32; 3],
    /// Expected D_max per channel (density above base).
    pub density_max: [f32; 3],
    /// 3×3 dye crosstalk correction matrix (row = output channel, col = input).
    /// Default = identity.
    pub crosstalk: [[f32; 3]; 3],
    /// Print-curve toe and shoulder fractions.
    pub curve_toe: f32,
    pub curve_shoulder: f32,
    /// "user" | "builtin"
    pub source: String,
}

impl FilmProfile {
    pub fn identity() -> Self {
        Self {
            schema: 1,
            id: "generic-colour-negative".into(),
            vendor: "Generic".into(),
            name: "Colour Negative".into(),
            iso: 0,
            kind: FilmKind::ColourNegative,
            base_rgb: [0.60, 0.32, 0.18],
            gamma: [1.0, 1.0, 1.0],
            density_max: [2.0, 2.0, 2.0],
            crosstalk: [[1.0,0.0,0.0],[0.0,1.0,0.0],[0.0,0.0,1.0]],
            curve_toe: 0.12,
            curve_shoulder: 0.18,
            source: "builtin".into(),
        }
    }

    pub fn generic_bw() -> Self {
        Self {
            id: "generic-bw-negative".into(),
            vendor: "Generic".into(),
            name: "B&W Negative".into(),
            kind: FilmKind::BwNegative,
            base_rgb: [0.85, 0.85, 0.85],
            gamma: [1.0, 1.0, 1.0],
            density_max: [2.2, 2.2, 2.2],
            ..Self::identity()
        }
    }

    pub fn generic_positive() -> Self {
        Self {
            id: "generic-positive".into(),
            vendor: "Generic".into(),
            name: "Positive / Slide".into(),
            kind: FilmKind::ColourReversal,
            base_rgb: [0.05, 0.05, 0.05],
            gamma: [1.0, 1.0, 1.0],
            density_max: [0.0, 0.0, 0.0],
            ..Self::identity()
        }
    }
}
