//! End to end with ArgyllCMS: a synthetic IT8 scan from a known scanner model
//! goes in, a profile comes out, and applying it to the patches gives back
//! the target's colours. Skipped when Argyll is not installed.

use nv_image::FrameBuf;
use nv_store::RawWriter;

const XYZ_TO_SRGB: [[f64; 3]; 3] = [[3.1338561, -1.6168667, -0.4906146], [-0.9787684, 1.9161415, 0.0334540], [0.0719453, -0.2289914, 1.4052427]];
/// How the pretend scanner sees colour: not sRGB, so the profile has work to do.
const MIX: [[f64; 3]; 3] = [[0.90, 0.08, 0.02], [0.05, 0.90, 0.05], [0.02, 0.10, 0.88]];
const SCALE: usize = 4;

struct Chart {
    /// Patch id, expected XYZ (0–100), and the box in chart units.
    patches: Vec<(String, [f64; 3], [f64; 4])>,
}

fn chart(cht: &str) -> Chart {
    let section = cht.split("EXPECTED XYZ").nth(1).expect("expected values");
    let mut patches = Vec::new();
    for line in section.lines().skip(1) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() != 4 {
            break;
        }
        let id = f[0];
        let xyz = [f[1].parse().unwrap(), f[2].parse().unwrap(), f[3].parse().unwrap()];
        // The grey strip runs along the bottom; the rest is a grid of 25.625-unit cells.
        let area = if let Some(n) = id.strip_prefix("GS") {
            [n.parse::<f64>().unwrap() * 25.625, 358.75, 25.625, 51.25]
        } else {
            let (row, col) = ((id.as_bytes()[0] - b'A') as f64, id[1..].parse::<f64>().unwrap() - 1.0);
            [26.625 + col * 25.625, 26.625 + row * 25.625, 25.625, 25.625]
        };
        patches.push((id.to_owned(), xyz, area));
    }
    Chart { patches }
}

fn device(xyz: [f64; 3]) -> [f64; 3] {
    let lin: Vec<f64> = (0..3).map(|r| (0..3).map(|k| XYZ_TO_SRGB[r][k] * xyz[k] / 100.0).sum::<f64>().max(0.0)).collect();
    let mut d = [0.0; 3];
    for r in 0..3 {
        d[r] = (0..3).map(|k| MIX[r][k] * lin[k]).sum::<f64>() * 0.9;
    }
    d
}

#[test]
fn an_it8_scan_becomes_a_profile_that_gives_the_target_colours_back() {
    if !nv_calibrate::available() {
        eprintln!("ArgyllCMS not installed: skipped");
        return;
    }
    let Some(cht_path) = nv_calibrate::default_chart() else { return eprintln!("no it8.cht: skipped") };
    let cht = std::fs::read_to_string(&cht_path).unwrap();
    let ch = chart(&cht);
    assert_eq!(ch.patches.len(), 264);

    let dir = std::env::temp_dir().join(format!("nv-calibrate-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // The image: a white page, a dark frame round the patches, and the patches.
    let (w, h) = (616 * SCALE, 410 * SCALE);
    let mut px = vec![[0.9f64 * 0.9; 3]; w * h];
    let mut fill = |x0: f64, y0: f64, bw: f64, bh: f64, c: [f64; 3]| {
        for y in (y0 * SCALE as f64) as usize..(((y0 + bh) * SCALE as f64) as usize).min(h) {
            for x in (x0 * SCALE as f64) as usize..(((x0 + bw) * SCALE as f64) as usize).min(w) {
                px[y * w + x] = c;
            }
        }
    };
    for (_, xyz, b) in &ch.patches {
        fill(b[0], b[1], b[2], b[3], device(*xyz));
    }
    let mut buf = FrameBuf::new(w as u32, Some(h as u32), 16, 3);
    let mut bytes = Vec::with_capacity(w * h * 6);
    for p in &px {
        for c in p {
            bytes.extend(((c.clamp(0.0, 1.0) * 65535.0).round() as u16).to_ne_bytes());
        }
    }
    buf.append(&bytes);
    let scan = dir.join("scan.tif");
    RawWriter::write_scan(&buf, &scan).unwrap();

    // The target's measured values: the chart's own expected ones, in the CGATS form.
    let mut reference = String::from(
        "IT8.7/1\nORIGINATOR \"test\"\nDESCRIPTOR \"synthetic\"\nCREATED \"2026\"\nNUMBER_OF_FIELDS 4\nBEGIN_DATA_FORMAT\nSAMPLE_ID XYZ_X XYZ_Y XYZ_Z\nEND_DATA_FORMAT\n",
    );
    reference += &format!("NUMBER_OF_SETS {}\nBEGIN_DATA\n", ch.patches.len());
    for (id, xyz, _) in &ch.patches {
        reference += &format!("{id} {} {} {}\n", xyz[0], xyz[1], xyz[2]);
    }
    reference += "END_DATA\n";
    let reference_path = dir.join("target.txt");
    std::fs::write(&reference_path, reference).unwrap();

    let s = SCALE as f64;
    let job = nv_calibrate::Job {
        scan,
        chart: cht_path,
        reference: reference_path,
        out_dir: dir.join("out"),
        name: "synthetic".into(),
        corners: Some([1.0 * s, 1.0 * s, 616.0 * s, 1.5 * s, 615.5 * s, 358.0 * s, 1.0 * s, 358.5 * s]),
    };
    let out = nv_calibrate::calibrate(&job).unwrap_or_else(|e| panic!("{e}"));
    assert!(out.profile.is_file());
    assert!(out.average_error.is_some_and(|e| e < 3.0), "colprof reports {:?}", out.average_error);

    // Independent of colprof's own report: through our reader, device values give the target's colours.
    let mut worst = 0f64;
    let mut sum = 0f64;
    // The profile may scale overall brightness; fit one factor.
    let mut got: Vec<([f64; 3], [f64; 3])> = Vec::new();
    for (_, xyz, _) in &ch.patches {
        let mut d: Vec<f32> = device(*xyz).iter().map(|v| *v as f32).collect();
        out.input.apply(&mut d);
        let want: Vec<f64> = (0..3).map(|r| (0..3).map(|k| XYZ_TO_SRGB[r][k] * xyz[k] / 100.0).sum::<f64>()).collect();
        got.push(([d[0] as f64, d[1] as f64, d[2] as f64], [want[0], want[1], want[2]]));
    }
    let k = got.iter().map(|(g, w)| g.iter().zip(w).map(|(a, b)| a * b).sum::<f64>()).sum::<f64>() / got.iter().map(|(g, _)| g.iter().map(|a| a * a).sum::<f64>()).sum::<f64>();
    for (g, w) in &got {
        let e = (0..3).map(|c| (g[c] * k - w[c].clamp(0.0, 1.0)).abs()).fold(0.0, f64::max);
        worst = worst.max(e);
        sum += e;
    }
    let mean = sum / got.len() as f64;
    eprintln!("profile error against the target, in linear sRGB: mean {mean:.4}, worst {worst:.4}, brightness factor {k:.3}, colprof {:?}/{:?}", out.average_error, out.peak_error);
    assert!(mean < 0.02 && worst < 0.1, "mean {mean}, worst {worst}");
    if std::env::var_os("NV_KEEP").is_none() {
        let _ = std::fs::remove_dir_all(&dir);
    }
}
