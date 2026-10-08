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
}

macro_rules! each_field {
    ($m:ident) => {
        $m!(image_type, profile, film_base, d_max, cast_fit, exposure, cast_removal, black_point, white_point, midtone, contrast, balance, curves)
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
            if let Some(profile) = nv_film::builtin_profiles().into_iter().find(|f| &f.id == id) {
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
        }
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
}
