//! Exports of real scans, read back with independent decoders.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use nv_export::{copy_raw, export_image, icc, ColourSpace, Compression, ExportError, ExportFormat, ExportSettings};
use nv_film::builtin_profiles;
use nv_image::FrameBuf;
use nv_process::{default_params_for, render, tiles::freeze, ProcessParams, Proxy};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("nv-export-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn tulip() -> (FrameBuf, ProcessParams, Vec<u8>) {
    let raw = nv_store::RawReader::read(&root().join("tests/fixtures/plustek8200i/tulip-600.pnm")).unwrap();
    let proxy = Proxy::from_raw(&raw, usize::MAX);
    let frozen = freeze(&proxy, &default_params_for(&builtin_profiles()[0]));
    let reference = render(&proxy, &frozen);
    (raw, frozen, reference)
}

fn export(raw: &FrameBuf, p: &ProcessParams, s: &ExportSettings, dir: &Path, name: &str) -> PathBuf {
    let dest = dir.join(name);
    export_image(raw, p, s, &dest, &|_| {}, &AtomicBool::new(false)).unwrap_or_else(|e| panic!("{name}: {e}"));
    dest
}

fn tiff_pixels(path: &Path) -> (u32, u32, tiff::ColorType, Vec<u16>, Option<Vec<u8>>) {
    let mut d = tiff::decoder::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap())).unwrap();
    let (w, h) = d.dimensions().unwrap();
    let ct = d.colortype().unwrap();
    let icc = d.get_tag_u8_vec(tiff::tags::Tag::Unknown(34675)).ok();
    let px = match d.read_image().unwrap() {
        tiff::decoder::DecodingResult::U16(v) => v,
        tiff::decoder::DecodingResult::U8(v) => v.into_iter().map(u16::from).collect(),
        _ => panic!("unexpected sample type"),
    };
    (w, h, ct, px, icc)
}

fn mean_abs_diff(a: impl Iterator<Item = f64>, b: impl Iterator<Item = f64>) -> f64 {
    let (mut s, mut n) = (0.0, 0.0);
    for (x, y) in a.zip(b) {
        s += (x - y).abs();
        n += 1.0;
    }
    s / n
}

#[test]
fn a_16_bit_tiff_matches_the_render_and_carries_its_profile() {
    let (raw, frozen, reference) = tulip();
    let dir = scratch("tiff16");
    let dest = export(&raw, &frozen, &ExportSettings::default(), &dir, "a.tif");
    let (w, h, ct, px, icc) = tiff_pixels(&dest);
    assert_eq!((w, h, ct), (864, 645, tiff::ColorType::RGB(16)));
    assert_eq!(icc.as_deref(), Some(&icc::rgb_profile(ColourSpace::Srgb)[..]), "profile not embedded");
    let diff = px.iter().zip(&reference).filter(|(p, r)| ((**p as f64 / 257.0) - **r as f64).abs() > 1.01).count();
    assert_eq!(diff, 0, "{diff} samples are more than a level from the 8-bit render");
    assert!(!dest.with_file_name(".a.tif.part").exists());
}

#[test]
fn every_tiff_compression_holds_the_same_pixels() {
    let (raw, frozen, _) = tulip();
    let dir = scratch("compression");
    let pixels = |c| {
        let s = ExportSettings { compression: c, bit_depth: 8, ..Default::default() };
        tiff_pixels(&export(&raw, &frozen, &s, &dir, &format!("{c:?}.tif"))).3
    };
    let none = pixels(Compression::None);
    assert_eq!(pixels(Compression::Lzw), none);
    assert_eq!(pixels(Compression::Deflate), none);
    let size = |c: Compression| std::fs::metadata(dir.join(format!("{c:?}.tif"))).unwrap().len();
    assert!(size(Compression::Deflate) < size(Compression::None));
}

#[test]
fn the_profile_can_be_left_out() {
    let (raw, frozen, _) = tulip();
    let dir = scratch("noicc");
    let s = ExportSettings { embed_profile: false, bit_depth: 8, ..Default::default() };
    assert!(tiff_pixels(&export(&raw, &frozen, &s, &dir, "n.tif")).4.is_none());
}

#[test]
fn a_jpeg_is_close_to_the_render_and_carries_its_profile() {
    let (raw, frozen, reference) = tulip();
    let dir = scratch("jpeg");
    let s = ExportSettings { format: ExportFormat::Jpeg, jpeg_quality: 92, colour_space: ColourSpace::Srgb, ..Default::default() };
    let dest = export(&raw, &frozen, &s, &dir, "a.jpg");
    let mut d = jpeg_decoder::Decoder::new(std::io::BufReader::new(std::fs::File::open(&dest).unwrap()));
    let px = d.decode().unwrap();
    let info = d.info().unwrap();
    assert_eq!((info.width, info.height, info.pixel_format), (864, 645, jpeg_decoder::PixelFormat::RGB24));
    assert!(mean_abs_diff(px.iter().map(|v| *v as f64), reference.iter().map(|v| *v as f64)) < 3.0);
    assert_eq!(d.icc_profile().as_deref(), Some(&icc::rgb_profile(ColourSpace::Srgb)[..]));
}

#[test]
fn a_16_bit_png_matches_the_render_and_carries_its_profile() {
    let (raw, frozen, reference) = tulip();
    let dir = scratch("png");
    let s = ExportSettings { format: ExportFormat::Png, bit_depth: 16, ..Default::default() };
    let dest = export(&raw, &frozen, &s, &dir, "a.png");
    let mut r = png::Decoder::new(std::fs::File::open(&dest).unwrap()).read_info().unwrap();
    assert_eq!(r.info().bit_depth, png::BitDepth::Sixteen);
    assert_eq!(r.info().icc_profile.as_deref(), Some(&icc::rgb_profile(ColourSpace::Srgb)[..]));
    let mut buf = vec![0; r.output_buffer_size()];
    r.next_frame(&mut buf).unwrap();
    let px = buf.chunks_exact(2).map(|b| u16::from_be_bytes([b[0], b[1]]) as f64 / 257.0);
    assert!(mean_abs_diff(px, reference.iter().map(|v| *v as f64)) < 0.6);
}

/// Convert an Adobe RGB export back to sRGB with our own colour maths and
/// compare it with the sRGB export of the same picture.
#[test]
fn adobe_rgb_is_the_same_picture_in_another_space() {
    let (raw, frozen, _) = tulip();
    let dir = scratch("adobe");
    let srgb = tiff_pixels(&export(&raw, &frozen, &ExportSettings::default(), &dir, "s.tif")).3;
    let s = ExportSettings { colour_space: ColourSpace::AdobeRgb, ..Default::default() };
    let adobe_path = export(&raw, &frozen, &s, &dir, "a.tif");
    let (_, _, _, adobe, icc_bytes) = tiff_pixels(&adobe_path);
    assert_eq!(icc_bytes.as_deref(), Some(&icc::rgb_profile(ColourSpace::AdobeRgb)[..]));

    // Inverse of the sRGB -> Adobe matrix, by cofactors.
    let m = ColourSpace::AdobeRgb.matrix().map(|r| r.map(|v| v as f64));
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let inv = [
        [(m[1][1] * m[2][2] - m[1][2] * m[2][1]) / det, (m[0][2] * m[2][1] - m[0][1] * m[2][2]) / det, (m[0][1] * m[1][2] - m[0][2] * m[1][1]) / det],
        [(m[1][2] * m[2][0] - m[1][0] * m[2][2]) / det, (m[0][0] * m[2][2] - m[0][2] * m[2][0]) / det, (m[0][2] * m[1][0] - m[0][0] * m[1][2]) / det],
        [(m[1][0] * m[2][1] - m[1][1] * m[2][0]) / det, (m[0][1] * m[2][0] - m[0][0] * m[2][1]) / det, (m[0][0] * m[1][1] - m[0][1] * m[1][0]) / det],
    ];

    let mut worst = 0.0f64;
    let mut checked = 0;
    for (a, s) in adobe.chunks_exact(3).zip(srgb.chunks_exact(3)) {
        // Away from clipping: out-of-gamut values are clamped on purpose.
        if a.iter().any(|v| *v < 1500 || *v > 64000) {
            continue;
        }
        let lin: Vec<f64> = a.iter().map(|v| ColourSpace::AdobeRgb.decode(*v as f32 / 65535.0) as f64).collect();
        for c in 0..3 {
            let l = inv[c][0] * lin[0] + inv[c][1] * lin[1] + inv[c][2] * lin[2];
            let back = ColourSpace::Srgb.encode(l as f32) as f64 * 65535.0;
            worst = worst.max((back - s[c] as f64).abs());
        }
        checked += 1;
    }
    assert!(checked > 100_000, "only {checked} pixels were checkable");
    assert!(worst < 200.0, "worst difference {worst} of 65535");
}

/// An independent check of the ICC profile and the matrix: ImageMagick (lcms)
/// converts the Adobe RGB export to sRGB using our two profiles, and the
/// result must be the sRGB export. Skipped when ImageMagick is not installed.
#[test]
fn an_independent_colour_engine_agrees_with_our_profiles() {
    if std::process::Command::new("magick").arg("-version").output().is_err() {
        eprintln!("ImageMagick not installed: skipping the independent colour check");
        return;
    }
    let (raw, frozen, _) = tulip();
    let dir = scratch("magick");
    let srgb = export(&raw, &frozen, &ExportSettings { bit_depth: 8, ..Default::default() }, &dir, "s.tif");
    let adobe = export(&raw, &frozen, &ExportSettings { bit_depth: 8, colour_space: ColourSpace::AdobeRgb, ..Default::default() }, &dir, "a.tif");
    std::fs::write(dir.join("srgb.icc"), icc::rgb_profile(ColourSpace::Srgb)).unwrap();

    let out = dir.join("converted.tif");
    let status = std::process::Command::new("magick")
        .arg(&adobe).arg("-intent").arg("relative").arg("+black-point-compensation").arg("-profile").arg(dir.join("srgb.icc")).arg(&out)
        .output()
        .expect("run magick");
    assert!(status.status.success(), "{}", String::from_utf8_lossy(&status.stderr));

    let a = tiff_pixels(&out).3;
    let b = tiff_pixels(&srgb).3;
    // Both are 8 bit; ImageMagick may promote to 16: bring both to 0..255.
    let to8 = |v: &[u16]| -> Vec<f64> { let hi = v.iter().copied().max().unwrap_or(255) > 255; v.iter().map(|x| if hi { *x as f64 / 257.0 } else { *x as f64 }).collect() };
    let d = mean_abs_diff(to8(&a).into_iter(), to8(&b).into_iter());
    assert!(d < 1.5, "an independent engine disagrees by {d} levels on average");
}

#[test]
fn a_b_and_w_negative_exports_as_one_grey_channel() {
    let raw = nv_store::RawReader::read(&root().join("tests/fixtures/plustek8200i/bw-negative-glass-with-light.dng")).unwrap();
    let proxy = Proxy::from_raw(&raw, nv_process::DEFAULT_PROXY_PIXELS);
    let frozen = freeze(&proxy, &default_params_for(&builtin_profiles()[1]));
    let dir = scratch("grey");
    let s = ExportSettings { grey: true, ..Default::default() };
    let (w, h, ct, px, icc_bytes) = tiff_pixels(&export(&raw, &frozen, &s, &dir, "g.tif"));
    assert_eq!((w, h, ct), (2508, 1680, tiff::ColorType::Gray(16)));
    assert_eq!(icc_bytes.as_deref(), Some(&icc::grey_profile()[..]));
    assert_eq!(px.len(), 2508 * 1680);
    // A real picture: a wide spread of tones.
    let (lo, hi) = (px.iter().copied().min().unwrap(), px.iter().copied().max().unwrap());
    assert!(hi - lo > 30000, "{lo}..{hi}");
}

#[test]
fn cancelling_leaves_no_file_behind() {
    let (raw, frozen, _) = tulip();
    let dir = scratch("cancel");
    let dest = dir.join("c.tif");
    let cancel = AtomicBool::new(true);
    let r = export_image(&raw, &frozen, &ExportSettings::default(), &dest, &|_| {}, &cancel);
    assert!(matches!(r, Err(ExportError::Cancelled)));
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "left {:?}", std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect::<Vec<_>>());

    // Cancel after the first band: the partial file is removed too.
    let cancel = AtomicBool::new(false);
    let r = export_image(&raw, &frozen, &ExportSettings::default(), &dest, &|f| if f > 0.0 { cancel.store(true, Ordering::Relaxed) }, &cancel);
    assert!(matches!(r, Err(ExportError::Cancelled)));
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
}

#[test]
fn progress_rises_to_one() {
    let (raw, frozen, _) = tulip();
    let dir = scratch("progress");
    let seen = std::sync::Mutex::new(Vec::new());
    export_image(&raw, &frozen, &ExportSettings::default(), &dir.join("p.tif"), &|f| seen.lock().unwrap().push(f), &AtomicBool::new(false)).unwrap();
    let seen = seen.into_inner().unwrap();
    assert!(seen.len() >= 2 && seen.windows(2).all(|w| w[1] > w[0]) && (seen.last().unwrap() - 1.0).abs() < 1e-6, "{seen:?}");
}

#[test]
fn an_unwritable_folder_is_an_error_not_a_panic() {
    let (raw, frozen, _) = tulip();
    let r = export_image(&raw, &frozen, &ExportSettings::default(), Path::new("/nonexistent-dir/x.tif"), &|_| {}, &AtomicBool::new(false));
    assert!(matches!(r, Err(ExportError::Io(_))));
}

#[test]
fn raw_export_is_the_scan_byte_for_byte() {
    let dir = scratch("raw");
    let src = root().join("tests/fixtures/plustek8200i/tulip-600.pnm");
    let dest = dir.join("raw.pnm");
    let n = copy_raw(&src, &dest).unwrap();
    assert_eq!(std::fs::read(&src).unwrap(), std::fs::read(&dest).unwrap());
    assert_eq!(n, std::fs::metadata(&dest).unwrap().len());
    assert!(copy_raw(&src, Path::new("/nonexistent-dir/r.pnm")).is_err());
}

/// Peak resident memory of this process so far, in MB (Linux).
fn peak_rss_mb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmHWM:")).and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()))
        .map_or(0, |kb| kb / 1024)
}

/// DESIGN.md check H8 in miniature: a 7200 dpi frame (10368 × 7755, 482 MB
/// raw) exported as 16-bit TIFF must cost about one band of memory on top of
/// the raw itself. Run with
/// `cargo test -p nv-export --release --test export -- --ignored --nocapture`.
#[test]
#[ignore = "allocates 482 MB and writes about as much"]
fn a_7200_dpi_frame_exports_in_bounded_memory() {
    let (w, h) = (10368u32, 7755u32);
    let mut raw = FrameBuf::new(w, Some(h), 16, 3);
    let row: Vec<u8> = (0..w * 3).flat_map(|i| (((i * 37) % 40000 + 5000) as u16).to_ne_bytes()).collect();
    for _ in 0..h {
        raw.append(&row);
    }
    let proxy = Proxy::from_raw(&raw, nv_process::DEFAULT_PROXY_PIXELS);
    let frozen = freeze(&proxy, &default_params_for(&builtin_profiles()[0]));
    let before = peak_rss_mb();

    let dir = scratch("big");
    for (name, s) in [
        ("tiff16-deflate", ExportSettings::default()),
        ("tiff8-none", ExportSettings { bit_depth: 8, compression: Compression::None, ..Default::default() }),
    ] {
        let t = std::time::Instant::now();
        let r = export_image(&raw, &frozen, &s, &dir.join(format!("{name}.tif")), &|_| {}, &AtomicBool::new(false)).unwrap();
        eprintln!("{name}: {}x{} in {:?}, {} MB written, peak RSS {} MB (raw and proxy alone: {} MB)", r.width, r.height, t.elapsed(), r.bytes >> 20, peak_rss_mb(), before);
        let _ = std::fs::remove_file(&r.path);
    }
    let over = peak_rss_mb().saturating_sub(before);
    assert!(over < 1500, "the export added {over} MB on top of the raw");
}
