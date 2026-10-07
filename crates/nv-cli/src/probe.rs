use clap::Args;
use nv_device::{DeviceCapabilities, Quirks};

#[derive(Args)]
pub struct ProbeArgs {
    /// Include network SANE backends (slower).
    #[arg(long)]
    network: bool,
    /// Path to devices.toml quirks file.
    #[arg(long, default_value = "data/devices.toml")]
    quirks: std::path::PathBuf,
    /// Show all SANE option descriptors for each device.
    #[arg(long)]
    verbose: bool,
}

pub fn run(args: ProbeArgs) -> anyhow::Result<()> {
    let quirks = Quirks::load(&args.quirks);
    let session = nv_sane::Session::new().map_err(|e| anyhow::anyhow!("sane_init: {e}"))?;
    let devices = session
        .get_devices(!args.network)
        .map_err(|e| anyhow::anyhow!("sane_get_devices: {e}"))?;

    if devices.is_empty() {
        println!("No SANE devices found.");
        return Ok(());
    }

    for dev in &devices {
        let handle = match session.open(&dev.name) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("  [cannot open: {e}]");
                continue;
            }
        };

        let backend = nv_device::quirks::backend_name(&dev.name);
        let qk = quirks.find(backend, &dev.model);
        let caps = DeviceCapabilities::detect(&dev.name, &dev.vendor, &dev.model, &handle, qk)
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        print_caps(&caps);

        if args.verbose {
            println!("  Options:");
            let opts = handle.all_options().map_err(|e| anyhow::anyhow!("{e}"))?;
            for opt in opts {
                if !opt.is_active() { continue; }
                println!("    [{}] {}  ({:?}) cap={:#010x}", opt.index, opt.name, opt.kind, opt.cap);
                if let Ok(val) = handle.get_option(&opt) {
                    println!("        value = {val:?}");
                }
            }
        }
    }

    Ok(())
}

fn print_caps(caps: &DeviceCapabilities) {
    println!("─── {} ───", caps.device_name);
    println!("  Vendor/Model : {} {}", caps.vendor, caps.model);
    println!("  Film source  : {}", caps.film_source_value.as_deref().unwrap_or("(device-only)"));
    println!("  IR           : {:?}", caps.ir);
    println!("  IR registered: {}", caps.ir_registered);
    println!("  Depths       : {:?}", caps.depths);
    println!("  Resolutions  : {:?}", caps.resolutions);
    if let Some([tlx, tly, brx, bry]) = caps.scan_area_mm {
        println!("  Scan area    : {tlx}–{brx} × {tly}–{bry} mm");
    }
    if let Some([w, h]) = caps.film_window_mm {
        println!("  Film window  : {w} × {h} mm");
    }
    if let Some(r) = caps.recommended_dpi {
        println!("  Rec. DPI     : {r}");
    }
    println!();
}

