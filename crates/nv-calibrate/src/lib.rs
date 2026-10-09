//! IT8 calibration (F§11, DESIGN §7.6): a raw scan of an IT8 target becomes
//! an input ICC profile for the scanner. The heavy lifting (finding the
//! patches, fitting the profile) is ArgyllCMS's `scanin` and `colprof`, run as
//! subprocesses; this crate prepares their input, runs them, and reads the
//! result back.

use std::path::{Path, PathBuf};
use std::process::Command;

use nv_process::InputProfile;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("ArgyllCMS (scanin, colprof) was not found; install it to calibrate")]
    NoArgyll,
    #[error("{tool} failed: {message}")]
    Tool { tool: &'static str, message: String },
    #[error("{0}")]
    Input(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Directories searched after `PATH` for the Argyll tools.
const EXTRA_DIRS: [&str; 3] = ["/usr/bin", "/usr/local/bin", "/opt/argyll/bin"];

/// Where a tool lives, if installed.
pub fn find_tool(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path).chain(EXTRA_DIRS.iter().map(PathBuf::from)).map(|d| d.join(name)).find(|p| p.is_file())
}

/// True when both tools are installed.
pub fn available() -> bool {
    find_tool("scanin").is_some() && find_tool("colprof").is_some()
}

/// Where Argyll keeps its chart recognition files, for the user to pick from.
pub fn chart_dirs() -> Vec<PathBuf> {
    ["/usr/share/color/argyll/ref", "/usr/local/share/color/argyll/ref", "/usr/share/argyllcms/ref"].iter().map(PathBuf::from).filter(|p| p.is_dir()).collect()
}

/// The standard IT8.7 chart layout, if Argyll's data is installed.
pub fn default_chart() -> Option<PathBuf> {
    chart_dirs().into_iter().map(|d| d.join("it8.cht")).find(|p| p.is_file())
}

/// What to calibrate from.
#[derive(Debug, Clone)]
pub struct Job {
    /// The raw scan of the target (the app's linear 16-bit TIFF).
    pub scan: PathBuf,
    /// Chart layout (`.cht`), e.g. Argyll's `it8.cht`.
    pub chart: PathBuf,
    /// The target's measured values: the manufacturer's data file for this
    /// target (CGATS, as supplied with it).
    pub reference: PathBuf,
    /// Where to put the profile (`<name>.icc`) and the files it came from.
    pub out_dir: PathBuf,
    /// Profile and file name stem.
    pub name: String,
    /// The four chart corners as pixel coordinates, when the chart cannot be
    /// found by itself: `[x1,y1, x2,y2, x3,y3, x4,y4]` (top-left, top-right,
    /// bottom-right, bottom-left of the patch area, as the chart file lists
    /// its fiducials).
    pub corners: Option<[f64; 8]>,
}

#[derive(Debug, Clone)]
pub struct Outcome {
    pub profile: PathBuf,
    /// The profile as the pipeline uses it.
    pub input: InputProfile,
    /// How well the profile reproduces the target, in ΔE as colprof reports it.
    pub average_error: Option<f64>,
    pub peak_error: Option<f64>,
    pub patches: Option<usize>,
}

/// Write the scan as the plain 3-channel 16-bit TIFF `scanin` reads (ours may
/// carry a fourth, infrared, sample).
pub fn write_scanin_tiff(scan: &Path, out: &Path) -> Result<()> {
    use tiff::encoder::{TiffEncoder, colortype};
    let buf = nv_store::RawReader::read(scan).map_err(|e| Error::Input(format!("reading {}: {e}", scan.display())))?;
    let (w, h) = (buf.width as usize, buf.effective_height().min(buf.rows_available()) as usize);
    if buf.depth != 16 || buf.channels < 3 || w == 0 || h == 0 {
        return Err(Error::Input("the scan must be a 16-bit colour scan".into()));
    }
    let (ch, bpr) = (buf.channels as usize, buf.bytes_per_row);
    let bytes = buf.as_bytes();
    let mut data = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let o = y * bpr + x * ch * 2;
            for c in 0..3 {
                data.push(u16::from_ne_bytes([bytes[o + c * 2], bytes[o + c * 2 + 1]]));
            }
        }
    }
    let file = std::fs::File::create(out)?;
    let mut enc = TiffEncoder::new(std::io::BufWriter::new(file)).map_err(|e| Error::Input(e.to_string()))?;
    enc.write_image::<colortype::RGB16>(w as u32, h as u32, &data).map_err(|e| Error::Input(e.to_string()))?;
    Ok(())
}

fn run(tool: &'static str, args: &[std::ffi::OsString], dir: &Path) -> Result<String> {
    let exe = find_tool(tool).ok_or(Error::NoArgyll)?;
    let out = Command::new(exe).args(args).current_dir(dir).output()?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if !out.status.success() {
        let last = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no message");
        return Err(Error::Tool { tool, message: last.trim().to_owned() });
    }
    Ok(text)
}

/// `Profile check complete, peak err = 1.2, avg err = 0.4, RMS = 0.5` and the
/// number of patches read, from colprof's report.
pub fn parse_report(text: &str) -> (Option<f64>, Option<f64>, Option<usize>) {
    let after = |line: &str, key: &str| -> Option<f64> {
        let i = line.find(key)? + key.len();
        line[i..].trim_start().split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).next()?.parse().ok()
    };
    let (mut avg, mut peak, mut n) = (None, None, None);
    for line in text.lines() {
        if line.contains("peak err") {
            peak = after(line, "peak err =");
            avg = after(line, "avg err =");
        }
        if let Some(i) = line.find("Number of patches") {
            n = line[i..].split(|c: char| !c.is_ascii_digit()).find(|s| !s.is_empty()).and_then(|s| s.parse().ok());
        }
    }
    (avg, peak, n)
}

/// Run the whole calibration.
pub fn calibrate(job: &Job) -> Result<Outcome> {
    if !available() {
        return Err(Error::NoArgyll);
    }
    for (what, p) in [("chart file", &job.chart), ("reference data", &job.reference), ("scan", &job.scan)] {
        if !p.is_file() {
            return Err(Error::Input(format!("the {what} {} does not exist", p.display())));
        }
    }
    std::fs::create_dir_all(&job.out_dir)?;
    let stem = job.name.trim();
    if stem.is_empty() || stem.contains(['/', '\\']) {
        return Err(Error::Input("the profile needs a plain name".into()));
    }
    let tif = job.out_dir.join(format!("{stem}.tif"));
    write_scanin_tiff(&job.scan, &tif)?;

    let mut args: Vec<std::ffi::OsString> = vec!["-v".into()];
    if let Some(c) = job.corners {
        args.push("-F".into());
        args.push(c.iter().map(|v| format!("{v}")).collect::<Vec<_>>().join(",").into());
    }
    args.extend([tif.clone().into_os_string(), job.chart.clone().into_os_string(), job.reference.clone().into_os_string()]);
    let scanin = run("scanin", &args, &job.out_dir)?;
    let ti3 = job.out_dir.join(format!("{stem}.ti3"));
    if !ti3.is_file() {
        return Err(Error::Tool { tool: "scanin", message: "it produced no patch values".into() });
    }

    // A matrix and curves profile: the form the pipeline can apply.
    let colprof = run("colprof", &["-v".into(), "-qm".into(), "-as".into(), "-D".into(), stem.into(), job.out_dir.join(stem).into_os_string()], &job.out_dir)?;
    let profile = job.out_dir.join(format!("{stem}.icc"));
    let bytes = std::fs::read(&profile)?;
    let input = InputProfile::from_icc(stem, &bytes).map_err(|e| Error::Tool { tool: "colprof", message: e.to_string() })?;
    let (average_error, peak_error, patches) = parse_report(&format!("{scanin}\n{colprof}"));
    Ok(Outcome { profile, input, average_error, peak_error, patches })
}

/// Where profiles are kept: `$XDG_DATA_HOME/negative-vibes/profiles`.
pub fn profiles_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("negative-vibes/profiles")
}

/// Names of the profiles built so far, sorted.
pub fn list_profiles() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(profiles_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension().is_some_and(|x| x == "icc")).then(|| p.file_stem()?.to_str().map(str::to_owned)).flatten()
        })
        .collect();
    names.sort();
    names
}

/// Read one of those profiles for the pipeline.
pub fn load_profile(name: &str) -> Result<InputProfile> {
    if name.contains(['/', '\\']) {
        return Err(Error::Input("not a profile name".into()));
    }
    let bytes = std::fs::read(profiles_dir().join(format!("{name}.icc")))?;
    InputProfile::from_icc(name, &bytes).map_err(|e| Error::Input(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colprof_report_is_read() {
        let text = "Number of patches = 264\nProfile check complete, peak err = 4.5, avg err = 1.25, RMS = 1.6\n";
        assert_eq!(parse_report(text), (Some(1.25), Some(4.5), Some(264)));
        assert_eq!(parse_report("nothing"), (None, None, None));
    }

    #[test]
    fn bad_jobs_are_refused_before_running_anything() {
        let job = Job {
            scan: "/nonexistent.tif".into(),
            chart: "/nonexistent.cht".into(),
            reference: "/nonexistent.txt".into(),
            out_dir: std::env::temp_dir(),
            name: "x".into(),
            corners: None,
        };
        let e = calibrate(&job);
        assert!(matches!(e, Err(Error::Input(_)) | Err(Error::NoArgyll)), "{e:?}");
    }
}
