//! Temporary: render one raw through a chosen film profile at a chosen cast
//! removal strength, so the two can be told apart. No-op unless NV_P_IN is set.
//!
//! `NV_P_IN` (absolute), `NV_P_OUT`, `NV_P_PROFILE` (index into
//! `builtin_profiles`), `NV_P_CAST` (0..1), `NV_P_AUTO` (1 = auto tone).

use nv_film::builtin_profiles;
use nv_process::{Proxy, auto_tone, default_params_for, estimate_film_base_proxy, render};

#[test]
fn profile_harness() {
    let Ok(input) = std::env::var("NV_P_IN") else { return };
    let output = std::env::var("NV_P_OUT").expect("NV_P_OUT");
    let idx: usize = std::env::var("NV_P_PROFILE").ok().and_then(|s| s.parse().ok()).unwrap_or(4);
    let cast: f32 = std::env::var("NV_P_CAST").ok().and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let auto = std::env::var("NV_P_AUTO").unwrap_or_else(|_| "1".into()) == "1";

    let raw = nv_store::raw_io::RawReader::read(std::path::Path::new(&input)).expect("raw");
    let proxy = Proxy::from_raw(&raw, usize::MAX);
    let profiles = builtin_profiles();
    let profile = &profiles[idx];
    let mut params = default_params_for(profile);
    params.conversion.film_base = Some(estimate_film_base_proxy(&proxy));
    params.conversion.cast_removal = cast;
    if auto {
        params.tone = auto_tone(&proxy, &params);
    }

    let rgb = render(&proxy, &params);
    let mut out = format!("P6\n{} {}\n255\n", proxy.width, proxy.height).into_bytes();
    out.extend_from_slice(&rgb);
    std::fs::write(&output, out).expect("write");
    eprintln!("profile[{idx}] {} / {}  cast={cast}  -> {output}", profile.vendor, profile.name);
}
