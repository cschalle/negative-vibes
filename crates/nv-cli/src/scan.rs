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

    /// Scan only part of the window: `x0,y0,x1,y1` as fractions of it
    /// (for example `0.25,0.2,0.75,0.8`).
    #[arg(long, value_parser = parse_area)]
    area: Option<[f64; 4]>,

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

fn parse_area(s: &str) -> Result<[f64; 4], String> {
    let v: Vec<f64> = s.split(',').map(|p| p.trim().parse::<f64>().map_err(|e| format!("{p:?}: {e}"))).collect::<Result<_, _>>()?;
    match v[..] {
        [a, b, c, d] if (0.0..=1.0).contains(&a) && (0.0..=1.0).contains(&c) && (0.0..=1.0).contains(&b) && (0.0..=1.0).contains(&d) && a < c && b < d => Ok([a, b, c, d]),
        _ => Err("expected x0,y0,x1,y1 with 0 <= x0 < x1 <= 1 and 0 <= y0 < y1 <= 1".into()),
    }
}

pub fn run(args: ScanArgs) -> anyhow::Result<()> {
    let worker = AcquireWorker::spawn(args.quirks.clone());
    worker.probe();

    let device_name = wait_for_device(&worker.evt_rx, args.device.as_deref())?;
    println!("Scanning on {device_name} @ {} dpi (ir={})…", args.dpi, args.ir);

    worker.start_scan(device_name, args.dpi, args.ir, args.area);

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
            dust: "off".into(), // a scan from this command has no infrared plane
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

    // PNM stores 16-bit samples most significant byte first, whatever the
    // host's order, so the raw buffer is converted on the way out. Writing
    // it unswapped produced a file that claimed to be a 16-bit PNM but no
    // other tool could read.
    let emit = |f: &mut File, row: &[u8]| -> std::io::Result<()> {
        if buf.depth > 8 {
            let be: Vec<u8> = row
                .chunks_exact(2)
                .flat_map(|p| u16::from_ne_bytes([p[0], p[1]]).to_be_bytes())
                .collect();
            f.write_all(&be)
        } else {
            f.write_all(row)
        }
    };

    for row in 0..h as usize {
        let start = row * bpr_actual;
        emit(&mut f, &raw[start..start + bpr_packed])?;
    }

    println!("Saved {w}×{h} {}-bit raw image to {}", buf.depth, output.display());
    Ok(buf)
}
