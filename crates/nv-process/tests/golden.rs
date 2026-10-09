//! Whole-pipeline tests on the real tulip scan (DESIGN.md §11.1).
//!
//! Each case renders a fixture and compares it with a stored 8-bit PNG in
//! `tests/goldens/`. Regenerate deliberately with
//! `NV_UPDATE_GOLDENS=1 cargo test -p nv-process --test golden`.

use std::path::{Path, PathBuf};

use nv_film::builtin_profiles;
use nv_process::{Histogram, ProcessParams, Proxy, auto_tone, default_params_for, estimate_film_base_proxy, pipette, render, sample_converted};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tulip() -> Proxy {
    let raw = nv_store::RawReader::read(&root().join("tests/fixtures/plustek8200i/tulip-600.pnm")).expect("fixture");
    Proxy::from_raw(&raw, usize::MAX)
}

/// What Open Raw Scans does: generic colour negative, film base estimated.
fn defaults(p: &Proxy) -> ProcessParams {
    let mut params = default_params_for(&builtin_profiles()[0]);
    params.conversion.film_base = Some(estimate_film_base_proxy(p));
    params
}

fn write_png(path: &Path, w: usize, h: usize, rgb: &[u8]) {
    let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut enc = png::Encoder::new(file, w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(rgb).unwrap();
}

fn read_png(path: &Path) -> (usize, usize, Vec<u8>) {
    let dec = png::Decoder::new(std::fs::File::open(path).unwrap());
    let mut r = dec.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    (info.width as usize, info.height as usize, buf)
}

/// Compare with the golden: mean absolute error under 1.5 levels and no
/// more than 0.5% of samples more than 12 levels off.
fn check_golden(name: &str, w: usize, h: usize, rgb: &[u8]) {
    let path = root().join("tests/goldens").join(format!("{name}.png"));
    if std::env::var_os("NV_UPDATE_GOLDENS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        write_png(&path, w, h, rgb);
        return;
    }
    let (gw, gh, want) = read_png(&path);
    assert_eq!((gw, gh), (w, h), "{name}: size changed");
    let diffs: Vec<u32> = rgb.iter().zip(&want).map(|(a, b)| (*a as i32 - *b as i32).unsigned_abs()).collect();
    let mean = diffs.iter().sum::<u32>() as f64 / diffs.len() as f64;
    let far = diffs.iter().filter(|d| **d > 12).count() as f64 / diffs.len() as f64;
    assert!(mean < 1.5 && far < 0.005, "{name}: mean error {mean:.2}, {:.2}% of samples far off", far * 100.0);
}

/// Mean absolute difference between neighbouring pixels of one channel
/// over the picture interior: colour speckle shows up as a large value.
fn roughness(rgb: &[u8], w: usize, h: usize) -> f64 {
    let (mut sum, mut n) = (0u64, 0u64);
    for y in h / 6..h * 5 / 6 {
        for x in w / 6..w * 5 / 6 - 1 {
            let i = (y * w + x) * 3;
            sum += (rgb[i + 1] as i32 - rgb[i + 4] as i32).unsigned_abs() as u64;
            n += 1;
        }
    }
    sum as f64 / n as f64
}

#[test]
fn tulip_default_conversion_is_plausible_and_stable() {
    let p = tulip();
    let rgb = render(&p, &defaults(&p));
    let h = Histogram::from_rgb8(&rgb);
    let px = (p.width * p.height) as f64;

    // A picture, not a black or white frame.
    let mean: f64 = h.luma.iter().enumerate().map(|(i, n)| i as f64 * *n as f64).sum::<f64>() / px;
    assert!((40.0..215.0).contains(&mean), "mean luma {mean}");
    // Little clipping.
    assert!((h.luma[0] + h.luma[255]) as f64 / px < 0.25, "clipped fraction too high");
    check_golden("tulip-default", p.width, p.height, &rgb);
}

#[test]
fn tulip_auto_levels_use_the_whole_range_without_speckle() {
    let p = tulip();
    let mut params = defaults(&p);
    let before = render(&p, &params);
    params.tone = auto_tone(&p, &params);
    let after = render(&p, &params);

    // Auto levels spread the interior over the range.
    let spread = |rgb: &[u8]| -> u32 {
        let mut luma: Vec<u8> = Vec::new();
        for y in p.height / 6..p.height * 5 / 6 {
            for x in p.width / 6..p.width * 5 / 6 {
                let i = (y * p.width + x) * 3;
                luma.push(rgb[i + 1]);
            }
        }
        luma.sort_unstable();
        (luma[luma.len() * 98 / 100] - luma[luma.len() * 2 / 100]) as u32
    };
    assert!(spread(&after) >= spread(&before), "auto levels narrowed the range");
    assert!(spread(&after) > 120, "auto levels left a flat picture: {}", spread(&after));
    // And do not turn grain into confetti.
    assert!(roughness(&after, p.width, p.height) < 14.0, "speckle: {}", roughness(&after, p.width, p.height));
    check_golden("tulip-auto", p.width, p.height, &after);
}

#[test]
fn rendering_a_reduced_proxy_matches_full_size_closely() {
    let raw = nv_store::RawReader::read(&root().join("tests/fixtures/plustek8200i/tulip-600.pnm")).unwrap();
    let full = Proxy::from_raw(&raw, usize::MAX);
    let small = Proxy::from_raw(&raw, 100_000);
    assert!(small.width < full.width);
    let params = defaults(&full);
    let a = Histogram::from_rgb8(&render(&full, &params));
    let b = Histogram::from_rgb8(&render(&small, &params));
    let mean = |h: &Histogram| h.luma.iter().enumerate().map(|(i, n)| i as f64 * *n as f64).sum::<f64>() / h.luma.iter().sum::<u32>() as f64;
    assert!((mean(&a) - mean(&b)).abs() < 4.0, "{} vs {}", mean(&a), mean(&b));
}

/// Click the concrete wall with the grey pipette: it comes out neutral, and
/// the rest of the picture keeps its colours.
#[test]
fn grey_pipette_on_the_wall_neutralises_it() {
    let p = tulip();
    let mut params = defaults(&p);
    params.tone = auto_tone(&p, &params);

    let wall = |rgb: &[u8]| -> [f64; 3] {
        let mut acc = [0f64; 3];
        let mut n = 0.0;
        for y in 90..150 {
            for x in 720..800 {
                for c in 0..3 {
                    acc[c] += rgb[(y * p.width + x) * 3 + c] as f64;
                }
                n += 1.0;
            }
        }
        [acc[0] / n, acc[1] / n, acc[2] / n]
    };
    let before = wall(&render(&p, &params));
    let spread = |w: [f64; 3]| w.iter().cloned().fold(f64::MIN, f64::max) - w.iter().cloned().fold(f64::MAX, f64::min);
    assert!(spread(before) > 15.0, "fixture wall is not tinted: {before:?}");

    let sample = sample_converted(&p, &neutral_tone(&params), 760.0 / p.width as f64, 120.0 / p.height as f64, 6).unwrap();
    params.tone = pipette::grey(&params.tone, sample);
    let rgb = render(&p, &params);
    let after = wall(&rgb);
    assert!(spread(after) < 8.0, "wall still tinted: {after:?}");
    check_golden("tulip-grey-wall", p.width, p.height, &rgb);
}

/// The pipettes sample the conversion before tone and balance.
fn neutral_tone(params: &ProcessParams) -> ProcessParams {
    ProcessParams { tone: Default::default(), ..params.clone() }
}
