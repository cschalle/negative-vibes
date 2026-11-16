use clap::Args;
use std::path::PathBuf;

#[derive(Args)]
pub struct CalibrateArgs {
    /// Raw scan of the IT8 target (the app's TIFF).
    #[arg(long)]
    scan: PathBuf,
    /// The target's measured values: the data file (.txt) for the slide's batch, or the
    /// maker's ZIP of all of them, in which case the batch that fits the scan is used.
    #[arg(long)]
    reference: PathBuf,
    /// With a ZIP: the batch printed on the slide (for example T100926). Without it the batch
    /// whose grey scale is closest to the scan is used, which is only a likelihood.
    #[arg(long)]
    batch: Option<String>,
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
    let is_zip = job.reference.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"));
    let r = if is_zip {
        let bytes = std::fs::read(&job.reference)?;
        let (outcome, ranked) = nv_calibrate::batch::calibrate_from_zip(&job, &bytes, a.batch.as_deref())?;
        if a.batch.is_none() {
            println!("No --batch given: using the batch whose grey scale is closest to the scan, which is a likelihood. Read the code printed on the slide to be sure.");
        }
        if !ranked.is_empty() {
            println!("Batches tried: {}", ranked.len());
        }
        for (i, b) in ranked.iter().take(5).enumerate() {
            println!("  {}{}  grey-scale residual {:.2}%", if i == 0 { "closest: " } else { "         " }, b.name, b.residual);
        }
        outcome
    } else {
        nv_calibrate::calibrate(&job)?
    };
    println!("Profile: {}", r.profile.display());
    println!("Patches: {:?}  average error: {:?}  peak error: {:?}", r.patches, r.average_error, r.peak_error);
    Ok(())
}
