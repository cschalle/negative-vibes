use std::fs::File;
use std::io::Write as IoWrite;
use std::path::PathBuf;

use clap::Args;
use nv_film::builtin_profiles;
use nv_process::{
    estimate_film_base, process,
    params::{ConversionParams, ImageType, ToneParams},
    ProcessParams,
};
use nv_store::RawReader;

#[derive(Args)]
pub struct RenderArgs {
    /// Raw PNM input file (from `nv-cli scan`).
    pub input: PathBuf,

    /// Output PPM file path.
    #[arg(long, short)]
    pub output: Option<PathBuf>,

    /// Image type: colour-negative | bw-negative | positive
    #[arg(long, default_value = "colour-negative")]
    pub image_type: String,

    /// Built-in film profile id (e.g. kodak-portra-400, generic-colour-negative).
    #[arg(long, default_value = "generic-colour-negative")]
    pub profile: String,

    /// Override film base as R,G,B in [0,1] (e.g. 0.62,0.31,0.17).
    /// If omitted, auto-estimated from border pixels.
    #[arg(long)]
    pub film_base: Option<String>,

    /// Exposure offset in stops.
    #[arg(long, default_value = "0.0")]
    pub exposure: f32,

    /// Black point [0,1].
    #[arg(long, default_value = "0.0")]
    pub black_point: f32,

    /// White point [0,1].
    #[arg(long, default_value = "1.0")]
    pub white_point: f32,

    /// Midtone gamma (1.0 = neutral).
    #[arg(long, default_value = "1.0")]
    pub midtone: f32,

    /// Print per-channel statistics, film base estimate and first bytes.
    #[arg(long)]
    pub diagnose: bool,
}

pub fn run(args: RenderArgs) -> anyhow::Result<()> {
    let buf = RawReader::read(&args.input)
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {e}", args.input.display()))?;

    let w = buf.width;
    let h = buf.effective_height();
    let bpr = buf.bytes_per_row;
    println!(
        "Input: {}×{} depth={} ch={} bytes_per_row={} ({})",
        w, h, buf.depth, buf.channels, bpr, args.input.display()
    );

    // Always print first raw bytes so we can spot obvious corruption.
    let raw = buf.as_bytes();
    let preview_len = 24.min(raw.len());
    let first_bytes: Vec<String> = raw[..preview_len]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    println!("First {} raw bytes: {}", preview_len, first_bytes.join(" "));

    // Per-channel statistics on the raw scan (big-endian 16-bit).
    let bps = buf.depth as usize / 8;
    let ch = buf.channels as usize;
    let n_pixels = (w * h) as usize;
    let mut sums = [0f64; 3];
    let mut mins = [f64::MAX; 3];
    let mut maxs = [0f64; 3];
    let scale = if bps == 2 { 65535.0f64 } else { 255.0f64 };

    for y in 0..h as usize {
        for x in 0..w as usize {
            let off = y * bpr + x * ch * bps;
            for c in 0..3.min(ch) {
                let v = read_be(raw, off + c * bps, bps) as f64 / scale;
                sums[c] += v;
                mins[c] = mins[c].min(v);
                maxs[c] = maxs[c].max(v);
            }
        }
    }

    println!("Raw channel stats (linear [0,1]):");
    for c in 0..3.min(ch) {
        let mean = sums[c] / n_pixels as f64;
        println!("  {}: min={:.4} max={:.4} mean={:.4}", ["R","G","B"][c], mins[c], maxs[c], mean);
    }

    // Film base estimate from borders.
    let estimated_base = estimate_film_base(&buf);
    println!(
        "Estimated film base: R={:.4} G={:.4} B={:.4}",
        estimated_base[0], estimated_base[1], estimated_base[2]
    );

    if args.diagnose {
        // Print more detail about border vs interior pixel stats.
        let border_w = (w as usize / 12).max(4);
        let border_h = (h as usize / 12).max(4);
        let mut border_sums = [0f64; 3];
        let mut border_n = 0usize;
        let mut interior_sums = [0f64; 3];
        let mut interior_n = 0usize;

        for y in 0..h as usize {
            for x in 0..w as usize {
                let off = y * bpr + x * ch * bps;
                let is_border = x < border_w || x >= w as usize - border_w
                    || y < border_h || y >= h as usize - border_h;
                for c in 0..3.min(ch) {
                    let v = read_be(raw, off + c * bps, bps) as f64 / scale;
                    if is_border { border_sums[c] += v; border_n += 1; }
                    else { interior_sums[c] += v; interior_n += 1; }
                }
            }
        }
        let bn = (border_n / 3).max(1) as f64;
        let intn = (interior_n / 3).max(1) as f64;
        println!("Border  mean: R={:.4} G={:.4} B={:.4} ({border_n} samples)",
            border_sums[0]/bn, border_sums[1]/bn, border_sums[2]/bn);
        println!("Interior mean: R={:.4} G={:.4} B={:.4} ({interior_n} samples)",
            interior_sums[0]/intn, interior_sums[1]/intn, interior_sums[2]/intn);
    }

    // Build ProcessParams.
    let profiles = builtin_profiles();
    let profile = profiles
        .iter()
        .find(|p| p.id == args.profile)
        .cloned()
        .unwrap_or_else(|| {
            eprintln!("Profile '{}' not found; using generic-colour-negative", args.profile);
            profiles[0].clone()
        });

    let image_type = match args.image_type.as_str() {
        "bw-negative" | "bw" | "grey" | "gray" => ImageType::BwNegative,
        "positive" | "slide" | "reversal" => ImageType::Positive,
        _ => ImageType::ColourNegative,
    };

    let film_base = if let Some(s) = &args.film_base {
        let parts: Vec<f32> = s
            .split(',')
            .filter_map(|v| v.trim().parse().ok())
            .collect();
        if parts.len() == 3 {
            println!("Using manual film base: R={} G={} B={}", parts[0], parts[1], parts[2]);
            Some([parts[0], parts[1], parts[2]])
        } else {
            eprintln!("--film-base must be R,G,B — using auto estimate");
            Some(estimated_base)
        }
    } else {
        println!("Using auto-estimated film base");
        Some(estimated_base)
    };

    let params = ProcessParams {
        image_type,
        profile: profile.clone(),
        conversion: ConversionParams {
            film_base,
            exposure: args.exposure,
            tolerance: 0.0016,
            auto_ccr: true,
        },
        tone: ToneParams {
            black_point: args.black_point,
            white_point: args.white_point,
            midtone: args.midtone,
            contrast: 0.0,
        },
    };

    println!(
        "Processing: type={:?} profile='{}' base={:?} exposure={}",
        params.image_type, profile.name, params.conversion.film_base, args.exposure
    );

    let rgb8 = process(&buf, &params);

    // Output stats.
    let out_sums: [f64; 3] = {
        let mut s = [0f64; 3];
        for i in 0..n_pixels {
            s[0] += rgb8[i * 3] as f64;
            s[1] += rgb8[i * 3 + 1] as f64;
            s[2] += rgb8[i * 3 + 2] as f64;
        }
        s
    };
    println!("Output channel means (0-255):");
    for c in 0..3 {
        println!("  {}: {:.1}", ["R","G","B"][c], out_sums[c] / n_pixels as f64);
    }

    // Write output PPM.
    let out_path = args.output.unwrap_or_else(|| {
        let stem = args.input.file_stem().unwrap_or_default().to_string_lossy();
        PathBuf::from(format!("{stem}-rendered.ppm"))
    });

    let mut f = File::create(&out_path)?;
    writeln!(f, "P6\n{w} {h}\n255")?;
    f.write_all(&rgb8)?;
    println!("Saved rendered image to {}", out_path.display());

    // Also write raw-to-rgb8 (sqrt gamma, no pipeline) for comparison.
    let raw_rgb = buf.to_rgb8();
    let raw_out = {
        let stem = out_path.file_stem().unwrap_or_default().to_string_lossy();
        out_path.with_file_name(format!("{stem}-raw.ppm"))
    };
    let mut f2 = File::create(&raw_out)?;
    writeln!(f2, "P6\n{w} {h}\n255")?;
    f2.write_all(&raw_rgb)?;
    println!("Saved raw-gamma preview to {}", raw_out.display());

    Ok(())
}

fn read_be(data: &[u8], offset: usize, bps: usize) -> u32 {
    if offset + bps > data.len() { return 0; }
    if bps == 1 { data[offset] as u32 }
    else { u16::from_be_bytes([data[offset], data[offset + 1]]) as u32 }
}
