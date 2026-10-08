use nv_film::FilmProfile;

#[derive(Debug, Clone, PartialEq)]
pub enum ImageType {
    ColourNegative,
    BwNegative,
    Positive,
}

#[derive(Debug, Clone)]
pub struct ConversionParams {
    /// Override film base [R,G,B] in [0,1]. `None` → auto-estimate from borders.
    pub film_base: Option<[f32; 3]>,
    /// Exposure offset in stops (applied as `2^stops` multiplier on density scale).
    pub exposure: f32,
    /// High-percentile clip fraction for D_max (default 0.0016 = 0.16%).
    pub tolerance: f32,
    /// Apply dye-gamma alignment (CCR) from film profile.
    pub auto_ccr: bool,
    /// Colour cast removal in [0, 1]: how far red and blue are lined up with
    /// green along the picture's own neutral axis (DESIGN.md §7.4 step 5).
    pub cast_removal: f32,
}

#[derive(Debug, Clone)]
pub struct ToneParams {
    /// Output black point in [0,1]. Pixels at or below map to 0.
    pub black_point: f32,
    /// Output white point in [0,1]. Pixels at or above map to 1.
    pub white_point: f32,
    /// Midtone gamma (>1 = brighter midtones). Applied as x^(1/midtone).
    pub midtone: f32,
    /// Contrast boost in [-1, 1]. 0 = none; positive = more contrast.
    pub contrast: f32,
    /// Per-channel gain applied to the converted image before the points
    /// (white balance). The grey pipette sets it.
    pub balance: [f32; 3],
}

impl Default for ToneParams {
    fn default() -> Self {
        Self {
            black_point: 0.0,
            white_point: 1.0,
            midtone: 1.0,
            contrast: 0.0,
            balance: [1.0; 3],
        }
    }
}

/// All adjustable processing parameters for one frame.
#[derive(Debug, Clone)]
pub struct ProcessParams {
    pub image_type: ImageType,
    pub profile: FilmProfile,
    pub conversion: ConversionParams,
    pub tone: ToneParams,
}
