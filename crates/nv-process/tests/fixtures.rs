//! Real scans of every image type, when the repository has them.
//!
//! The colour negative (`tulip-600.pnm`) is covered by `golden.rs`. A B&W
//! negative goes in as `bw-negative-*.pnm` and a slide as `slide-*.pnm`; see
//! `tests/fixtures/README.md`. With none present these tests do nothing, and
//! say so, rather than pretending to check.

use std::path::{Path, PathBuf};

use nv_film::builtin_profiles;
use nv_process::{auto_tone, default_params_for, estimate_film_base_proxy, render, Histogram, Proxy};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/plustek8200i")
}

fn fixtures(prefix: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            n.starts_with(prefix) && (n.ends_with(".pnm") || n.ends_with(".tif") || n.ends_with(".dng"))
        })
        .collect();
    v.sort();
    v
}

fn write_png(path: &Path, w: usize, h: usize, rgb: &[u8]) {
    let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut enc = png::Encoder::new(file, w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(rgb).unwrap();
}

fn read_png(path: &Path) -> Vec<u8> {
    let mut r = png::Decoder::new(std::fs::File::open(path).unwrap()).read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    buf
}

/// Convert each fixture of a kind with Auto levels, check it is a picture
/// and matches its golden.
fn check_kind(prefix: &str, profile: usize) {
    let files = fixtures(prefix);
    if files.is_empty() {
        eprintln!("no `{prefix}*` fixtures in tests/fixtures/plustek8200i: nothing checked (see its README.md)");
        return;
    }
    for file in files {
        let name = file.file_stem().unwrap().to_string_lossy().into_owned();
        let raw = nv_store::RawReader::read(&file).unwrap_or_else(|e| panic!("{name}: {e}"));
        let proxy = Proxy::from_raw(&raw, usize::MAX);
        let mut params = default_params_for(&builtin_profiles()[profile]);
        if profile != 2 {
            params.conversion.film_base = Some(estimate_film_base_proxy(&proxy));
        }
        params.tone = auto_tone(&proxy, &params);
        let rgb = render(&proxy, &params);

        let h = Histogram::from_rgb8(&rgb);
        let px = (proxy.width * proxy.height) as f64;
        let mean: f64 = h.luma.iter().enumerate().map(|(i, n)| i as f64 * *n as f64).sum::<f64>() / px;
        assert!((30.0..225.0).contains(&mean), "{name}: mean luma {mean}");
        assert!((h.luma[0] + h.luma[255]) as f64 / px < 0.35, "{name}: mostly clipped");
        if profile == 1 {
            assert!(rgb.chunks(3).all(|p| p[0] == p[1] && p[1] == p[2]), "{name}: B&W output has colour");
        }

        let golden = dir().join("../../goldens").join(format!("{name}.png"));
        if std::env::var_os("NV_UPDATE_GOLDENS").is_some() {
            write_png(&golden, proxy.width, proxy.height, &rgb);
        } else {
            let want = read_png(&golden);
            assert_eq!(want.len(), rgb.len(), "{name}: size changed");
            let mean_err = rgb.iter().zip(&want).map(|(a, b)| (*a as i32 - *b as i32).unsigned_abs() as f64).sum::<f64>() / rgb.len() as f64;
            assert!(mean_err < 1.5, "{name}: mean error {mean_err:.2}");
        }
    }
}

#[test]
fn real_bw_negatives() {
    check_kind("bw-negative-", 1);
}

#[test]
fn real_slides() {
    check_kind("slide-", 2);
}
