//! Temporary: render a raw PNM the way the window does -- with auto_tone --
//! so CLI output and app output can be told apart. No-op unless NV_T_IN is set.

use nv_film::builtin_profiles;
use nv_process::{Proxy, auto_tone, convert, default_params_for, estimate_film_base_proxy, render};

#[test]
fn autotone_harness() {
    let Ok(input) = std::env::var("NV_T_IN") else { return };
    let output = std::env::var("NV_T_OUT").expect("NV_T_OUT");
    let auto = std::env::var("NV_T_AUTO").unwrap_or_else(|_| "1".into()) == "1";

    let raw = nv_store::raw_io::RawReader::read(std::path::Path::new(&input)).expect("raw");
    let proxy = Proxy::from_raw(&raw, usize::MAX);
    let profiles = builtin_profiles();
    let idx: usize = std::env::var("NV_T_PROFILE").ok().and_then(|s| s.parse().ok()).unwrap_or(4);
    let profile = &profiles[idx];
    let mut params = default_params_for(profile);
    params.conversion.film_base = Some(estimate_film_base_proxy(&proxy));
    if auto {
        params.tone = auto_tone(&proxy, &params);
    }
    // Is there anything in the pixels the density clamp flattens? Compare the
    // raw transmission of the interior against the film base it is measured
    // from: everything at or above the base lands on density 0 together.
    {
        let base = params.conversion.film_base.unwrap();
        let (w, h) = (proxy.width, proxy.height);
        let (x0, x1, y0, y1) = (w / 10, w * 9 / 10, h / 10, h * 9 / 10);
        let mut over: Vec<f32> = Vec::new();
        let mut n = 0usize;
        for y in y0..y1 {
            for x in x0..x1 {
                let t = proxy.pixels[(y * w + x) * 3 + 1];
                n += 1;
                if t >= base[1] {
                    over.push(t / base[1]);
                }
            }
        }
        over.sort_by(|a, b| a.partial_cmp(b).unwrap());
        // Where does the estimate come from, and what is actually in the scan?
        let bw = (w / 12).max(4);
        let bh = (h / 12).max(4);
        let mut border: Vec<f32> = Vec::new();
        let mut inner: Vec<f32> = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let t = proxy.pixels[(y * w + x) * 3 + 1];
                if x < bw || x >= w - bw || y < bh || y >= h - bh {
                    border.push(t);
                } else {
                    inner.push(t);
                }
            }
        }
        border.sort_by(|a, b| a.partial_cmp(b).unwrap());
        inner.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let q = |v: &Vec<f32>, f: f64| v[((v.len() - 1) as f64 * f) as usize];
        eprintln!("border  p50 {:.4} p90 {:.4} p98 {:.4} p99.9 {:.4} max {:.4}", q(&border, 0.5), q(&border, 0.9), q(&border, 0.98), q(&border, 0.999), border[border.len() - 1]);
        eprintln!("picture p50 {:.4} p90 {:.4} p98 {:.4} p99.9 {:.4} max {:.4}", q(&inner, 0.5), q(&inner, 0.9), q(&inner, 0.98), q(&inner, 0.999), inner[inner.len() - 1]);
        eprintln!("film base (green) {:.5}", base[1]);
        eprintln!("interior at or above base: {:.2}% of {n}", 100.0 * over.len() as f64 / n as f64);
        if !over.is_empty() {
            let q = |f: f64| over[((over.len() - 1) as f64 * f) as usize];
            eprintln!("  t/base among them: p1 {:.3}  median {:.3}  p99 {:.3}  max {:.3}", q(0.01), q(0.5), q(0.99), over[over.len() - 1]);
        }
    }

    // What the conversion alone does, before any tone stage: saturation here
    // is data the tone stage never gets to see.
    {
        let neutral = nv_process::ProcessParams { tone: Default::default(), ..params.clone() };
        let conv = convert(&proxy, &neutral);
        let (w, h) = (proxy.width, proxy.height);
        let (x0, x1, y0, y1) = (w / 10, w * 9 / 10, h / 10, h * 9 / 10);
        let (mut lo, mut hi, mut n) = (0usize, 0usize, 0usize);
        for y in y0..y1 {
            for x in x0..x1 {
                for c in 0..3 {
                    let v = conv[(y * w + x) * 3 + c];
                    if v <= 0.0 {
                        lo += 1;
                    }
                    if v >= 1.0 {
                        hi += 1;
                    }
                    n += 1;
                }
            }
        }
        eprintln!("converted interior: at 0 {:.2}%, at 1 {:.2}%", 100.0 * lo as f64 / n as f64, 100.0 * hi as f64 / n as f64);
    }

    let rgb = render(&proxy, &params);
    let mut out = format!("P6\n{} {}\n255\n", proxy.width, proxy.height).into_bytes();
    out.extend_from_slice(&rgb);
    std::fs::write(&output, out).expect("write");
    eprintln!("wrote {output} auto={auto} black={:?} white={:?}", params.tone.black_point, params.tone.white_point);
}
