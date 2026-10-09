use crate::profile::{FilmKind, FilmProfile};

/// Built-in film profiles shipped with the application.
pub fn builtin_profiles() -> Vec<FilmProfile> {
    vec![FilmProfile::identity(), FilmProfile::generic_bw(), FilmProfile::generic_positive(), kodak_portra_400(), kodak_gold_200(), fuji_superia_400(), kodak_tmax_400()]
}

fn kodak_portra_400() -> FilmProfile {
    FilmProfile {
        schema: 1,
        id: "kodak-portra-400".into(),
        vendor: "Kodak".into(),
        name: "Portra 400".into(),
        iso: 400,
        kind: FilmKind::ColourNegative,
        base_rgb: [0.62, 0.31, 0.17],
        gamma: [0.92, 1.0, 1.08],
        density_max: [1.9, 2.3, 2.4],
        crosstalk: [[1.0, -0.05, 0.0], [-0.08, 1.0, -0.06], [0.0, -0.10, 1.0]],
        curve_toe: 0.15,
        curve_shoulder: 0.20,
        source: "builtin".into(),
        measured_on: None,
    }
}

fn kodak_gold_200() -> FilmProfile {
    FilmProfile {
        schema: 1,
        id: "kodak-gold-200".into(),
        vendor: "Kodak".into(),
        name: "Gold 200".into(),
        iso: 200,
        kind: FilmKind::ColourNegative,
        base_rgb: [0.64, 0.30, 0.15],
        gamma: [0.90, 1.0, 1.10],
        density_max: [1.8, 2.2, 2.3],
        crosstalk: [[1.0, -0.04, 0.0], [-0.06, 1.0, -0.05], [0.0, -0.08, 1.0]],
        curve_toe: 0.12,
        curve_shoulder: 0.18,
        source: "builtin".into(),
        measured_on: None,
    }
}

fn fuji_superia_400() -> FilmProfile {
    FilmProfile {
        schema: 1,
        id: "fuji-superia-400".into(),
        vendor: "Fujifilm".into(),
        name: "Superia 400".into(),
        iso: 400,
        kind: FilmKind::ColourNegative,
        base_rgb: [0.58, 0.33, 0.19],
        gamma: [0.95, 1.0, 1.05],
        density_max: [2.0, 2.2, 2.3],
        crosstalk: [[1.0, -0.03, 0.0], [-0.05, 1.0, -0.04], [0.0, -0.07, 1.0]],
        curve_toe: 0.13,
        curve_shoulder: 0.17,
        source: "builtin".into(),
        measured_on: None,
    }
}

fn kodak_tmax_400() -> FilmProfile {
    FilmProfile {
        schema: 1,
        id: "kodak-tmax-400".into(),
        vendor: "Kodak".into(),
        name: "T-MAX 400".into(),
        iso: 400,
        kind: FilmKind::BwNegative,
        base_rgb: [0.88, 0.88, 0.88],
        gamma: [1.0, 1.0, 1.0],
        density_max: [2.3, 2.3, 2.3],
        crosstalk: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        curve_toe: 0.10,
        curve_shoulder: 0.15,
        source: "builtin".into(),
        measured_on: None,
    }
}
