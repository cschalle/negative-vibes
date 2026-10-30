//! Temporary: render a raw PNM the way the window does -- with auto_tone --
//! so CLI output and app output can be told apart. No-op unless NV_T_IN is set.

use nv_film::builtin_profiles;
use nv_process::{Proxy, auto_tone, default_params_for, estimate_film_base_proxy, render};

#[test]
fn autotone_harness() {
    let Ok(input) = std::env::var("NV_T_IN") else { return };
    let output = std::env::var("NV_T_OUT").expect("NV_T_OUT");
    let auto = std::env::var("NV_T_AUTO").unwrap_or_else(|_| "1".into()) == "1";

    let raw = nv_store::raw_io::RawReader::read(std::path::Path::new(&input)).expect("raw");
    let proxy = Proxy::from_raw(&raw, usize::MAX);
    let profiles = builtin_profiles();
    let profile = profiles.iter().find(|p| p.id == "fuji-superia-400").expect("profile");
    let mut params = default_params_for(profile);
    params.conversion.film_base = Some(estimate_film_base_proxy(&proxy));
    if auto {
        params.tone = auto_tone(&proxy, &params);
    }
    let rgb = render(&proxy, &params);
    let mut out = format!("P6\n{} {}\n255\n", proxy.width, proxy.height).into_bytes();
    out.extend_from_slice(&rgb);
    std::fs::write(&output, out).expect("write");
    eprintln!("wrote {output} auto={auto} black={:?} white={:?}", params.tone.black_point, params.tone.white_point);
}
