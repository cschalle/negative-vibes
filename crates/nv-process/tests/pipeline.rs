//! Pipeline behaviour that does not need a real scan: the other image types,
//! the render cache, and what a 7200 dpi frame costs.

use nv_film::builtin_profiles;
use nv_image::FrameBuf;
use nv_process::{ImageType, ProcessParams, Proxy, RenderCache, default_params_for, render, render_cached, render_detailed};

/// A horizontal ramp, `w × h`, of linear transmission from `lo` to `hi`
/// (the same in all three channels), with a bright rebate frame around it.
fn ramp(w: usize, h: usize, lo: f32, hi: f32) -> Proxy {
    let mut px = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let border = x < w / 10 || x >= w - w / 10 || y < h / 10 || y >= h - h / 10;
            let t = if border { hi } else { lo + (hi - lo) * (x - w / 10) as f32 / (w - 2 * (w / 10)) as f32 };
            px.extend_from_slice(&[t, t, t]);
        }
    }
    Proxy::new(w, h, 1, px)
}

fn params_for(profile: usize) -> ProcessParams {
    default_params_for(&builtin_profiles()[profile])
}

fn middle_row_grey(rgb: &[u8], w: usize, h: usize) -> Vec<u8> {
    (w / 5..w * 4 / 5).map(|x| rgb[((h / 2) * w + x) * 3 + 1]).collect()
}

#[test]
fn a_bw_negative_comes_out_grey_and_denser_is_brighter() {
    let (w, h) = (200, 100);
    let p = ramp(w, h, 0.02, 0.85); // film: thin on the right, dense on the left? lo..hi along x
    let mut params = params_for(1);
    assert_eq!(params.image_type, ImageType::BwNegative);
    params.conversion.film_base = Some([0.85; 3]);
    let rgb = render(&p, &params);

    // Grey: R = G = B everywhere.
    assert!(rgb.chunks(3).all(|px| px[0] == px[1] && px[1] == px[2]), "B&W output has colour");
    // Transmission rises along the ramp, so the positive falls: dense film is bright.
    let row = middle_row_grey(&rgb, w, h);
    assert!(row.first().unwrap() > row.last().unwrap(), "{} vs {}", row.first().unwrap(), row.last().unwrap());
    assert!(row.windows(2).all(|p| p[1] <= p[0] + 1), "not monotone");
    assert!(*row.first().unwrap() as i32 - *row.last().unwrap() as i32 > 80, "ramp squashed");
}

#[test]
fn a_positive_is_normalised_and_keeps_its_order() {
    let (w, h) = (200, 100);
    let p = ramp(w, h, 0.05, 0.6);
    let params = params_for(2);
    assert_eq!(params.image_type, ImageType::Positive);
    let rgb = render(&p, &params);
    let row = middle_row_grey(&rgb, w, h);
    // Brighter transmission is brighter in the positive, and the range is used.
    assert!(row.windows(2).all(|p| p[1] + 1 >= p[0]), "not monotone");
    assert!(
        row.last().unwrap() > &200 && row.first().unwrap() < &130 && *row.last().unwrap() as i32 - *row.first().unwrap() as i32 > 90,
        "{} .. {}",
        row.first().unwrap(),
        row.last().unwrap()
    );
}

#[test]
fn tone_changes_reuse_the_conversion_and_conversion_changes_do_not() {
    let (w, h) = (120, 80);
    let p = ramp(w, h, 0.05, 0.7);
    let mut params = params_for(0);
    params.conversion.film_base = Some([0.7; 3]);
    let cache = RenderCache::default();

    let first = render_cached(&p, &params, &cache);
    assert_eq!(first.rgb, render_detailed(&p, &params).rgb, "cached result differs");
    assert_eq!(cache.conversions(), 1);

    params.tone.black_point = 0.1;
    params.tone.balance = [1.1, 1.0, 0.9];
    let toned = render_cached(&p, &params, &cache);
    assert_eq!(cache.conversions(), 1, "a tone edit converted again");
    assert_ne!(toned.rgb, first.rgb);
    assert_eq!(toned.rgb, render_detailed(&p, &params).rgb, "reused conversion gives another picture");

    params.conversion.exposure = 0.5;
    render_cached(&p, &params, &cache);
    assert_eq!(cache.conversions(), 2);

    // Another picture with the same parameters is not the same conversion.
    let other = ramp(w, h, 0.05, 0.7);
    render_cached(&other, &params, &cache);
    assert_eq!(cache.conversions(), 3);
}

/// A 7200 dpi frame is 10368 × 7755 px, 482 MB as 16-bit RGB (DESIGN.md §1).
/// Run with `cargo test -p nv-process --release -- --ignored --nocapture`.
#[test]
#[ignore = "allocates 482 MB"]
fn a_7200_dpi_frame_is_reduced_once_and_then_renders_interactively() {
    let (w, h) = (10368u32, 7755u32);
    let mut raw = FrameBuf::new(w, Some(h), 16, 3);
    let row: Vec<u8> = (0..w * 3).flat_map(|i| (((i * 37) % 40000 + 5000) as u16).to_ne_bytes()).collect();
    for _ in 0..h {
        raw.append(&row);
    }

    let t = std::time::Instant::now();
    let proxy = Proxy::from_raw(&raw, nv_process::DEFAULT_PROXY_PIXELS);
    let reduce = t.elapsed();
    assert!(proxy.width * proxy.height <= nv_process::DEFAULT_PROXY_PIXELS);

    let params = params_for(0);
    let cache = RenderCache::default();
    let t = std::time::Instant::now();
    render_cached(&proxy, &params, &cache);
    let first = t.elapsed();
    let mut toned = params.clone();
    toned.tone.midtone = 1.3;
    let t = std::time::Instant::now();
    render_cached(&proxy, &toned, &cache);
    let tone_only = t.elapsed();
    eprintln!("{w}x{h} -> {}x{}: reduce {reduce:?}, first render {first:?}, tone-only render {tone_only:?}", proxy.width, proxy.height);
    assert!(tone_only.as_millis() < 250, "tone-only render took {tone_only:?}");

    // Full resolution, the way 1:1 view and export do it.
    let frozen = nv_process::tiles::freeze(&proxy, &params);
    let mut tiles = nv_process::tiles::TileCache::new(512 << 20);
    let t = std::time::Instant::now();
    let view = tiles.region(&raw, &frozen, 4000, 3000, 1600, 1000);
    let first_view = t.elapsed();
    assert_eq!(view.len(), 1600 * 1000 * 3);
    let t = std::time::Instant::now();
    tiles.region(&raw, &frozen, 4100, 3100, 1600, 1000);
    let panned = t.elapsed();
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get());
    let t = std::time::Instant::now();
    let full = nv_process::tiles::render_full(&raw, &frozen, threads);
    let export = t.elapsed();
    assert_eq!(full.len(), w as usize * h as usize * 3);
    eprintln!("1:1 view {first_view:?}, panned {panned:?}, full export on {threads} threads {export:?}");
    assert!(first_view.as_millis() < 1500, "a 1:1 view took {first_view:?}");
}
