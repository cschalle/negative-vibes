//! Temporary harness: run the detail stage on a PPM so USM, noise reduction and
//! local contrast can be measured against another converter. Driven by
//! environment variables; does nothing unless NV_H_IN is set.

use nv_process::detail::{self, DetailParams, NoiseLevel, Placement, Quality, UsmParams};

fn read_ppm(path: &str) -> (Vec<f32>, usize, usize) {
    let raw = std::fs::read(path).expect("input");
    let mut it = raw.splitn(4, |&b| b == b'\n');
    assert_eq!(it.next().unwrap(), b"P6", "P6 only");
    let dims: Vec<usize> = String::from_utf8_lossy(it.next().unwrap()).split_whitespace().map(|s| s.parse().unwrap()).collect();
    let maxv: usize = String::from_utf8_lossy(it.next().unwrap()).trim().parse().unwrap();
    assert_eq!(maxv, 255, "8-bit only");
    let body = it.next().unwrap();
    let (w, h) = (dims[0], dims[1]);
    (body[..w * h * 3].iter().map(|&b| b as f32 / 255.0).collect(), w, h)
}

fn write_ppm(path: &str, px: &[f32], w: usize, h: usize) {
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    out.extend(px.iter().map(|&v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
    std::fs::write(path, out).expect("output");
}

#[test]
fn detail_harness() {
    let Ok(input) = std::env::var("NV_H_IN") else { return };
    let output = std::env::var("NV_H_OUT").expect("NV_H_OUT");
    let (mut px, w, h) = read_ppm(&input);

    let mut d = DetailParams::default();
    if let Ok(s) = std::env::var("NV_H_USM")
        && s != "off"
    {
        let f: Vec<f32> = s.split(',').map(|x| x.parse().unwrap()).collect();
        d.usm = UsmParams { enabled: true, amount: f[0], radius: f[1], threshold: f[2] as u8 };
    }
    d.noise = match std::env::var("NV_H_NOISE").unwrap_or_default().as_str() {
        "light" => NoiseLevel::Light,
        "medium" => NoiseLevel::Medium,
        "strong" => NoiseLevel::Strong,
        _ => NoiseLevel::Off,
    };
    d.local_contrast = std::env::var("NV_H_AACO").ok().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    if d.local_contrast > 0.0 {
        d.aaco = Some(detail::measure_grid(&px, w, h));
    }

    detail::apply(&mut px, w, h, &d, 1, Quality::Full, Placement { origin: (0, 0), full: (w, h) });
    write_ppm(&output, &px, w, h);
}
