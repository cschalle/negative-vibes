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
    /// How and where a measured profile was made (§7.4); absent for the
    /// generic and estimated ones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measured_on: Option<MeasuredOn>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeasuredOn {
    pub device: String,
    pub date: String,
    pub method: String,
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
            crosstalk: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            curve_toe: 0.12,
            curve_shoulder: 0.18,
            source: "builtin".into(),
            measured_on: None,
        }
    }

    /// The colour negative a frame gets when no stock has been picked.
    ///
    /// This used to be [`Self::identity`]: unit gamma and an identity
    /// crosstalk matrix, so the default render -- which is what most frames
    /// get -- applied no film modelling at all, while the named stocks did.
    /// The crosstalk here is the consensus of the three consumer stocks this
    /// crate carries (Portra 400, Gold 200, Superia 400), which is what a
    /// "typical colour negative" means when the stock is unknown.
    ///
    /// Each dye absorbs a little in its neighbours' bands, and taking that
    /// back out spreads the colours apart. Measured on the tulip frame by
    /// sweeping the matrix with gamma held flat: the red tulip gains 9%
    /// chroma and the foliage moves from a\* -3.5 to -4.2, while the
    /// concrete wall's b\* barely moves, -5.14 to -5.94 (DESIGN.md §7.4).
    ///
    /// **Gamma stays at unity, deliberately.** The named stocks carry a
    /// spread and it was tempting to average theirs in too, but it does not
    /// do what the field name suggests here. Cast removal fits R and B to G
    /// with a straight line, and a straight line cannot undo a power, so what
    /// survives of a per-channel gamma is a warm or cool shift across the
    /// whole frame rather than any gain in separation: at [0.6, 1.0, 1.4] the
    /// wall's b\* runs from -5.14 to +13.89. Checked against the one external
    /// reference available, SilverFast's own conversion of the same scan of
    /// the dusty frame: the crosstalk leaves the hue error where it was
    /// (9.9 degrees to 9.8), while adding the stocks' average gamma rotated it
    /// to 13.7 -- away from the reference. A profile that moves hue needs a
    /// measurement of *that* stock, not an average of three others.
    pub fn generic_colour_negative() -> Self {
        Self { gamma: [1.0, 1.0, 1.0], crosstalk: [[1.0, -0.04, 0.0], [-0.06, 1.0, -0.05], [0.0, -0.08, 1.0]], ..Self::identity() }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The profile a frame gets when no stock has been picked has to model a
    /// film. It was [`FilmProfile::identity`] for a long time, which meant the
    /// default render -- most renders -- took the dye crosstalk correction
    /// that the named stocks get and did nothing with it.
    #[test]
    fn the_generic_colour_negative_is_a_film_and_not_a_placeholder() {
        let g = FilmProfile::generic_colour_negative();
        assert_eq!(g.kind, FilmKind::ColourNegative);
        assert_ne!(g.crosstalk, FilmProfile::identity().crosstalk, "back to an identity matrix: the default models no film");
        // Every dye takes a little out of its neighbours, never adds.
        for (r, row) in g.crosstalk.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                if r == c {
                    assert_eq!(*v, 1.0, "crosstalk[{r}][{c}] is not a unit diagonal");
                } else {
                    assert!(*v <= 0.0, "crosstalk[{r}][{c}] = {v} adds dye instead of taking it out");
                }
            }
        }
    }

    /// Gamma is a cast control here, not a separation one: cast removal fits
    /// R and B to G with a straight line and cannot undo a power, so what
    /// survives is a warm or cool shift. Against SilverFast's conversion of
    /// the same scan, the stocks' average gamma rotated our hue error from
    /// 9.8 degrees to 13.7, away from the reference.
    #[test]
    fn the_generic_colour_negative_does_not_guess_at_a_gamma() {
        assert_eq!(FilmProfile::generic_colour_negative().gamma, [1.0, 1.0, 1.0]);
    }

    /// Both are built on [`FilmProfile::identity`] and must not pick up the
    /// colour negative's dye crosstalk: silver has no dyes to correct, and a
    /// slide was never negative-converted.
    #[test]
    fn black_and_white_and_slides_carry_no_dye_crosstalk() {
        for p in [FilmProfile::generic_bw(), FilmProfile::generic_positive()] {
            assert_eq!(p.crosstalk, FilmProfile::identity().crosstalk, "{} picked up dye crosstalk", p.id);
            assert_eq!(p.gamma, [1.0, 1.0, 1.0], "{} picked up a dye gamma", p.id);
        }
    }
}
