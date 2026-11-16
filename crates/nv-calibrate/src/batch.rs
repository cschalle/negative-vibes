//! Finding which batch of a target a slide is from. Makers of IT8 targets
//! measure each batch and ship one data file per batch, often all in one ZIP,
//! and the batch is printed small on the slide. It matters: on a real slide
//! (GWB Ektachrome, batch T100926) a profile built from another batch was
//! 7 to 16 dE off the right one's data, which scored 1.5.
//!
//! The scan can say which batch it is, but not by fitting a profile to each
//! batch's values: a matrix profile bends to fit nearly any smooth data, and
//! the wrong batch fitted better than the right one. What a profile cannot
//! bend is the grey scale. A scanner is linear in light, so each channel must
//! be a straight line against the batch's X, Y and Z over the grey steps, and
//! only the batch that was really measured makes it so. On that slide the
//! right batch had the smallest residual, 1.4 times better than the next. It is
//! one slide, so the result is a hint for the person to confirm against the
//! code printed on the target, not a decision.

use std::collections::HashMap;
use std::path::Path;

use crate::zip::Zip;
use crate::{Error, Job, Outcome, Result, calibrate, run, write_scanin_tiff};

/// One batch's measured values.
#[derive(Debug, Clone)]
pub struct Candidate {
    /// The batch, from the data file's name (`T100926`).
    pub name: String,
    pub text: String,
}

/// How well one batch's grey scale matches what the scanner read.
#[derive(Debug, Clone, PartialEq)]
pub struct Ranked {
    pub name: String,
    /// Relative RMS residual of the grey steps against the scan, per cent: smaller is closer.
    pub residual: f64,
}

/// The batch a person typed, as printed on the slide: any case, with or
/// without spaces, and without the first letter when what is left is
/// unambiguous (`100926` for `T100926`).
pub fn find_batch<'a>(candidates: &'a [Candidate], typed: &str) -> Option<&'a Candidate> {
    let key = |s: &str| s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase();
    let want = key(typed);
    if want.is_empty() {
        return None;
    }
    if let Some(c) = candidates.iter().find(|c| key(&c.name) == want) {
        return Some(c);
    }
    let mut partial = candidates.iter().filter(|c| want.len() >= 4 && key(&c.name).ends_with(&want));
    match (partial.next(), partial.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The data files in a maker's ZIP: `.txt` files, in `.txt.zip` files inside it
/// or directly. Slide targets are kept when the ZIP separates them from the
/// reflective ones, since a slide is not one of those.
pub fn candidates_from_zip(bytes: &[u8]) -> Result<Vec<Candidate>> {
    let outer = Zip::parse(bytes).map_err(Error::Input)?;
    let mut found: Vec<(String, Candidate)> = Vec::new();
    let mut add = |path: &str, text: Vec<u8>| {
        let base = file_name(path);
        let name = base.strip_suffix(".txt").or_else(|| base.strip_suffix(".TXT")).unwrap_or(base).to_owned();
        if let Ok(text) = String::from_utf8(text).or_else(|e| Ok::<_, ()>(String::from_utf8_lossy(e.as_bytes()).into_owned()))
            && parse_reference(&text).is_some_and(|m| m.len() >= 24)
            && !found.iter().any(|(_, c)| c.name == name)
        {
            found.push((path.to_owned(), Candidate { name, text }));
        }
    };
    for path in outer.names().filter(|p| !p.contains("__MACOSX") && !file_name(p).starts_with("._")) {
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".txt") {
            if let Ok(t) = outer.read(path) {
                add(path, t);
            }
        } else if lower.ends_with(".zip")
            && let Ok(inner_bytes) = outer.read(path)
            && let Ok(inner) = Zip::parse(&inner_bytes)
        {
            for p in inner.names().filter(|p| p.to_ascii_lowercase().ends_with(".txt") && !file_name(p).starts_with("._")) {
                if let Ok(t) = inner.read(p) {
                    add(path.trim_end_matches(".zip").trim_end_matches(".ZIP"), t);
                }
            }
        }
    }
    if found.iter().any(|(p, _)| p.to_ascii_lowercase().contains("transmissive")) {
        found.retain(|(p, _)| p.to_ascii_lowercase().contains("transmissive"));
    }
    if found.is_empty() {
        return Err(Error::Input("there are no target data files (.txt) in that ZIP file".into()));
    }
    Ok(found.into_iter().map(|(_, c)| c).collect())
}

/// `A1` and `A01`, `GS0` and `GS00`, are the same patch.
fn patch_id(id: &str) -> String {
    let digits = id.find(|c: char| c.is_ascii_digit()).unwrap_or(id.len());
    match id[digits..].parse::<u32>() {
        Ok(n) if digits > 0 => format!("{}{n:02}", &id[..digits]),
        _ => id.to_owned(),
    }
}

/// Patch id to its X, Y and Z as written, from a CGATS data file.
pub fn parse_reference(text: &str) -> Option<HashMap<String, [String; 3]>> {
    let text = text.replace('\r', "\n");
    let mut lines = text.lines().map(str::trim_end);
    lines.by_ref().find(|l| l.trim() == "BEGIN_DATA_FORMAT")?;
    let fields: Vec<&str> = lines.next()?.split_whitespace().collect();
    let col = |name: &str| fields.iter().position(|f| *f == name);
    let (id, x, y, z) = (col("SAMPLE_ID")?, col("XYZ_X")?, col("XYZ_Y")?, col("XYZ_Z")?);
    lines.by_ref().find(|l| l.trim() == "BEGIN_DATA")?;
    let mut out = HashMap::new();
    for l in lines {
        if l.trim() == "END_DATA" {
            break;
        }
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() == fields.len() {
            out.insert(patch_id(f[id]), [f[x].to_owned(), f[y].to_owned(), f[z].to_owned()]);
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Solve the weighted least squares `y ≈ A·x` for four unknowns by the normal equations.
fn lstsq4(a: &[[f64; 4]], y: &[f64], w: &[f64]) -> Option<[f64; 4]> {
    let mut m = [[0.0f64; 5]; 4];
    for ((row, y), w) in a.iter().zip(y).zip(w) {
        for i in 0..4 {
            for j in 0..4 {
                m[i][j] += w * w * row[i] * row[j];
            }
            m[i][4] += w * w * row[i] * y;
        }
    }
    // X, Y and Z of a grey scale are nearly proportional, so the equations are nearly singular:
    // a negligible ridge keeps them solvable without changing the fit.
    let ridge = 1e-9 * (m[0][0] + m[1][1] + m[2][2] + m[3][3]);
    for (i, row) in m.iter_mut().enumerate() {
        row[i] += ridge;
    }
    for c in 0..4 {
        let p = (c..4).max_by(|&i, &j| m[i][c].abs().total_cmp(&m[j][c].abs()))?;
        if m[p][c].abs() < 1e-12 {
            return None;
        }
        m.swap(c, p);
        for r in 0..4 {
            if r != c {
                let f = m[r][c] / m[c][c];
                let pivot = m[c];
                for (v, p) in m[r].iter_mut().zip(pivot).skip(c) {
                    *v -= f * p;
                }
            }
        }
    }
    Some([m[0][4] / m[0][0], m[1][4] / m[1][1], m[2][4] / m[2][2], m[3][4] / m[3][3]])
}

/// Patch id to the scanner's R, G and B (0 to 1), from scanin's output.
fn scanned_values(ti3: &str) -> Option<HashMap<String, [f64; 3]>> {
    let mut lines = ti3.lines();
    lines.by_ref().find(|l| l.trim() == "BEGIN_DATA_FORMAT")?;
    let fields: Vec<&str> = lines.next()?.split_whitespace().collect();
    let col = |name: &str| fields.iter().position(|f| *f == name);
    let (id, r, g, b) = (col("SAMPLE_ID")?, col("RGB_R")?, col("RGB_G")?, col("RGB_B")?);
    lines.by_ref().find(|l| l.trim() == "BEGIN_DATA")?;
    let mut out = HashMap::new();
    for l in lines.take_while(|l| l.trim() != "END_DATA") {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() == fields.len() {
            let v = |i: usize| f[i].parse::<f64>().ok().map(|x| x / 100.0);
            out.insert(patch_id(f[id]), [v(r)?, v(g)?, v(b)?]);
        }
    }
    Some(out)
}

/// The grey steps used: all but the last two, which sit on the scanner's black floor.
fn grey_steps() -> Vec<String> {
    (0..22).map(|i| format!("GS{i:02}")).collect()
}

/// How far a batch's grey scale is from the scan's, in per cent (see the module notes).
fn grey_residual(scan: &HashMap<String, [f64; 3]>, reference: &HashMap<String, [String; 3]>) -> Option<f64> {
    let mut rows = Vec::new();
    let mut seen = Vec::new();
    for id in grey_steps() {
        let (rgb, xyz) = (scan.get(&id)?, reference.get(&id)?);
        let v: Vec<f64> = xyz.iter().map(|s| s.parse::<f64>().ok()).collect::<Option<_>>()?;
        rows.push([v[0] / 100.0, v[1] / 100.0, v[2] / 100.0, 1.0]);
        seen.push(*rgb);
    }
    let mut sum = 0.0;
    for c in 0..3 {
        let y: Vec<f64> = seen.iter().map(|v| v[c]).collect();
        let w: Vec<f64> = y.iter().map(|v| 1.0 / v.max(1e-3)).collect();
        let x = lstsq4(&rows, &y, &w)?;
        for ((row, y), w) in rows.iter().zip(&y).zip(&w) {
            let fit: f64 = row.iter().zip(x).map(|(a, b)| a * b).sum();
            sum += ((fit - y) * w).powi(2);
        }
    }
    Some((sum / (3 * rows.len()) as f64).sqrt() * 100.0)
}

/// Rank the batches by how well their grey scale matches the scan, closest first.
/// The patches are found once; each batch then costs a few multiplications.
pub fn rank(job: &Job, candidates: &[Candidate]) -> Result<Vec<Ranked>> {
    let first = candidates.first().ok_or_else(|| Error::Input("there are no batches to try".into()))?;
    for (what, p) in [("chart file", &job.chart), ("scan", &job.scan)] {
        if !p.is_file() {
            return Err(Error::Input(format!("the {what} {} does not exist", p.display())));
        }
    }
    std::fs::create_dir_all(&job.out_dir)?;
    let work = job.out_dir.join(".batch-search");
    std::fs::create_dir_all(&work)?;
    let result = rank_in(job, candidates, first, &work);
    let _ = std::fs::remove_dir_all(&work);
    result
}

fn rank_in(job: &Job, candidates: &[Candidate], first: &Candidate, work: &Path) -> Result<Vec<Ranked>> {
    let tif = work.join("target.tif");
    write_scanin_tiff(&job.scan, &tif)?;
    let probe = work.join("probe.txt");
    std::fs::write(&probe, &first.text)?;
    let mut args: Vec<std::ffi::OsString> = vec!["-v".into()];
    if let Some(c) = job.corners {
        args.push("-F".into());
        args.push(c.iter().map(|v| format!("{v}")).collect::<Vec<_>>().join(",").into());
    }
    args.extend([tif.into_os_string(), job.chart.clone().into_os_string(), probe.into_os_string()]);
    run("scanin", &args, work)?;
    let ti3 = std::fs::read_to_string(work.join("target.ti3")).map_err(|_| Error::Tool { tool: "scanin", message: "it produced no patch values".into() })?;
    let scan = scanned_values(&ti3).ok_or_else(|| Error::Tool { tool: "scanin", message: "its patch values could not be read".into() })?;
    let mut ranked: Vec<Ranked> = candidates.iter().filter_map(|c| Some(Ranked { name: c.name.clone(), residual: grey_residual(&scan, &parse_reference(&c.text)?)? })).collect();
    if ranked.is_empty() {
        return Err(Error::Input("those data files have no grey scale (GS00 to GS21) to compare with the scan".into()));
    }
    ranked.sort_by(|a, b| a.residual.total_cmp(&b.residual));
    Ok(ranked)
}

/// Calibrate from a maker's ZIP of data files, with the batch `batch` the
/// person read off the slide. Without one, the batch whose grey scale is
/// closest to the scan is used, which is a likelihood and not a fact. The
/// ranking is returned when it was made. `job.reference` is not read: the
/// batch's file is written next to the profile.
pub fn calibrate_from_zip(job: &Job, zip: &[u8], batch: Option<&str>) -> Result<(Outcome, Vec<Ranked>)> {
    let candidates = candidates_from_zip(zip)?;
    let (chosen, ranked) = match batch {
        Some(name) => (find_batch(&candidates, name).ok_or_else(|| Error::Input(format!("there is no batch {name} in that ZIP file")))?, Vec::new()),
        None => {
            let ranked = rank(job, &candidates)?;
            let best = candidates.iter().find(|c| c.name == ranked[0].name).ok_or_else(|| Error::Input("the best batch went missing".into()))?;
            (best, ranked)
        }
    };
    std::fs::create_dir_all(&job.out_dir)?;
    let reference = job.out_dir.join(format!("{}.txt", chosen.name));
    std::fs::write(&reference, &chosen.text)?;
    let outcome = calibrate(&Job { reference, ..job.clone() })?;
    Ok((outcome, ranked))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zip::tests::build;

    fn data(rows: &[(&str, f64)]) -> String {
        let mut s = String::from("IT8.7/1\t\t\nNUMBER_OF_FIELDS 4\nBEGIN_DATA_FORMAT\nSAMPLE_ID\tXYZ_X\tXYZ_Y\tXYZ_Z\nEND_DATA_FORMAT\nBEGIN_DATA\n");
        for (id, v) in rows {
            s.push_str(&format!("{id}\t\t{v}\t{v}\t{v}\t\n"));
        }
        s.push_str("END_DATA\n");
        s
    }

    fn many() -> Vec<(String, f64)> {
        (1..=30).map(|i| (format!("A{i}"), i as f64)).collect()
    }

    fn text_of(rows: &[(String, f64)]) -> String {
        data(&rows.iter().map(|(a, b)| (a.as_str(), *b)).collect::<Vec<_>>())
    }

    #[test]
    fn patch_ids_are_matched_across_the_two_spellings() {
        assert_eq!(patch_id("A1"), "A01");
        assert_eq!(patch_id("A01"), "A01");
        assert_eq!(patch_id("GS0"), "GS00");
        assert_eq!(patch_id("L22"), "L22");
        assert_eq!(patch_id("odd"), "odd");
        let m = parse_reference(&data(&[("A1", 1.5), ("GS0", 60.0)])).unwrap();
        assert_eq!(m["A01"], ["1.5", "1.5", "1.5"]);
        assert_eq!(m["GS00"][1], "60");
        // Old Mac line endings.
        assert!(parse_reference(&data(&[("A1", 1.5)]).replace('\n', "\r")).is_some());
        assert!(parse_reference("not a data file").is_none());
    }

    fn candidate(name: &str) -> Candidate {
        Candidate { name: name.into(), text: String::new() }
    }

    #[test]
    fn the_typed_batch_is_found_however_it_was_typed() {
        let all = [candidate("T100926"), candidate("T100726"), candidate("E410327"), candidate("2410327")];
        let found = |t: &str| find_batch(&all, t).map(|c| c.name.as_str());
        assert_eq!(found("T100926"), Some("T100926"));
        assert_eq!(found(" t 100926 "), Some("T100926"));
        assert_eq!(found("100926"), Some("T100926"));
        // Ambiguous or too short to be a code, or not there: nothing is guessed.
        assert_eq!(found("10"), None);
        assert_eq!(found("410327"), None, "E410327 and 2410327 both end this way");
        assert_eq!(found("T999999"), None);
        assert_eq!(found(""), None);
    }

    #[test]
    fn the_batch_whose_grey_scale_is_a_straight_line_to_the_scan_ranks_first() {
        // A scanner reading: each channel linear in the true batch's X, Y and Z, plus a little black offset.
        let steps: Vec<String> = grey_steps();
        let truth = |i: usize| {
            let y = 63.0 * 0.78f64.powi(i as i32);
            [y * 0.92, y, y * 1.1]
        };
        let scan: HashMap<String, [f64; 3]> = steps
            .iter()
            .enumerate()
            .map(|(i, id)| {
                (id.clone(), {
                    let t = truth(i);
                    [0.4 * t[0] / 100.0 + 0.001, 0.38 * t[1] / 100.0 + 0.001, 0.3 * t[2] / 100.0 + 0.001]
                })
            })
            .collect();
        let batch = |f: &dyn Fn(usize) -> [f64; 3]| -> HashMap<String, [String; 3]> { steps.iter().enumerate().map(|(i, id)| (id.clone(), f(i).map(|v| v.to_string()))).collect() };
        let right = grey_residual(&scan, &batch(&truth)).unwrap();
        // Another film: its ramp bends where the scan's does not.
        let other = grey_residual(
            &scan,
            &batch(&|i| {
                let y = 63.0 * 0.78f64.powi(i as i32).powf(1.15);
                [y * 0.92, y, y * 1.1]
            }),
        )
        .unwrap();
        assert!(right < 0.5, "{right}");
        assert!(other > 3.0 * right, "{other} vs {right}");
        // A batch with steps missing cannot be compared.
        let mut short = batch(&truth);
        short.remove("GS05");
        assert!(grey_residual(&scan, &short).is_none());
    }

    #[test]
    fn a_makers_zip_yields_the_slide_batches() {
        let rows = many();
        let (t1, t2) = (text_of(&rows), text_of(&rows.iter().map(|(a, b)| (a.clone(), b + 1.0)).collect::<Vec<_>>()));
        let inner1 = build(&[("T100926.txt", t1.as_bytes(), true), ("__MACOSX/._T100926.txt", b"junk", false)]);
        let inner2 = build(&[("T270725.txt", t2.as_bytes(), true)]);
        let reflective = build(&[("R202104.txt", t1.as_bytes(), true)]);
        let zip = build(&[
            ("it8_data/Transmissive/35MM/T100926.txt.zip", &inner1, true),
            ("it8_data/Transmissive/35MM/T270725.txt.zip", &inner2, false),
            ("it8_data/Reflective/FUJI_7x5/R202104.txt.zip", &reflective, false),
            ("it8_data/ArgyllCMS/IT8.cht.zip", b"not data", false),
            ("__MACOSX/it8_data/Transmissive/._x", b"junk", false),
        ]);
        let c = candidates_from_zip(&zip).unwrap();
        assert_eq!(c.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["T100926", "T270725"], "reflective targets and junk were not left out");
        assert!(c[0].text.contains("BEGIN_DATA"));
        // Plain text files in the ZIP work too, and a ZIP without any is refused.
        let plain = build(&[("T1.txt", t1.as_bytes(), false)]);
        assert_eq!(candidates_from_zip(&plain).unwrap()[0].name, "T1");
        assert!(candidates_from_zip(&build(&[("readme.md", b"x", false)])).is_err());
        assert!(candidates_from_zip(b"junk").is_err());
    }
}
