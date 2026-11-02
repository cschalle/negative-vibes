//! The aperture detector on the real scans in `tests/fixtures`, where the
//! holder and the rebate are whatever the scanner actually produced.

use std::path::{Path, PathBuf};

use nv_process::{DEFAULT_PROXY_PIXELS, Proxy, aperture};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A reduced proxy must never crop *more* than the full scan would.
///
/// A proxy pixel on the boundary mixes the mount with the picture, so it reads
/// brighter than the mount and is kept: the measurement gets looser as the
/// reduction grows, never tighter. That is the safe direction, and it is the
/// property worth pinning -- the editor and the export agree with each other
/// because they measure the same proxy at the same budget, not because a proxy
/// matches the scan to the pixel.
#[test]
fn a_reduced_proxy_never_cuts_more_than_the_scan_itself() {
    for name in ["tulip-600.pnm", "tulip-600-repeat.pnm"] {
        let path = root().join("tests/fixtures/plustek8200i").join(name);
        let Ok(raw) = nv_store::RawReader::read(&path) else {
            eprintln!("no {name}: nothing checked");
            continue;
        };
        let full = aperture::picture_window(&raw).expect("a window from the scan");
        let (w, h) = (raw.width, raw.effective_height());
        assert!(full[3] - full[1] + 1 < h, "{name}: nothing was cut off the height of {h}");
        println!("{name}: {w}x{h} -> {full:?}");

        // The editor's own budget, and one small enough to force the reduction
        // a 3600 dpi scan actually gets.
        for budget in [DEFAULT_PROXY_PIXELS, 40_000] {
            let proxy = Proxy::from_raw(&raw, budget);
            let got = aperture::picture_window_of(&proxy).expect("a window from the proxy");
            println!("  factor {}: {got:?}", proxy.factor);
            assert!(got[0] <= full[0] && got[1] <= full[1], "{name} at factor {}: {got:?} starts inside {full:?}", proxy.factor);
            assert!(got[2] >= full[2] && got[3] >= full[3], "{name} at factor {}: {got:?} ends inside {full:?}", proxy.factor);
            // Still a crop, not a quiet giving-up on the whole frame.
            assert!(got[3] - got[1] + 1 < h, "{name} at factor {}: nothing cut", proxy.factor);
        }
    }
}
