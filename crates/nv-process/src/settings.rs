//! Settings that live in `roll.json`: every field optional, so the same
//! struct serves as the roll's defaults and as a frame's overrides
//! (DESIGN.md §6.1). A frame's effective parameters are the defaults, then
//! the roll's settings, then the frame's own.

use serde::{Deserialize, Serialize};

use crate::params::{ImageType, ProcessParams, ToneParams};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub image_type: Option<ImageType>,
    /// Film profile id.
    pub profile: Option<String>,
    pub film_base: Option<[f32; 3]>,
    pub d_max: Option<[f32; 3]>,
    pub cast_fit: Option<[[f32; 2]; 3]>,
    pub exposure: Option<f32>,
    pub cast_removal: Option<f32>,
    pub black_point: Option<f32>,
    pub white_point: Option<f32>,
    pub midtone: Option<f32>,
    pub contrast: Option<f32>,
    pub balance: Option<[f32; 3]>,
    pub curves: Option<crate::curves::Curves>,
    pub dust_mode: Option<crate::dust::DustMode>,
    pub dust_detection: Option<u8>,
    pub dust_method: Option<crate::dust::DustMethod>,
    pub dust_defect: Option<crate::dust::DefectType>,
    /// Brush strokes belong to one frame's picture: they are always a frame's
    /// own, whatever scope an edit was made in.
    pub dust_exclude: Option<Vec<crate::dust::Stroke>>,
    pub dust_include: Option<Vec<crate::dust::Stroke>>,
    pub usm_enabled: Option<bool>,
    pub usm_amount: Option<f32>,
    pub usm_radius: Option<f32>,
    pub usm_threshold: Option<u8>,
    pub noise: Option<crate::detail::NoiseLevel>,
    pub local_contrast: Option<f32>,
    pub saturation: Option<f32>,
    pub acr: Option<f32>,
    pub wheel: Option<[[f32; 2]; 3]>,
    pub selective: Option<[[f32; 3]; 6]>,
    pub grey_enabled: Option<bool>,
    pub grey_weights: Option<[f32; 3]>,
    pub input: Option<crate::input::InputProfile>,
}

macro_rules! each_field {
    ($m:ident) => {
        $m!(image_type, profile, film_base, d_max, cast_fit, exposure, cast_removal, black_point, white_point, midtone, contrast, balance, curves, dust_mode, dust_detection, dust_method, dust_defect, dust_exclude, dust_include, usm_enabled, usm_amount, usm_radius, usm_threshold, noise, local_contrast, saturation, acr, wheel, selective, grey_enabled, grey_weights, input)
    };
}

impl Settings {
    pub fn is_empty(&self) -> bool {
        *self == Settings::default()
    }

    /// Put these settings on top of `p`.
    pub fn apply_to(&self, p: &mut ProcessParams) {
        if let Some(v) = self.image_type { p.image_type = v; }
        if let Some(id) = &self.profile {
            if let Some(profile) = nv_film::find_profile(id) {
                p.profile = profile;
            }
        }
        if let Some(v) = self.film_base { p.conversion.film_base = Some(v); }
        if let Some(v) = self.d_max { p.conversion.d_max = Some(v); }
        if let Some(v) = self.cast_fit { p.conversion.cast_fit = Some(v); }
        if let Some(v) = self.exposure { p.conversion.exposure = v; }
        if let Some(v) = self.cast_removal { p.conversion.cast_removal = v; }
        if let Some(v) = self.black_point { p.tone.black_point = v; }
        if let Some(v) = self.white_point { p.tone.white_point = v; }
        if let Some(v) = self.midtone { p.tone.midtone = v; }
        if let Some(v) = self.contrast { p.tone.contrast = v; }
        if let Some(v) = self.balance { p.tone.balance = v; }
        if let Some(v) = &self.curves { p.tone.curves = v.clone(); }
        if let Some(v) = self.dust_mode { p.dust.mode = v; }
        if let Some(v) = self.dust_detection { p.dust.detection = v; }
        if let Some(v) = self.dust_method { p.dust.method = v; }
        if let Some(v) = self.dust_defect { p.dust.defect = v; }
        if let Some(v) = &self.dust_exclude { p.dust.exclude = v.clone(); }
        if let Some(v) = &self.dust_include { p.dust.include = v.clone(); }
        if let Some(v) = self.usm_enabled { p.detail.usm.enabled = v; }
        if let Some(v) = self.usm_amount { p.detail.usm.amount = v; }
        if let Some(v) = self.usm_radius { p.detail.usm.radius = v; }
        if let Some(v) = self.usm_threshold { p.detail.usm.threshold = v; }
        if let Some(v) = self.noise { p.detail.noise = v; }
        if let Some(v) = self.local_contrast { p.detail.local_contrast = v; }
        if let Some(v) = self.saturation { p.colour.saturation = v; }
        if let Some(v) = self.acr { p.colour.acr = v; }
        if let Some(v) = self.wheel { p.colour.wheel = v; }
        if let Some(v) = self.selective { p.colour.selective = v; }
        if let Some(v) = self.grey_enabled { p.colour.grey.enabled = v; }
        if let Some(v) = self.grey_weights { p.colour.grey.weights = v; }
        if let Some(v) = &self.input { p.input = Some(v.clone()); }
    }

    /// The fields that differ between `old` and `new`.
    pub fn diff(old: &ProcessParams, new: &ProcessParams) -> Settings {
        fn ch<T: PartialEq + Copy>(a: T, b: T) -> Option<T> { (a != b).then_some(b) }
        fn chf(a: f32, b: f32) -> Option<f32> { ((a - b).abs() > 1e-6).then_some(b) }
        fn ch3(a: [f32; 3], b: [f32; 3]) -> Option<[f32; 3]> {
            // Spin rows show three decimals; anything finer is rounding.
            a.iter().zip(&b).any(|(x, y)| (x - y).abs() > 5e-4).then_some(b)
        }
        Settings {
            image_type: ch(old.image_type, new.image_type),
            profile: (old.profile.id != new.profile.id).then(|| new.profile.id.clone()),
            film_base: match (old.conversion.film_base, new.conversion.film_base) {
                (Some(a), Some(b)) => ch3(a, b),
                (None, Some(b)) => Some(b),
                _ => None,
            },
            d_max: match (old.conversion.d_max, new.conversion.d_max) {
                (Some(a), Some(b)) => ch3(a, b),
                (None, Some(b)) => Some(b),
                _ => None,
            },
            cast_fit: (old.conversion.cast_fit != new.conversion.cast_fit).then_some(new.conversion.cast_fit).flatten(),
            exposure: chf(old.conversion.exposure, new.conversion.exposure),
            cast_removal: chf(old.conversion.cast_removal, new.conversion.cast_removal),
            black_point: chf(old.tone.black_point, new.tone.black_point),
            white_point: chf(old.tone.white_point, new.tone.white_point),
            midtone: chf(old.tone.midtone, new.tone.midtone),
            contrast: chf(old.tone.contrast, new.tone.contrast),
            balance: ch3(old.tone.balance, new.tone.balance),
            curves: (old.tone.curves != new.tone.curves).then(|| new.tone.curves.clone()),
            dust_mode: ch(old.dust.mode, new.dust.mode),
            dust_detection: ch(old.dust.detection, new.dust.detection),
            dust_method: ch(old.dust.method, new.dust.method),
            dust_defect: ch(old.dust.defect, new.dust.defect),
            dust_exclude: (old.dust.exclude != new.dust.exclude).then(|| new.dust.exclude.clone()),
            dust_include: (old.dust.include != new.dust.include).then(|| new.dust.include.clone()),
            usm_enabled: ch(old.detail.usm.enabled, new.detail.usm.enabled),
            usm_amount: chf(old.detail.usm.amount, new.detail.usm.amount),
            usm_radius: chf(old.detail.usm.radius, new.detail.usm.radius),
            usm_threshold: ch(old.detail.usm.threshold, new.detail.usm.threshold),
            noise: ch(old.detail.noise, new.detail.noise),
            local_contrast: chf(old.detail.local_contrast, new.detail.local_contrast),
            saturation: chf(old.colour.saturation, new.colour.saturation),
            acr: chf(old.colour.acr, new.colour.acr),
            wheel: ch(old.colour.wheel, new.colour.wheel),
            selective: ch(old.colour.selective, new.colour.selective),
            grey_enabled: ch(old.colour.grey.enabled, new.colour.grey.enabled),
            grey_weights: ch3(old.colour.grey.weights, new.colour.grey.weights),
            input: (old.input != new.input).then(|| new.input.clone()).flatten(),
        }
    }

    /// Split the brush strokes off: a roll's defaults never hold them.
    pub fn take_strokes(&mut self) -> Settings {
        Settings { dust_exclude: self.dust_exclude.take(), dust_include: self.dust_include.take(), ..Default::default() }
    }

    /// Take every field `other` sets.
    pub fn merge(&mut self, other: &Settings) {
        macro_rules! m { ($($f:ident),*) => { $( if other.$f.is_some() { self.$f = other.$f.clone(); } )* }; }
        each_field!(m);
    }

    /// Forget every field `other` sets.
    pub fn clear(&mut self, other: &Settings) {
        macro_rules! m { ($($f:ident),*) => { $( if other.$f.is_some() { self.$f = None; } )* }; }
        each_field!(m);
    }
}

/// The three looks the guided flow offers after the automatic one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    Lighter,
    Neutral,
    Warmer,
}

impl Look {
    pub const ALL: [Look; 3] = [Look::Lighter, Look::Neutral, Look::Warmer];

    pub fn name(self) -> &'static str {
        match self {
            Look::Lighter => "Lighter",
            Look::Neutral => "Neutral",
            Look::Warmer => "Warmer",
        }
    }

    /// What this look changes, on top of the automatic conversion.
    pub fn settings(self) -> Settings {
        match self {
            Look::Lighter => Settings { exposure: Some(0.4), ..Default::default() },
            Look::Neutral => Settings { exposure: Some(0.0), balance: Some([1.0, 1.0, 1.0]), ..Default::default() },
            Look::Warmer => Settings { balance: Some([1.06, 1.0, 0.93]), ..Default::default() },
        }
    }
}

/// Default tone, for resets.
pub fn default_tone() -> ToneParams {
    ToneParams::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default_params_for;
    use nv_film::builtin_profiles;

    fn base() -> ProcessParams {
        default_params_for(&builtin_profiles()[0])
    }

    #[test]
    fn diff_then_apply_reproduces_the_edit() {
        let old = base();
        let mut new = old.clone();
        new.conversion.exposure = 0.7;
        new.tone.black_point = 0.1;
        new.tone.balance = [1.1, 1.0, 0.9];
        new.profile = builtin_profiles()[3].clone();
        let d = Settings::diff(&old, &new);
        assert_eq!(d.exposure, Some(0.7));
        assert_eq!(d.black_point, Some(0.1));
        assert!(d.white_point.is_none() && d.midtone.is_none() && d.image_type.is_none());
        let mut back = old.clone();
        d.apply_to(&mut back);
        assert_eq!(back.conversion.exposure, 0.7);
        assert_eq!(back.tone.balance, [1.1, 1.0, 0.9]);
        assert_eq!(back.profile.id, new.profile.id);
    }

    #[test]
    fn no_change_is_an_empty_diff() {
        assert!(Settings::diff(&base(), &base()).is_empty());
    }

    #[test]
    fn frame_overrides_win_over_roll_settings() {
        let roll = Settings { exposure: Some(1.0), black_point: Some(0.05), ..Default::default() };
        let frame = Settings { exposure: Some(-0.5), ..Default::default() };
        let mut p = base();
        roll.apply_to(&mut p);
        frame.apply_to(&mut p);
        assert_eq!((p.conversion.exposure, p.tone.black_point), (-0.5, 0.05));
    }

    #[test]
    fn merge_and_clear_are_inverse_on_the_fields_touched() {
        let mut s = Settings { exposure: Some(1.0), midtone: Some(1.2), ..Default::default() };
        let d = Settings { exposure: Some(2.0), contrast: Some(0.3), ..Default::default() };
        s.merge(&d);
        assert_eq!((s.exposure, s.midtone, s.contrast), (Some(2.0), Some(1.2), Some(0.3)));
        s.clear(&d);
        assert_eq!((s.exposure, s.midtone, s.contrast), (None, Some(1.2), None));
    }

    #[test]
    fn json_round_trip_keeps_only_what_is_set() {
        let s = Settings { image_type: Some(ImageType::Positive), film_base: Some([0.6, 0.3, 0.2]), ..Default::default() };
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
        assert!(serde_json::from_str::<Settings>("{}").unwrap().is_empty());
    }

    #[test]
    fn looks_change_what_they_say_and_neutral_undoes_the_others() {
        let mut p = base();
        let plain = p.clone();
        Look::Lighter.settings().apply_to(&mut p);
        assert!(p.conversion.exposure > plain.conversion.exposure);
        Look::Warmer.settings().apply_to(&mut p);
        assert!(p.tone.balance[0] > p.tone.balance[2]);
        Look::Neutral.settings().apply_to(&mut p);
        assert_eq!((p.conversion.exposure, p.tone.balance), (plain.conversion.exposure, plain.tone.balance));
    }

    #[test]
    fn colour_settings_round_trip_through_a_diff() {
        let base = base();
        let mut edited = base.clone();
        edited.colour.saturation = 0.3;
        edited.colour.acr = 40.0;
        edited.colour.wheel[2] = [0.2, -0.4];
        edited.colour.selective[4] = [0.1, -0.3, 0.5];
        edited.colour.grey.enabled = true;
        edited.colour.grey.weights = crate::GREY_PRESETS[1].1;
        let s = Settings::diff(&base, &edited);
        let mut out = base.clone();
        s.apply_to(&mut out);
        assert_eq!(out.colour, edited.colour);
        assert!(Settings::diff(&base, &base).is_empty());
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }
}
