//! ICC profiles for the output spaces, written here rather than shipped as
//! blobs: small matrix/TRC profiles (ICC v2.4) that every colour-managed
//! program reads. The numbers are the published D50-adapted colorants.

use crate::colour::{ADOBE_GAMMA, ColourSpace, GREY_GAMMA};

/// D50, the profile connection space white.
const D50: [f64; 3] = [0.9642, 1.0, 0.8249];

/// D50-adapted primaries (Bradford), as ICC profiles for these spaces carry them.
fn colorants(space: ColourSpace) -> [[f64; 3]; 3] {
    match space {
        ColourSpace::Srgb => [[0.4360, 0.2225, 0.0139], [0.3851, 0.7169, 0.0971], [0.1431, 0.0606, 0.7141]],
        ColourSpace::AdobeRgb => [[0.6097, 0.3111, 0.0195], [0.2053, 0.6257, 0.0609], [0.1492, 0.0632, 0.7446]],
    }
}

fn s15f16(v: f64) -> [u8; 4] {
    ((v * 65536.0).round() as i32).to_be_bytes()
}

fn pad4(v: &mut Vec<u8>) {
    while v.len() % 4 != 0 {
        v.push(0);
    }
}

fn xyz_tag(xyz: [f64; 3]) -> Vec<u8> {
    let mut t = b"XYZ \0\0\0\0".to_vec();
    for c in xyz {
        t.extend_from_slice(&s15f16(c));
    }
    t
}

fn text_tag(s: &str) -> Vec<u8> {
    let mut t = b"text\0\0\0\0".to_vec();
    t.extend_from_slice(s.as_bytes());
    t.push(0);
    t
}

/// `textDescriptionType`: ASCII, then the empty Unicode and Macintosh parts v2 requires.
fn desc_tag(s: &str) -> Vec<u8> {
    let mut t = b"desc\0\0\0\0".to_vec();
    t.extend_from_slice(&(s.len() as u32 + 1).to_be_bytes());
    t.extend_from_slice(s.as_bytes());
    t.push(0);
    t.extend_from_slice(&[0; 4]); // Unicode language code
    t.extend_from_slice(&[0; 4]); // Unicode count
    t.extend_from_slice(&[0; 2]); // ScriptCode code
    t.push(0); // ScriptCode count
    t.extend_from_slice(&[0; 67]);
    t
}

/// `curveType` with a single gamma, in u8Fixed8.
fn gamma_curve(g: f32) -> Vec<u8> {
    let mut t = b"curv\0\0\0\0".to_vec();
    t.extend_from_slice(&1u32.to_be_bytes());
    t.extend_from_slice(&((g * 256.0).round() as u16).to_be_bytes());
    t
}

/// `curveType` sampled from the sRGB transfer function (its piecewise
/// definition has no single gamma): 1024 entries.
fn srgb_curve() -> Vec<u8> {
    let mut t = b"curv\0\0\0\0".to_vec();
    t.extend_from_slice(&1024u32.to_be_bytes());
    for i in 0..1024 {
        let e = i as f32 / 1023.0;
        let lin = ColourSpace::Srgb.decode(e);
        // The curve maps device value to linear light, 0..1 in u16.
        t.extend_from_slice(&((lin * 65535.0).round() as u16).to_be_bytes());
    }
    t
}

fn assemble(class: &[u8; 4], space: &[u8; 4], tags: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    // Tags that share data (the three TRCs) share one entry in the data area.
    let mut table: Vec<([u8; 4], usize, usize)> = Vec::new();
    let mut data: Vec<u8> = Vec::new();
    let mut shared: Vec<(Vec<u8>, usize, usize)> = Vec::new();
    let base = 128 + 4 + 12 * tags.len();
    for (sig, bytes) in tags {
        if let Some((_, off, len)) = shared.iter().find(|(b, _, _)| *b == bytes) {
            table.push((sig, *off, *len));
            continue;
        }
        pad4(&mut data);
        let off = base + data.len();
        let len = bytes.len();
        data.extend_from_slice(&bytes);
        shared.push((bytes, off, len));
        table.push((sig, off, len));
    }
    pad4(&mut data);

    let mut p = Vec::with_capacity(base + data.len());
    p.extend_from_slice(&((base + data.len()) as u32).to_be_bytes()); // size
    p.extend_from_slice(&[0; 4]); // preferred CMM
    p.extend_from_slice(&[2, 0x40, 0, 0]); // version 2.4.0
    p.extend_from_slice(class);
    p.extend_from_slice(space);
    p.extend_from_slice(b"XYZ ");
    for v in [2024u16, 1, 1, 0, 0, 0] {
        p.extend_from_slice(&v.to_be_bytes()); // date: fixed, so profiles are reproducible
    }
    p.extend_from_slice(b"acsp");
    p.extend_from_slice(&[0; 4 + 4 + 4 + 4 + 8]); // platform, flags, manufacturer, model, attributes
    p.extend_from_slice(&[0; 4]); // rendering intent: perceptual
    for c in D50 {
        p.extend_from_slice(&s15f16(c));
    }
    p.extend_from_slice(b"nvib"); // creator
    p.extend_from_slice(&[0; 16]); // profile ID
    p.extend_from_slice(&[0; 28]); // reserved
    assert_eq!(p.len(), 128);

    p.extend_from_slice(&(table.len() as u32).to_be_bytes());
    for (sig, off, len) in table {
        p.extend_from_slice(&sig);
        p.extend_from_slice(&(off as u32).to_be_bytes());
        p.extend_from_slice(&(len as u32).to_be_bytes());
    }
    p.extend_from_slice(&data);
    p
}

/// The ICC profile for RGB output in `space`.
pub fn rgb_profile(space: ColourSpace) -> Vec<u8> {
    let c = colorants(space);
    let trc = match space {
        ColourSpace::Srgb => srgb_curve(),
        ColourSpace::AdobeRgb => gamma_curve(ADOBE_GAMMA),
    };
    assemble(
        b"mntr",
        b"RGB ",
        vec![
            (*b"desc", desc_tag(space.name())),
            (*b"cprt", text_tag("No copyright, use freely")),
            (*b"wtpt", xyz_tag(D50)),
            (*b"rXYZ", xyz_tag(c[0])),
            (*b"gXYZ", xyz_tag(c[1])),
            (*b"bXYZ", xyz_tag(c[2])),
            (*b"rTRC", trc.clone()),
            (*b"gTRC", trc.clone()),
            (*b"bTRC", trc),
        ],
    )
}

/// The ICC profile for grey output: gamma 2.2.
pub fn grey_profile() -> Vec<u8> {
    assemble(
        b"mntr",
        b"GRAY",
        vec![(*b"desc", desc_tag("Grey gamma 2.2")), (*b"cprt", text_tag("No copyright, use freely")), (*b"wtpt", xyz_tag(D50)), (*b"kTRC", gamma_curve(GREY_GAMMA))],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Parsed {
        size: u32,
        tags: Vec<([u8; 4], usize, usize)>,
    }

    fn parse(p: &[u8]) -> Parsed {
        let u32_at = |o: usize| u32::from_be_bytes(p[o..o + 4].try_into().unwrap());
        let n = u32_at(128) as usize;
        let tags = (0..n).map(|i| (p[132 + 12 * i..136 + 12 * i].try_into().unwrap(), u32_at(136 + 12 * i) as usize, u32_at(140 + 12 * i) as usize)).collect();
        Parsed { size: u32_at(0), tags }
    }

    fn structurally_valid(p: &[u8], class: &[u8], space: &[u8]) {
        let parsed = parse(p);
        assert_eq!(parsed.size as usize, p.len(), "size field");
        assert_eq!(p.len() % 4, 0);
        assert_eq!(&p[12..16], class);
        assert_eq!(&p[16..20], space);
        assert_eq!(&p[20..24], b"XYZ ");
        assert_eq!(&p[36..40], b"acsp");
        assert_eq!(p[8], 2, "version");
        for (sig, off, len) in &parsed.tags {
            assert!(off % 4 == 0 && off + len <= p.len(), "tag {} out of bounds", String::from_utf8_lossy(sig));
        }
    }

    #[test]
    fn rgb_profiles_are_well_formed_and_complete() {
        for space in [ColourSpace::Srgb, ColourSpace::AdobeRgb] {
            let p = rgb_profile(space);
            structurally_valid(&p, b"mntr", b"RGB ");
            let tags: Vec<_> = parse(&p).tags.iter().map(|t| String::from_utf8_lossy(&t.0).into_owned()).collect();
            for want in ["desc", "cprt", "wtpt", "rXYZ", "gXYZ", "bXYZ", "rTRC", "gTRC", "bTRC"] {
                assert!(tags.iter().any(|t| t == want), "{space:?} lacks {want}");
            }
        }
    }

    #[test]
    fn colorants_add_up_to_the_connection_space_white() {
        for space in [ColourSpace::Srgb, ColourSpace::AdobeRgb] {
            let c = colorants(space);
            for axis in 0..3 {
                let sum: f64 = c.iter().map(|p| p[axis]).sum();
                assert!((sum - D50[axis]).abs() < 2e-3, "{space:?} axis {axis}: {sum}");
            }
        }
    }

    #[test]
    fn the_three_curves_share_one_block() {
        let p = rgb_profile(ColourSpace::Srgb);
        let t = parse(&p).tags;
        let find = |s: &[u8; 4]| t.iter().find(|x| &x.0 == s).unwrap();
        assert_eq!(find(b"rTRC").1, find(b"gTRC").1);
        assert_eq!(find(b"gTRC").1, find(b"bTRC").1);
    }

    #[test]
    fn the_srgb_curve_is_the_srgb_function() {
        let p = rgb_profile(ColourSpace::Srgb);
        let (_, off, _) = *parse(&p).tags.iter().find(|x| &x.0 == b"rTRC").unwrap();
        assert_eq!(&p[off..off + 4], b"curv");
        assert_eq!(u32::from_be_bytes(p[off + 8..off + 12].try_into().unwrap()), 1024);
        let at = |i: usize| u16::from_be_bytes(p[off + 12 + 2 * i..off + 14 + 2 * i].try_into().unwrap()) as f32 / 65535.0;
        assert_eq!(at(0), 0.0);
        assert!((at(1023) - 1.0).abs() < 1e-4);
        // Encoded 0.5 is 18.6% linear.
        assert!((at(512) - 0.2140).abs() < 0.01, "{}", at(512));
    }

    #[test]
    fn the_grey_profile_is_a_grey_profile() {
        let p = grey_profile();
        structurally_valid(&p, b"mntr", b"GRAY");
        let tags: Vec<_> = parse(&p).tags.iter().map(|t| t.0).collect();
        assert!(tags.contains(b"kTRC") && !tags.contains(b"rXYZ"));
    }

    #[test]
    fn profiles_are_reproducible() {
        assert_eq!(rgb_profile(ColourSpace::AdobeRgb), rgb_profile(ColourSpace::AdobeRgb));
    }
}
