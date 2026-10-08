//! Exports of real scans, read back with independent decoders.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use nv_export::{copy_raw, export_image, icc, ColourSpace, Compression, ExportError, ExportFormat, ExportSettings, OutputSize};
use nv_film::builtin_profiles;
use nv_image::FrameBuf;
use nv_process::{default_params_for, render, tiles::freeze, Orientation, ProcessParams, Proxy};

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
    // These tests are about encoders and colour, and compare an export with a
    // whole-picture render sample for sample; dust removal measures each tile
    // for itself, so it is tested on its own below.
    let mut params = default_params_for(&builtin_profiles()[0]);
    params.dust.mode = nv_process::DustMode::Off;
    let frozen = freeze(&proxy, &params);
    let reference = render(&proxy, &frozen);
    (raw, frozen, reference)
}

fn export(raw: &FrameBuf, p: &ProcessParams, s: &ExportSettings, dir: &Path, name: &str) -> PathBuf {
    let dest = dir.join(name);
    export_image(raw, p, Orientation::IDENTITY, s, &dest, &|_| {}, &AtomicBool::new(false)).unwrap_or_else(|e| panic!("{name}: {e}"));
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

fn export_oriented(raw: &FrameBuf, p: &ProcessParams, o: Orientation, s: &ExportSettings, dir: &Path, name: &str) -> PathBuf {
    let dest = dir.join(name);
    export_image(raw, p, o, s, &dest, &|_| {}, &AtomicBool::new(false)).unwrap_or_else(|e| panic!("{name}: {e}"));
    dest
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
    let r = export_image(&raw, &frozen, Orientation::IDENTITY, &ExportSettings::default(), &dest, &|_| {}, &cancel);
    assert!(matches!(r, Err(ExportError::Cancelled)));
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "left {:?}", std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect::<Vec<_>>());

    // Cancel after the first band: the partial file is removed too.
    let cancel = AtomicBool::new(false);
    let r = export_image(&raw, &frozen, Orientation::IDENTITY, &ExportSettings::default(), &dest, &|f| if f > 0.0 { cancel.store(true, Ordering::Relaxed) }, &cancel);
    assert!(matches!(r, Err(ExportError::Cancelled)));
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
}

#[test]
fn progress_rises_to_one() {
    let (raw, frozen, _) = tulip();
    let dir = scratch("progress");
    let seen = std::sync::Mutex::new(Vec::new());
    export_image(&raw, &frozen, Orientation::IDENTITY, &ExportSettings::default(), &dir.join("p.tif"), &|f| seen.lock().unwrap().push(f), &AtomicBool::new(false)).unwrap();
    let seen = seen.into_inner().unwrap();
    assert!(seen.len() >= 2 && seen.windows(2).all(|w| w[1] > w[0]) && (seen.last().unwrap() - 1.0).abs() < 1e-6, "{seen:?}");
}

#[test]
fn an_unwritable_folder_is_an_error_not_a_panic() {
    let (raw, frozen, _) = tulip();
    let r = export_image(&raw, &frozen, Orientation::IDENTITY, &ExportSettings::default(), Path::new("/nonexistent-dir/x.tif"), &|_| {}, &AtomicBool::new(false));
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

    // The same frame with sharpening and noise reduction on: the cost of the detail stage.
    let mut detailed = frozen.clone();
    detailed.detail.usm = nv_process::UsmParams { enabled: true, amount: 150.0, radius: 1.4, threshold: 0 };
    detailed.detail.noise = nv_process::NoiseLevel::Medium;
    let dir = scratch("big");
    {
        let t = std::time::Instant::now();
        let r = export_image(&raw, &detailed, Orientation::IDENTITY, &ExportSettings::default(), &dir.join("detail.tif"), &|_| {}, &AtomicBool::new(false)).unwrap();
        eprintln!("tiff16-deflate with sharpening and noise reduction: {}x{} in {:?}, peak RSS {} MB", r.width, r.height, t.elapsed(), peak_rss_mb());
        let _ = std::fs::remove_file(&r.path);
    }
    for (name, s) in [
        ("tiff16-deflate", ExportSettings::default()),
        ("tiff8-none", ExportSettings { bit_depth: 8, compression: Compression::None, ..Default::default() }),
    ] {
        let t = std::time::Instant::now();
        let r = export_image(&raw, &frozen, Orientation::IDENTITY, &s, &dir.join(format!("{name}.tif")), &|_| {}, &AtomicBool::new(false)).unwrap();
        eprintln!("{name}: {}x{} in {:?}, {} MB written, peak RSS {} MB (raw and proxy alone: {} MB)", r.width, r.height, t.elapsed(), r.bytes >> 20, peak_rss_mb(), before);
        let _ = std::fs::remove_file(&r.path);
    }
    let over = peak_rss_mb().saturating_sub(before);
    assert!(over < 1500, "the export added {over} MB on top of the raw");
}

#[test]
fn an_oriented_export_is_the_upright_export_turned_exactly() {
    let (raw, frozen, _) = tulip();
    let dir = scratch("orient");
    let s = ExportSettings { compression: Compression::None, ..Default::default() };
    let (uw, uh, _, upright, _) = tiff_pixels(&export(&raw, &frozen, &s, &dir, "up.tif"));
    for (i, o) in [
        Orientation { rotation: 1, ..Orientation::IDENTITY },
        Orientation { rotation: 2, ..Orientation::IDENTITY },
        Orientation { rotation: 3, ..Orientation::IDENTITY },
        Orientation { flip_h: true, ..Orientation::IDENTITY },
        Orientation { flip_v: true, ..Orientation::IDENTITY },
        Orientation { rotation: 1, flip_h: true, flip_v: false },
    ]
    .into_iter()
    .enumerate()
    {
        let (w, h, _, px, _) = tiff_pixels(&export_oriented(&raw, &frozen, o, &s, &dir, &format!("o{i}.tif")));
        let (ew, eh, expected) = o.apply(&upright, uw as usize, uh as usize, 3);
        assert_eq!((w as usize, h as usize), (ew, eh), "{o:?}");
        assert!(px == expected, "{o:?}: the pixels differ from the turned upright export");
    }
}

#[test]
fn a_smaller_export_has_the_size_asked_and_the_same_look() {
    let (raw, frozen, reference) = tulip();
    let dir = scratch("resize");
    for (size, want) in [
        (OutputSize::LongEdge { pixels: 300 }, (300, 224)),
        (OutputSize::Percent { percent: 25.0 }, (216, 161)),
        (OutputSize::Fit { width: 400, height: 100 }, (134, 100)),
        (OutputSize::LongEdge { pixels: 5000 }, (864, 645)),
    ] {
        let s = ExportSettings { size, bit_depth: 8, compression: Compression::Lzw, ..Default::default() };
        let (w, h, _, px, _) = tiff_pixels(&export(&raw, &frozen, &s, &dir, "r.tif"));
        assert_eq!((w, h), want, "{size:?}");
        assert_eq!(px.len(), w as usize * h as usize * 3);
        // Averaging keeps the picture's brightness.
        let mean = |v: &mut dyn Iterator<Item = f64>| { let (s, n) = v.fold((0.0, 0.0), |(s, n), x| (s + x, n + 1.0)); s / n };
        let (a, b) = (mean(&mut px.iter().map(|v| *v as f64)), mean(&mut reference.iter().map(|v| *v as f64)));
        assert!((a - b).abs() < 3.0, "{size:?}: mean {a} vs {b}");
    }
}

/// An independent resampler (ImageMagick, area-weighted) agrees with ours on
/// what a downscaled picture looks like. Skipped when it is not installed.
#[test]
fn an_independent_resampler_agrees_on_the_look_of_a_smaller_picture() {
    if std::process::Command::new("magick").arg("-version").output().is_err() {
        eprintln!("ImageMagick not installed: skipping");
        return;
    }
    let (raw, frozen, _) = tulip();
    let dir = scratch("magick-resize");
    let full = export(&raw, &frozen, &ExportSettings { bit_depth: 8, compression: Compression::None, ..Default::default() }, &dir, "full.tif");
    let small = export(&raw, &frozen, &ExportSettings { bit_depth: 8, compression: Compression::None, size: OutputSize::LongEdge { pixels: 288 }, ..Default::default() }, &dir, "small.tif");
    let theirs = dir.join("theirs.tif");
    let out = std::process::Command::new("magick").arg(&full).args(["-filter", "box", "-resize", "288x215!", "-depth", "8"]).arg(&theirs).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let (_, _, _, a, _) = tiff_pixels(&small);
    let (_, _, _, b, _) = tiff_pixels(&theirs);
    assert_eq!(a.len(), b.len());
    // Ours averages linear light and theirs averages the encoded values, so
    // edges differ a little; the two must still be the same picture.
    let d = mean_abs_diff(a.iter().map(|v| *v as f64), b.iter().map(|v| *v as f64));
    assert!(d < 3.0, "ImageMagick's downscale differs by {d} levels on average");
}

#[test]
fn rotated_and_smaller_together() {
    let (raw, frozen, _) = tulip();
    let dir = scratch("both");
    let s = ExportSettings { size: OutputSize::LongEdge { pixels: 300 }, bit_depth: 8, ..Default::default() };
    let (w, h, _, px, _) = tiff_pixels(&export_oriented(&raw, &frozen, Orientation { rotation: 1, ..Orientation::IDENTITY }, &s, &dir, "b.tif"));
    assert_eq!((w, h), (224, 300), "rotated first, then sized by the picture's long edge");
    assert_eq!(px.len(), 224 * 300 * 3);
}

#[test]
fn every_format_handles_a_resized_picture() {
    let (raw, frozen, _) = tulip();
    let dir = scratch("formats");
    let size = OutputSize::Percent { percent: 40.0 };
    let jpg = export(&raw, &frozen, &ExportSettings { format: ExportFormat::Jpeg, size, ..Default::default() }, &dir, "a.jpg");
    let info = { let mut d = jpeg_decoder::Decoder::new(std::fs::File::open(&jpg).unwrap()); d.read_info().unwrap(); d.info().unwrap() };
    assert_eq!((info.width, info.height), (346, 258));
    let png_path = export(&raw, &frozen, &ExportSettings { format: ExportFormat::Png, bit_depth: 16, size, ..Default::default() }, &dir, "a.png");
    let r = png::Decoder::new(std::fs::File::open(&png_path).unwrap()).read_info().unwrap();
    assert_eq!((r.info().width, r.info().height), (346, 258));
}

/// The tulip scan has an infrared plane, and exporting it removes the dust
/// found there: the export differs from the dust-free one, and agrees with
/// the whole-picture render with dust removal on, to within what tiling does.
#[test]
fn an_export_of_a_scan_with_infrared_removes_dust() {
    let raw = nv_store::RawReader::read(&root().join("tests/fixtures/plustek8200i/tulip-600.pnm")).unwrap();
    assert_eq!(raw.channels, 4);
    let proxy = Proxy::from_raw(&raw, usize::MAX);
    let frozen = freeze(&proxy, &default_params_for(&builtin_profiles()[0]));
    assert_eq!(frozen.dust.mode, nv_process::DustMode::Correct);
    let dir = scratch("dust");
    let s = ExportSettings { bit_depth: 8, compression: Compression::None, ..Default::default() };
    let (_, _, _, px, _) = tiff_pixels(&export(&raw, &frozen, &s, &dir, "d.tif"));
    let whole = render(&proxy, &frozen);
    let far = px.iter().zip(&whole).filter(|(a, b)| (**a as i32 - **b as i32).abs() > 8).count();
    assert!((far as f64) < 0.01 * whole.len() as f64, "{:.2}% of samples differ by more than 8 levels", 100.0 * far as f64 / whole.len() as f64);
    let mut off = frozen.clone();
    off.dust.mode = nv_process::DustMode::Off;
    let (_, _, _, undusted, _) = tiff_pixels(&export(&raw, &off, &s, &dir, "n.tif"));
    assert!(px != undusted, "the export did not remove any dust");
}
