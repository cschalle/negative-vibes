use std::fs::File;
use std::io::Write as IoWrite;
use std::path::PathBuf;

use clap::Args;
use nv_acquire::{AcquireWorker, ScanEvent};
use nv_image::FrameBuf;

#[derive(Args)]
pub struct ScanArgs {
    /// SANE device name (e.g. `plustek8200i:libusb:003:007`).
    /// If omitted the first device found is used.
    device: Option<String>,

    /// Scan resolution in DPI.
    #[arg(long, default_value = "600")]
    dpi: u32,

    /// Also capture the infrared channel.
    #[arg(long)]
    ir: bool,

    /// Output file path (.pnm for raw data).
    #[arg(long, short, default_value = "scan.pnm")]
    output: PathBuf,

    /// Path to devices.toml quirks file.
    #[arg(long, default_value = "data/devices.toml")]
    quirks: PathBuf,

    /// After scanning, immediately run through the nv-process pipeline and
    /// save a rendered PPM alongside the raw PNM.
    #[arg(long)]
    render: bool,
}

pub fn run(args: ScanArgs) -> anyhow::Result<()> {
    let worker = AcquireWorker::spawn(args.quirks.clone());
    worker.probe();

    let device_name = wait_for_device(&worker.evt_rx, args.device.as_deref())?;
    println!("Scanning on {device_name} @ {} dpi (ir={})…", args.dpi, args.ir);

    worker.start_scan(device_name, args.dpi, args.ir);

    let buf = collect_scan(&worker.evt_rx, &args.output)?;

    worker.quit();

    if args.render {
        let stem = args.output.file_stem().unwrap_or_default().to_string_lossy();
        let rendered_path = args.output.with_file_name(format!("{stem}-rendered.ppm"));
        let render_args = crate::render::RenderArgs {
            input: args.output.clone(),
            output: Some(rendered_path),
            image_type: "colour-negative".into(),
            profile: "generic-colour-negative".into(),
            film_base: None,
            exposure: 0.0,
            black_point: 0.0,
            white_point: 1.0,
            midtone: 1.0,
            diagnose: true,
        };
        // Build a FrameBuf from the already-received data so we don't re-read disk.
        // Also run render from the saved PNM to verify the round-trip.
        println!("\n--- Pipeline render ---");
        drop(buf); // buf was saved to disk; render reads it back via RawReader
        crate::render::run(render_args)?;
    }

    Ok(())
}

fn wait_for_device(
    rx: &async_channel::Receiver<ScanEvent>,
    preferred: Option<&str>,
) -> anyhow::Result<String> {
    loop {
        let event = rx.recv_blocking().map_err(|_| anyhow::anyhow!("worker channel closed"))?;
        match event {
            ScanEvent::DeviceList(caps) => {
                if caps.is_empty() {
                    return Err(anyhow::anyhow!("No SANE devices found"));
                }
                let chosen = match preferred {
                    Some(name) => caps
                        .iter()
                        .find(|c| c.device_name == name)
                        .ok_or_else(|| anyhow::anyhow!("device '{name}' not found"))?,
                    None => caps.first().unwrap(),
                };
                return Ok(chosen.device_name.clone());
            }
            ScanEvent::Error(e) => return Err(anyhow::anyhow!("probe error: {e}")),
            _ => {}
        }
    }
}

/// Collect a scan into a FrameBuf and write the raw PNM to disk.
/// Strips any backend scanline padding so the PNM is tightly packed.
fn collect_scan(
    rx: &async_channel::Receiver<ScanEvent>,
    output: &PathBuf,
) -> anyhow::Result<FrameBuf> {
    let mut buf_opt: Option<FrameBuf> = None;

    loop {
        let event = rx.recv_blocking().map_err(|_| anyhow::anyhow!("worker channel closed"))?;
        match event {
            ScanEvent::ScanStarted { width_px, height_px, bytes_per_line, depth: d, channels: ch } => {
                let packed = width_px as usize * ch as usize * (d as usize / 8);
                let padded = bytes_per_line != packed;
                println!(
                    "  Started: {width_px}×{height_px:?} {d}-bit {ch}ch \
                     bytes_per_line={bytes_per_line} (packed={packed}{})",
                    if padded { " ← PADDED!" } else { "" }
                );
                buf_opt = Some(FrameBuf::with_stride(width_px, height_px, d, ch, bytes_per_line));
            }
            ScanEvent::DataChunk { data, .. } => {
                if let Some(buf) = buf_opt.as_mut() {
                    buf.append(&data);
                }
            }
            ScanEvent::Progress(p) => {
                print!("\r  {:.0}%   ", p * 100.0);
                std::io::stdout().flush().ok();
            }
            ScanEvent::ScanDone => {
                println!("\r  Done.  ");
                break;
            }
            ScanEvent::Error(e) => return Err(anyhow::anyhow!("scan error: {e}")),
            ScanEvent::Cancelled => return Err(anyhow::anyhow!("scan cancelled")),
            _ => {}
        }
    }

    let buf = buf_opt.ok_or_else(|| anyhow::anyhow!("ScanStarted event never received"))?;
    let w = buf.width;
    let h = buf.effective_height();
    let bpr_actual = buf.bytes_per_row;
    let bps = buf.depth as usize / 8;
    let ch = buf.channels as usize;
    let bpr_packed = w as usize * ch * bps;
    let raw = buf.as_bytes();

    println!(
        "Received {} bytes — {}×{} rows×{} bpr",
        raw.len(), h, bpr_actual,
        if bpr_actual == bpr_packed { "(packed)" } else { "(stripping padding)" }
    );

    // Write PNM — always tightly packed (strip padding if present).
    let mut f = File::create(output)?;
    let magic = if ch == 1 { "P5" } else { "P6" };
    let maxval = if buf.depth > 8 { 65535u32 } else { 255u32 };
    writeln!(f, "{magic}\n{w} {h}\n{maxval}")?;

    if bpr_actual == bpr_packed {
        f.write_all(&raw[..bpr_actual * h as usize])?;
    } else {
        // Strip per-row padding.
        for row in 0..h as usize {
            f.write_all(&raw[row * bpr_actual..row * bpr_actual + bpr_packed])?;
        }
    }

    println!("Saved {w}×{h} {}-bit raw image to {}", buf.depth, output.display());
    Ok(buf)
}
