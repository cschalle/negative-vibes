//! The portal-backed `ScannerBackend` against libsane, through the same traits the app uses.
//!
//! Starts a private session bus (`dbus-daemon`) and the prototype service on the SANE test
//! backend, then scans with identical options both ways and compares what comes out. Also
//! the failure paths the worker relies on: cancel, no documents, busy.
//!
//! Skipped (with a message) when `dbus-daemon` is not installed.

use nv_sane::{OptionValue, Parameters, SaneError, ScannerBackend, ScannerHandle, Session};
use nv_sane_portal::PortalBackend;
use nv_sane_sys as sys;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn scan_all(h: &dyn ScannerHandle) -> Vec<(Parameters, Vec<u8>)> {
    let mut frames = Vec::new();
    loop {
        h.start().expect("start");
        let p = h.get_parameters().expect("parameters");
        let mut data = Vec::new();
        let mut buf = vec![0u8; 32 * 1024];
        loop {
            let n = h.read(&mut buf).expect("read");
            if n == 0 {
                break;
            }
            data.extend_from_slice(&buf[..n]);
        }
        let last = p.last_frame;
        frames.push((p, data));
        if last {
            return frames;
        }
    }
}

fn configure(h: &dyn ScannerHandle, opts: &[(&str, OptionValue)]) {
    // In the given order: the options depend on each other.
    for (name, value) in opts {
        h.set_option_by_name(name, value).unwrap_or_else(|e| panic!("set {name}: {e}")).unwrap_or_else(|| panic!("no option {name}"));
    }
}

fn code(e: &SaneError) -> Option<u32> {
    match e {
        SaneError::Status { code, .. } => Some(*code),
        _ => None,
    }
}

fn s(x: &str) -> OptionValue {
    OptionValue::String(x.into())
}

#[test]
fn portal_backend_matches_libsane() {
    let cfg = std::env::temp_dir().join(format!("nv-portal-proto-test-{}", std::process::id()));
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::write(cfg.join("dll.conf"), "test\n").unwrap();

    let Ok(mut bus) = Command::new("dbus-daemon").args(["--session", "--nofork", "--print-address"]).stdout(Stdio::piped()).spawn() else {
        eprintln!("skipped: dbus-daemon not found");
        return;
    };
    let mut address = String::new();
    BufReader::new(bus.stdout.take().unwrap()).read_line(&mut address).unwrap();
    let _bus = Kill(bus);

    // SAFETY: this is the only test in the binary, so nothing else reads the environment now.
    unsafe {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", address.trim());
        std::env::set_var("SANE_CONFIG_DIR", &cfg);
    }
    let _svc = Kill(Command::new(env!("CARGO_BIN_EXE_nv-portal-proto")).args(["--select", "test:"]).stderr(Stdio::null()).spawn().unwrap());
    // Wait for the service to own its name.
    let portal = (0..50)
        .find_map(|_| {
            std::thread::sleep(std::time::Duration::from_millis(100));
            PortalBackend::connect(nv_sane_portal::DEFAULT_BUS_NAME).ok()
        })
        .expect("service did not come up");
    let direct = Session::new().expect("sane_init");

    // Listing only lists: nothing is granted until the user selects.
    assert!(portal.get_devices(true).expect("portal devices").is_empty(), "listed before any selection");
    // Negative Vibes asks for film scanners only, and the test backend's virtual device is not one.
    // The portal answers "other" (2) when it has nothing to offer, which is also an empty selection.
    match portal.select_devices("") {
        Ok(chosen) => assert!(chosen.is_empty(), "a virtual device was offered as a film scanner"),
        Err(e) => assert!(e.to_string().contains("response 2"), "select: {e}"),
    }
    assert!(portal.get_devices(true).expect("portal devices").is_empty(), "granted although nothing was offered");
    let chosen = portal.select_scanners("", true, &[]).expect("select");
    assert!(!chosen.is_empty(), "the selection was empty");
    let devices = portal.get_devices(true).expect("portal devices");
    assert_eq!(devices.len(), chosen.len(), "list after select");
    let pid = devices[0].name.clone();
    // The id is opaque, yet quirks must still find the backend.
    assert_eq!(devices[0].backend, "test", "backend name through the portal ({})", devices[0].name);
    assert!(!devices[0].is_webcam());
    let direct_name = ScannerBackend::get_devices(&direct, true).unwrap().into_iter().find(|d| d.name.starts_with("test:")).expect("test device").name;

    // Same options, both ways, must give the same frames.
    type Case<'a> = (&'a str, Vec<(&'a str, OptionValue)>);
    let cases: Vec<Case> = vec![
        ("gray grid", vec![("resolution", OptionValue::Fixed(100.0)), ("test-picture", s("Grid"))]),
        // The app sets resolution as an integer; the test backend's option is fixed-point.
        ("int value for a fixed option", vec![("mode", s("Color")), ("resolution", OptionValue::Int(60)), ("test-picture", s("Color pattern"))]),
        ("rgb 8-bit", vec![("mode", s("Color")), ("resolution", OptionValue::Fixed(75.0)), ("test-picture", s("Color pattern"))]),
        ("rgb 16-bit", vec![("mode", s("Color")), ("depth", OptionValue::Int(16)), ("resolution", OptionValue::Fixed(50.0)), ("test-picture", s("Color pattern"))]),
        ("three-pass", vec![("mode", s("Color")), ("three-pass", OptionValue::Bool(true)), ("resolution", OptionValue::Fixed(50.0)), ("test-picture", s("Color pattern"))]),
    ];
    for (label, opts) in &cases {
        let via_portal = {
            let h = portal.open(&pid).expect("portal open");
            configure(h.as_ref(), opts);
            scan_all(h.as_ref())
        };
        let via_sane = {
            let h = ScannerBackend::open(&direct, &direct_name).expect("sane open");
            configure(h.as_ref(), opts);
            scan_all(h.as_ref())
        };
        assert_eq!(via_portal.len(), via_sane.len(), "{label}: frame count");
        for (i, ((pp, pd), (sp, sd))) in via_portal.iter().zip(&via_sane).enumerate() {
            assert_eq!(
                (&pp.format, pp.last_frame, pp.bytes_per_line, pp.pixels_per_line, pp.lines, pp.depth),
                (&sp.format, sp.last_frame, sp.bytes_per_line, sp.pixels_per_line, sp.lines, sp.depth),
                "{label}: frame {i} parameters"
            );
            assert!(pd == sd, "{label}: frame {i} pixel data differs ({} vs {} bytes)", pd.len(), sd.len());
        }
        eprintln!("ok: {label}: {} frame(s), identical", via_portal.len());
    }

    // The estimate before a frame equals the layout the frame reports, and goes stale-free
    // when an option changes.
    {
        let h = portal.open(&pid).unwrap();
        configure(h.as_ref(), &[("mode", s("Color")), ("three-pass", OptionValue::Bool(false)), ("resolution", OptionValue::Fixed(40.0))]);
        let before = h.get_parameters().expect("estimate");
        h.start().unwrap();
        let during = h.get_parameters().unwrap();
        let mut buf = vec![0u8; 64 * 1024];
        while h.read(&mut buf).unwrap() > 0 {}
        assert_eq!((before.pixels_per_line, before.lines, before.depth), (during.pixels_per_line, during.lines, during.depth));
        configure(h.as_ref(), &[("resolution", OptionValue::Fixed(80.0))]);
        let after = h.get_parameters().unwrap();
        assert!(after.pixels_per_line > during.pixels_per_line, "estimate must follow the new resolution, not the finished frame");
    }

    // Option descriptors survive the trip.
    {
        let h = portal.open(&pid).unwrap();
        let ours = h.all_options().unwrap();
        let theirs = ScannerBackend::open(&direct, &direct_name).unwrap().all_options().unwrap();
        for name in ["mode", "resolution", "depth", "test-picture", "tl-x", "br-y"] {
            let a = ours.iter().find(|d| d.name == name).unwrap_or_else(|| panic!("portal lacks {name}"));
            let b = theirs.iter().find(|d| d.name == name).unwrap();
            assert_eq!((&a.kind, &a.unit, a.is_settable()), (&b.kind, &b.unit, b.is_settable()), "{name}");
            assert_eq!(format!("{:?}", a.constraint), format!("{:?}", b.constraint), "{name}: constraint");
        }
    }

    // An option's automatic value, and the refusal for an option that has none, are the same
    // through the portal and through libsane.
    {
        let name = "bool-soft-select-soft-detect-auto";
        let h = portal.open(&pid).unwrap();
        let d = ScannerBackend::open(&direct, &direct_name).unwrap();
        for handle in [h.as_ref(), d.as_ref()] {
            configure(handle, &[("enable-test-options", OptionValue::Bool(true))]);
            let opts = handle.all_options().unwrap();
            let auto = opts.iter().find(|o| o.name == name).expect("test option with an automatic value");
            handle.set_option_auto(auto).unwrap_or_else(|e| panic!("set {name} to automatic: {e}"));
            let mode = opts.iter().find(|o| o.name == "mode").unwrap();
            let e = handle.set_option_auto(mode).expect_err("mode has no automatic value");
            assert_eq!(code(&e), Some(sys::SANE_STATUS_INVAL), "{e}");
        }
    }

    // A busy scanner is refused, and the SANE condition is what the worker expects.
    {
        let _held = portal.open(&pid).unwrap();
        let e = portal.open(&pid).err().expect("second open must fail");
        assert_eq!(code(&e), Some(sys::SANE_STATUS_DEVICE_BUSY), "{e}");
    }

    // Cancel mid-frame, with the app not reading, then scan again on the same handle.
    {
        let h = portal.open(&pid).unwrap();
        configure(h.as_ref(), &[("mode", s("Color")), ("resolution", OptionValue::Fixed(600.0))]);
        h.start().unwrap();
        let mut buf = vec![0u8; 4096];
        assert!(h.read(&mut buf).unwrap() > 0);
        std::thread::sleep(std::time::Duration::from_millis(500)); // the pipe fills
        let t = std::time::Instant::now();
        h.cancel();
        assert!(t.elapsed() < std::time::Duration::from_secs(5), "cancel took {:?}", t.elapsed());
        // The test backend keeps option state per device, not per handle: reset three-pass.
        configure(h.as_ref(), &[("three-pass", OptionValue::Bool(false)), ("resolution", OptionValue::Fixed(10.0))]);
        assert_eq!(scan_all(h.as_ref()).len(), 1, "a frame after a cancel");
        h.cancel(); // no frame running: must be a no-op
    }

    // Abandoning a multi-frame scan between frames (read the red frame, then cancel), then
    // scanning again: the portal must do whatever libsane does. (The SANE test backend keeps its
    // frame order across a cancel, so this cannot say what a real driver does; the service's unit
    // test covers that with a scanner that resets, like the 8200i.)
    {
        let sequence = |h: &dyn ScannerHandle| {
            configure(h, &[("mode", s("Color")), ("three-pass", OptionValue::Bool(true)), ("resolution", OptionValue::Fixed(20.0))]);
            h.start().unwrap();
            let first = h.get_parameters().unwrap().format;
            let mut buf = vec![0u8; 32 * 1024];
            while h.read(&mut buf).unwrap() > 0 {}
            h.cancel();
            let rest: Vec<_> = scan_all(h).iter().map(|(p, _)| p.format.clone()).collect();
            configure(h, &[("three-pass", OptionValue::Bool(false))]);
            (first, rest)
        };
        let via_portal = sequence(portal.open(&pid).unwrap().as_ref());
        let via_sane = sequence(ScannerBackend::open(&direct, &direct_name).unwrap().as_ref());
        assert_eq!(via_portal, via_sane, "a scan after an abandoned one");
    }

    // An empty feeder arrives as SANE_STATUS_NO_DOCS.
    {
        let h = portal.open(&pid).unwrap();
        configure(h.as_ref(), &[("source", s("Automatic Document Feeder")), ("resolution", OptionValue::Fixed(10.0))]);
        let mut seen = None;
        for _ in 0..12 {
            match h.start() {
                Ok(()) => {
                    let mut buf = vec![0u8; 32 * 1024];
                    while h.read(&mut buf).unwrap() > 0 {}
                }
                Err(e) => {
                    seen = Some(e);
                    break;
                }
            }
        }
        let e = seen.expect("the feeder should run out");
        assert_eq!(code(&e), Some(sys::SANE_STATUS_NO_DOCS), "{e}");
    }

    // A value the backend rejects is INVAL, not a transport error.
    {
        let h = portal.open(&pid).unwrap();
        let e = h.set_option_by_name("mode", &s("Nonsense")).expect_err("must be rejected");
        assert!(matches!(code(&e), Some(sys::SANE_STATUS_INVAL) | Some(sys::SANE_STATUS_UNSUPPORTED)), "{e}");
    }

    drop(portal);
    drop(direct);
    let _ = std::fs::remove_dir_all(&cfg);
}
