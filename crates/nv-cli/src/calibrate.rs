use clap::Args;
use std::path::PathBuf;

#[derive(Args)]
pub struct CalibrateArgs {
    /// Raw scan of the IT8 target (the app's TIFF).
    #[arg(long)]
    scan: PathBuf,
    /// The target's measured values (the data file that came with it).
    #[arg(long)]
    reference: PathBuf,
    /// Chart layout; Argyll's it8.cht when omitted.
    #[arg(long)]
    chart: Option<PathBuf>,
    /// Profile name.
    #[arg(long)]
    name: String,
    /// Chart corners in pixels, when it is not found by itself: x1,y1,x2,y2,x3,y3,x4,y4
    #[arg(long, value_delimiter = ',')]
    corners: Option<Vec<f64>>,
    /// Folder for the profile; the profile folder when omitted.
    #[arg(long)]
    out: Option<PathBuf>,
}

pub fn run(a: CalibrateArgs) -> anyhow::Result<()> {
    let chart = a.chart.or_else(nv_calibrate::default_chart).ok_or_else(|| anyhow::anyhow!("no chart file: pass --chart"))?;
    let corners = match a.corners {
        Some(c) => Some(<[f64; 8]>::try_from(c).map_err(|_| anyhow::anyhow!("--corners needs eight numbers"))?),
        None => None,
    };
    let job = nv_calibrate::Job { scan: a.scan, chart, reference: a.reference, out_dir: a.out.unwrap_or_else(nv_calibrate::profiles_dir), name: a.name, corners };
    let r = nv_calibrate::calibrate(&job)?;
    println!("Profile: {}", r.profile.display());
    println!("Patches: {:?}  average error: {:?}  peak error: {:?}", r.patches, r.average_error, r.peak_error);
    Ok(())
}
