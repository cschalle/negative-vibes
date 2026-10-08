//! Dust removal on the real tulip scan and its infrared plane.

use std::path::{Path, PathBuf};

use nv_film::builtin_profiles;
use nv_process::dust::{apply, detect, DustMode, DustParams};
use nv_process::{default_params_for, render, tiles::freeze, Proxy};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The scan with its infrared plane (the reader merges `tulip-600.pnm` with `tulip-600-ir.pgm`).
fn tulip() -> Proxy {
    let raw = nv_store::RawReader::read(&root().join("tests/fixtures/plustek8200i/tulip-600.pnm")).unwrap();
    assert_eq!(raw.channels, 4, "the infrared partner was not found");
    Proxy::from_raw(&raw, usize::MAX)
}

#[test]
fn the_real_scan_has_dust_that_is_found_in_the_infrared_and_repaired() {
    let p = tulip();
    let params = DustParams::default();
    let ir = p.ir.as_ref().expect("infrared plane");
    let report = detect(&p.pixels, ir, p.width, p.height, &params, 1);

    // Some dust, not a picture's worth.
    assert!((0.0005..0.04).contains(&report.covered), "covers {:.3}%", report.covered * 100.0);
    assert!(report.marks >= 20, "only {} marks on a dusty scan", report.marks);
    let ir = p.ir.as_ref().unwrap();

    // What was marked is darker in the infrared than its surroundings: each
    // marked pixel against the mean of the 15 × 15 pixels around it, which
    // also averages in the dust itself, so the bar is modest (about 4% darker is what it measures).
    let (w, h) = (p.width, p.height);
    let local = |x: usize, y: usize| -> f32 {
        let (x0, x1, y0, y1) = (x.saturating_sub(7), (x + 8).min(w), y.saturating_sub(7), (y + 8).min(h));
        let mut s = 0.0;
        for yy in y0..y1 { for xx in x0..x1 { s += ir[yy * w + xx]; } }
        s / ((x1 - x0) * (y1 - y0)) as f32
    };
    let (mut marked, mut n_marked, mut other, mut n_other) = (0f64, 0f64, 0f64, 0f64);
    // Inside the film window only: the holder's black surround is not dust.
    for y in h / 8..h * 7 / 8 {
        for x in w / 16..w * 15 / 16 {
            let r = (ir[y * w + x] / local(x, y).max(1e-4)) as f64;
            if report.mask[y * w + x] == 1 { marked += r; n_marked += 1.0 } else { other += r; n_other += 1.0 }
        }
    }
    assert!(marked / n_marked < 0.975 && other / n_other > 0.99, "marked {:.3} vs the rest {:.3} of their surroundings", marked / n_marked, other / n_other);

    // Repair touches only the mask.
    let (fixed, _) = apply(&p.pixels, Some(ir), p.width, p.height, &params, 1);
    let changed = (0..p.width * p.height).filter(|i| fixed[i * 3..i * 3 + 3] != p.pixels[i * 3..i * 3 + 3]).count();
    let masked = report.mask.iter().filter(|m| **m == 1).count();
    assert!(changed <= masked && changed > masked / 2, "{changed} changed of {masked} masked");
}

#[test]
fn dust_removal_does_not_eat_the_picture() {
    // The picture's own detail (stems, petals) must not be in the mask: the
    // marked pixels, seen in the colour data, are not unusually bright or dark
    // for their neighbourhood in the way real picture edges are. A crude but
    // honest check: the mask is thin (marks are small) and mostly dark in IR.
    let p = tulip();
    let report = detect(&p.pixels, p.ir.as_ref().unwrap(), p.width, p.height, &DustParams::default(), 1);
    let area_per_mark = report.mask.iter().filter(|m| **m == 1).count() as f32 / report.marks.max(1) as f32;
    assert!(area_per_mark < 120.0, "marks average {area_per_mark} px: that is not dust");
}

#[test]
fn view_the_result() {
    // Writes before/after PNGs when asked, for a person to look at.
    let Some(dir) = std::env::var_os("NV_DUST_VIEW") else { return };
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let p = tulip();
    let mut params = default_params_for(&builtin_profiles()[0]);
    params.dust = DustParams { mode: DustMode::Off, ..Default::default() };
    let frozen = freeze(&p, &params);
    let png = |name: &str, rgb: &[u8]| {
        let file = std::io::BufWriter::new(std::fs::File::create(dir.join(name)).unwrap());
        let mut enc = png::Encoder::new(file, p.width as u32, p.height as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(rgb).unwrap();
    };
    png("off.png", &render(&p, &frozen));
    let mut c = frozen.clone();
    c.dust.mode = DustMode::Correct;
    png("correct.png", &render(&p, &c));
    let mut m = frozen;
    m.dust.mode = DustMode::Mark;
    png("mark.png", &render(&p, &m));
}

/// A full-resolution render in tiles repairs the same dust the whole-picture
/// render does, to within the small differences that per-tile statistics make.
#[test]
fn tiles_with_dust_removal_agree_with_the_whole_picture() {
    let raw = nv_store::RawReader::read(&root().join("tests/fixtures/plustek8200i/tulip-600.pnm")).unwrap();
    let p = Proxy::from_raw(&raw, usize::MAX);
    let frozen = freeze(&p, &default_params_for(&builtin_profiles()[0]));
    assert_eq!(frozen.dust.mode, DustMode::Correct);
    let whole = render(&p, &frozen);
    let tiled = nv_process::tiles::render_full(&raw, &frozen, 2);
    assert_eq!(whole.len(), tiled.len());
    let far = whole.iter().zip(&tiled).filter(|(a, b)| (**a as i32 - **b as i32).abs() > 8).count();
    assert!((far as f64) < 0.01 * whole.len() as f64, "{:.2}% of samples differ by more than 8 levels", 100.0 * far as f64 / whole.len() as f64);

    // And dust removal does something: against the same picture with it off.
    let mut off = frozen.clone();
    off.dust.mode = DustMode::Off;
    let untouched = render(&p, &off);
    assert!(whole != untouched, "dust removal changed nothing");
}

/// A heal stroke lands at the same place in a tiled export as in the
/// whole-picture render.
#[test]
fn strokes_agree_between_tiles_and_the_whole_picture() {
    use nv_process::Stroke;
    let raw = nv_store::RawReader::read(&root().join("tests/fixtures/plustek8200i/tulip-600.pnm")).unwrap();
    let p = Proxy::from_raw(&raw, usize::MAX);
    let mut params = default_params_for(&builtin_profiles()[0]);
    params.dust.mode = DustMode::Mark; // marks show exactly which pixels the strokes cover
    params.dust.method = nv_process::DustMethod::Software;
    params.dust.detection = 1; // find (almost) nothing by itself
    params.dust.include = vec![Stroke { points: vec![[0.55, 0.5], [0.62, 0.55]], radius: 0.02 }];
    let frozen = freeze(&p, &params);
    let whole = render(&p, &frozen);
    let tiled = nv_process::tiles::render_full(&raw, &frozen, 2);
    let magenta = |rgb: &[u8]| rgb.chunks(3).filter(|px| px[0] > 240 && px[1] < 20 && px[2] > 240).count();
    let (a, b) = (magenta(&whole), magenta(&tiled));
    assert!(a > 500, "the stroke painted only {a} pixels");
    assert!((a as i64 - b as i64).abs() < (a as i64) / 20, "the stroke covers {a} pixels whole and {b} in tiles");
    // It lies where the fractions say: around x = 0.58 of the width.
    let xs: Vec<usize> = tiled.chunks(3).enumerate().filter(|(_, px)| px[0] > 240 && px[1] < 20 && px[2] > 240).map(|(i, _)| i % p.width).collect();
    let mean = xs.iter().sum::<usize>() as f64 / xs.len() as f64 / p.width as f64;
    assert!((mean - 0.58).abs() < 0.04, "centre at {mean}");
}

/// The 8200i delivers its infrared registered with the colour, and the
/// estimator agrees: it finds (almost) no offset on the real scan.
#[test]
fn the_real_infrared_is_registered_with_the_colour() {
    let p = tulip();
    let s = nv_process::dust::estimate_shift(&p.pixels, p.ir.as_ref().unwrap(), p.width, p.height, 1).expect("structure to correlate");
    assert!(s[0].abs() < 0.5 && s[1].abs() < 0.5, "the estimator found an offset of {s:?} on a registered scan");
    // Shifted by hand, it finds that.
    let ir = p.ir.as_ref().unwrap();
    let moved = nv_process::dust::shift_plane(ir, p.width, p.height, -6.0, 4.0);
    let s = nv_process::dust::estimate_shift(&p.pixels, &moved, p.width, p.height, 1).unwrap();
    assert!((s[0] - 6.0).abs() < 0.6 && (s[1] + 4.0).abs() < 0.6, "{s:?}");
}
