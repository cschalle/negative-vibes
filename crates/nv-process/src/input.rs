//! A scanner input profile (F§11): the matrix and tone curves of a matrix/TRC
//! ICC profile, as ArgyllCMS `colprof` builds from an IT8 target, reduced to
//! what the pipeline applies to raw scanner values: curves, then a matrix to
//! linear sRGB.

use serde::{Deserialize, Serialize};

/// Points in each sampled curve.
const CURVE_POINTS: usize = 256;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputProfile {
    pub name: String,
    /// Device-linear RGB to linear sRGB (D50 PCS, as the profile has it).
    pub matrix: [[f32; 3]; 3],
    /// Device value (0–1, evenly spaced) to linear light, per channel.
    pub trc: [Vec<f32>; 3],
}

#[derive(Debug, PartialEq)]
pub enum IccError {
    Truncated,
    NotMatrixProfile,
    BadTag(&'static str),
}

impl std::fmt::Display for IccError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IccError::Truncated => write!(f, "the ICC profile is truncated"),
            IccError::NotMatrixProfile => write!(f, "the ICC profile is not a matrix/curve RGB profile"),
            IccError::BadTag(t) => write!(f, "the ICC profile has an unreadable {t} tag"),
        }
    }
}

impl std::error::Error for IccError {}

/// XYZ (D50) to linear sRGB: the inverse of the D50-adapted sRGB colorants.
const XYZ_D50_TO_SRGB: [[f64; 3]; 3] = [[3.1338561, -1.6168667, -0.4906146], [-0.9787684, 1.9161415, 0.0334540], [0.0719453, -0.2289914, 1.4052427]];

fn be32(b: &[u8], at: usize) -> Result<u32, IccError> {
    b.get(at..at + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]])).ok_or(IccError::Truncated)
}

fn s15f16(b: &[u8], at: usize) -> Result<f64, IccError> {
    Ok(be32(b, at)? as i32 as f64 / 65536.0)
}

fn tag<'a>(b: &'a [u8], sig: &[u8; 4]) -> Option<&'a [u8]> {
    let n = be32(b, 128).ok()? as usize;
    for i in 0..n {
        let e = 132 + i * 12;
        if b.get(e..e + 4)? == sig {
            let (off, len) = (be32(b, e + 4).ok()? as usize, be32(b, e + 8).ok()? as usize);
            return b.get(off..off + len);
        }
    }
    None
}

fn xyz(t: &[u8], name: &'static str) -> Result<[f64; 3], IccError> {
    if t.get(..4) != Some(b"XYZ ") {
        return Err(IccError::BadTag(name));
    }
    Ok([s15f16(t, 8)?, s15f16(t, 12)?, s15f16(t, 16)?])
}

/// Sample a `curv` or `para` tag at `x` in 0–1.
fn curve(t: &[u8], name: &'static str) -> Result<Vec<f32>, IccError> {
    let bad = IccError::BadTag(name);
    let f: Box<dyn Fn(f64) -> f64> = match t.get(..4) {
        Some(b"curv") => {
            let n = be32(t, 8)? as usize;
            match n {
                0 => Box::new(|x| x),
                1 => {
                    let g = u16::from_be_bytes([*t.get(12).ok_or(IccError::Truncated)?, *t.get(13).ok_or(IccError::Truncated)?]) as f64 / 256.0;
                    Box::new(move |x| x.powf(g))
                }
                _ => {
                    let table: Vec<f64> = (0..n)
                        .map(|i| t.get(12 + i * 2..14 + i * 2).map(|s| u16::from_be_bytes([s[0], s[1]]) as f64 / 65535.0).ok_or(IccError::Truncated))
                        .collect::<Result<_, _>>()?;
                    Box::new(move |x| {
                        let p = x.clamp(0.0, 1.0) * (table.len() - 1) as f64;
                        let (i, fr) = (p.floor() as usize, p - p.floor());
                        let j = (i + 1).min(table.len() - 1);
                        table[i] * (1.0 - fr) + table[j] * fr
                    })
                }
            }
        }
        Some(b"para") => {
            let kind = u16::from_be_bytes([*t.get(8).ok_or(IccError::Truncated)?, *t.get(9).ok_or(IccError::Truncated)?]);
            let n = [1, 3, 4, 5, 7].get(kind as usize).copied().ok_or(bad)?;
            let p: Vec<f64> = (0..n).map(|i| s15f16(t, 12 + i * 4)).collect::<Result<_, _>>()?;
            Box::new(move |x| match kind {
                0 => x.powf(p[0]),
                1 => if x >= -p[2] / p[1] { (p[1] * x + p[2]).max(0.0).powf(p[0]) } else { 0.0 },
                2 => if x >= -p[2] / p[1] { (p[1] * x + p[2]).max(0.0).powf(p[0]) + p[3] } else { p[3] },
                3 => if x >= p[4] { (p[1] * x + p[2]).max(0.0).powf(p[0]) } else { p[3] * x },
                _ => if x >= p[4] { (p[1] * x + p[2]).max(0.0).powf(p[0]) + p[5] } else { p[3] * x + p[6] },
            })
        }
        _ => return Err(bad),
    };
    Ok((0..CURVE_POINTS).map(|i| f(i as f64 / (CURVE_POINTS - 1) as f64).clamp(0.0, 1.0) as f32).collect())
}

impl InputProfile {
    /// Read the matrix and curves out of an RGB matrix/TRC ICC profile.
    pub fn from_icc(name: &str, icc: &[u8]) -> Result<Self, IccError> {
        if icc.len() < 132 {
            return Err(IccError::Truncated);
        }
        if &icc[16..20] != b"RGB " {
            return Err(IccError::NotMatrixProfile);
        }
        let get = |sig: &[u8; 4]| tag(icc, sig).ok_or(IccError::NotMatrixProfile);
        let cols = [xyz(get(b"rXYZ")?, "rXYZ")?, xyz(get(b"gXYZ")?, "gXYZ")?, xyz(get(b"bXYZ")?, "bXYZ")?];
        let trc = [curve(get(b"rTRC")?, "rTRC")?, curve(get(b"gTRC")?, "gTRC")?, curve(get(b"bTRC")?, "bTRC")?];
        // linear sRGB = XYZ_D50_TO_SRGB · [rXYZ gXYZ bXYZ] · linear device
        let mut m = [[0f32; 3]; 3];
        for (r, row) in m.iter_mut().enumerate() {
            for (c, v) in row.iter_mut().enumerate() {
                *v = (0..3).map(|k| XYZ_D50_TO_SRGB[r][k] * cols[c][k]).sum::<f64>() as f32;
            }
        }
        Ok(Self { name: name.to_owned(), matrix: m, trc })
    }

    fn lookup(&self, c: usize, v: f32) -> f32 {
        let t = &self.trc[c];
        if t.is_empty() {
            return v;
        }
        let p = v.clamp(0.0, 1.0) * (t.len() - 1) as f32;
        let (i, fr) = (p as usize, p - p.floor());
        t[i] * (1.0 - fr) + t[(i + 1).min(t.len() - 1)] * fr
    }

    /// Scanner values (0–1, three per pixel) to linear sRGB, in place.
    pub fn apply(&self, pixels: &mut [f32]) {
        for px in pixels.chunks_exact_mut(3) {
            let lin = [self.lookup(0, px[0]), self.lookup(1, px[1]), self.lookup(2, px[2])];
            for r in 0..3 {
                px[r] = (self.matrix[r][0] * lin[0] + self.matrix[r][1] * lin[1] + self.matrix[r][2] * lin[2]).clamp(0.0, 1.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny RGB matrix profile with the given colorants and a gamma curve.
    fn profile(cols: [[f64; 3]; 3], gamma: f64) -> Vec<u8> {
        let mut tags: Vec<([u8; 4], Vec<u8>)> = Vec::new();
        for (sig, c) in [(*b"rXYZ", cols[0]), (*b"gXYZ", cols[1]), (*b"bXYZ", cols[2])] {
            let mut t = b"XYZ \0\0\0\0".to_vec();
            for v in c {
                t.extend(((v * 65536.0).round() as i32).to_be_bytes());
            }
            tags.push((sig, t));
        }
        for sig in [*b"rTRC", *b"gTRC", *b"bTRC"] {
            let mut t = b"curv\0\0\0\0".to_vec();
            t.extend(1u32.to_be_bytes());
            t.extend(((gamma * 256.0).round() as u16).to_be_bytes());
            t.extend([0, 0]);
            tags.push((sig, t));
        }
        let mut out = vec![0u8; 128];
        out[16..20].copy_from_slice(b"RGB ");
        out.extend((tags.len() as u32).to_be_bytes());
        let mut off = 132 + tags.len() * 12;
        let mut body: Vec<u8> = Vec::new();
        for (sig, t) in &tags {
            out.extend(sig);
            out.extend((off as u32).to_be_bytes());
            out.extend((t.len() as u32).to_be_bytes());
            off += t.len();
            body.extend(t.iter().copied());
        }
        out.extend(body);
        out
    }

    #[test]
    fn srgb_colorants_give_an_identity_matrix_and_the_curve_is_applied() {
        let cols = [[0.4360, 0.2225, 0.0139], [0.3851, 0.7169, 0.0971], [0.1431, 0.0606, 0.7141]];
        let p = InputProfile::from_icc("t", &profile(cols, 2.0)).unwrap();
        for r in 0..3 {
            for c in 0..3 {
                assert!((p.matrix[r][c] - if r == c { 1.0 } else { 0.0 }).abs() < 2e-3, "{:?}", p.matrix);
            }
        }
        let mut px = vec![0.5, 1.0, 0.0];
        p.apply(&mut px);
        assert!((px[0] - 0.25).abs() < 1e-2 && (px[1] - 1.0).abs() < 1e-2 && px[2] < 1e-3, "{px:?}");
    }

    #[test]
    fn bad_profiles_are_refused() {
        assert_eq!(InputProfile::from_icc("t", &[0; 10]), Err(IccError::Truncated));
        let mut grey = profile([[0.0; 3]; 3], 1.0);
        grey[16..20].copy_from_slice(b"GRAY");
        assert_eq!(InputProfile::from_icc("t", &grey), Err(IccError::NotMatrixProfile));
    }
}

#[cfg(test)]
mod pipeline_tests {
    use super::*;
    use crate::{default_params_for, ImageType, Proxy};
    use nv_film::builtin_profiles;

    /// A slide scan run through a profile comes out different, and a profile that
    /// does nothing changes nothing.
    #[test]
    fn a_slide_uses_the_scanner_profile() {
        let (w, h) = (16usize, 16usize);
        let pixels: Vec<f32> = (0..w * h).flat_map(|i| { let v = 0.1 + 0.8 * (i % w) as f32 / w as f32; [v, v * 0.8, v * 0.6] }).collect();
        let proxy = Proxy::new(w, h, 1, pixels);
        let mut p = default_params_for(&builtin_profiles()[0]);
        p.image_type = ImageType::Positive;
        let plain = crate::convert(&proxy, &p);

        let identity = InputProfile { name: "id".into(), matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], trc: Default::default() };
        p.input = Some(identity);
        assert_eq!(crate::convert(&proxy, &p), plain);

        let warm = InputProfile { name: "warm".into(), matrix: [[1.0, 0.0, 0.0], [0.0, 0.7, 0.0], [0.0, 0.0, 0.5]], trc: Default::default() };
        p.input = Some(warm);
        let changed = crate::convert(&proxy, &p);
        assert_ne!(changed, plain);
        // A negative ignores it: its colour comes from the film profile.
        p.image_type = ImageType::ColourNegative;
        let neg_with = crate::convert(&proxy, &p);
        p.input = None;
        assert_eq!(neg_with, crate::convert(&proxy, &p));
    }

    /// Kodachrome converts like a slide, and never trusts the infrared plane.
    #[test]
    fn kodachrome_is_a_slide_that_does_not_use_infrared() {
        let (w, h) = (96usize, 96usize);
        let pixels: Vec<f32> = (0..w * h).flat_map(|i| { let (x, y) = ((i % w) as f32, (i / w) as f32);
            let v = 0.45 + 0.3 * (x * 0.05).sin() * (y * 0.04).cos() + 0.1 * (x / w as f32); [v, v * 0.9, v * 0.7] }).collect();
        // A plane with a dark spot: infrared would call it dust.
        let mut ir: Vec<f32> = (0..w * h).map(|i| 0.9 + 0.01 * ((i as u32).wrapping_mul(2654435761) >> 24) as f32 / 128.0).collect();
        for y in 40..44 { for x in 40..44 { ir[y * w + x] = 0.1; } }
        let mut proxy = Proxy::new(w, h, 1, pixels);
        proxy.ir = Some(ir);
        let mut p = default_params_for(&builtin_profiles()[0]);
        p.dust.align = false; // this plane is flat, with nothing to register on
        // Mark mode paints what was found, so we can see what each type looked at.
        p.dust.mode = crate::dust::DustMode::Mark;
        p.dust.method = crate::dust::DustMethod::Infrared;
        let at = (41 * w + 41) * 3;
        let marked = |t: ImageType, method: crate::dust::DustMethod| {
            let mut q = p.clone();
            q.image_type = t;
            q.dust.method = method;
            let out = crate::render_toned_q(&proxy, &q, crate::Quality::Preview);
            out[at..at + 3] == q.dust.mark
        };
        assert!(marked(ImageType::Positive, crate::dust::DustMethod::Infrared), "a slide must find dust in the plane");
        assert!(!marked(ImageType::Kodachrome, crate::dust::DustMethod::Infrared), "Kodachrome must not trust the plane");
        assert!(ImageType::Kodachrome.is_positive() && ImageType::Positive.is_positive() && !ImageType::ColourNegative.is_positive());
    }
}
