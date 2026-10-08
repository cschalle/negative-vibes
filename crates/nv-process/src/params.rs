use nv_film::FilmProfile;
use serde::{Deserialize, Serialize};

pub use crate::curves::{Curve, Curves};
pub use crate::detail::DetailParams;
pub use crate::dust::DustParams;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImageType {
    ColourNegative,
    BwNegative,
    Positive,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Per-channel `D_max` fixed by roll analysis. `None` measures it on the
    /// picture, which makes every frame of a roll find its own range.
    pub d_max: Option<[f32; 3]>,
    /// `(offset, slope)` of red and blue against green, fixed by roll
    /// analysis. `None` fits it on the picture.
    pub cast_fit: Option<[[f32; 2]; 3]>,
    /// The part of the image that is picture, as fractions `[x0, y0, x1, y1]`
    /// (the frame's crop on a preview that holds several frames). Automatic
    /// measurements look inside it. `None` means the whole image, less its
    /// border.
    pub region: Option<[f32; 4]>,
    /// `(min, max)` per channel that a positive is normalised to, fixed from
    /// the whole picture so tiles of it agree. `None` measures the pixels it is given.
    #[serde(default)]
    pub positive_range: Option<[[f32; 2]; 3]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Master and per-channel curves, applied last, in the perceptual encoding.
    pub curves: Curves,
}

impl Default for ToneParams {
    fn default() -> Self {
        Self {
            black_point: 0.0,
            white_point: 1.0,
            midtone: 1.0,
            contrast: 0.0,
            balance: [1.0; 3],
            curves: Curves::default(),
        }
    }
}

/// All adjustable processing parameters for one frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessParams {
    pub image_type: ImageType,
    pub profile: FilmProfile,
    pub conversion: ConversionParams,
    pub tone: ToneParams,
    /// Infrared dust removal (stage 2).
    #[serde(default)]
    pub dust: DustParams,
    /// Sharpening and noise reduction (stage 7).
    #[serde(default)]
    pub detail: DetailParams,
    /// Saturation, colour wheel, faded-colour restoration, colour-to-grey (stage 6).
    #[serde(default)]
    pub colour: crate::colour::ColourParams,
}
