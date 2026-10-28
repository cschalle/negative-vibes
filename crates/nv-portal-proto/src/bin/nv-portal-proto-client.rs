//! Command-line client for the prototype portal, and the end-to-end check for it.
//!
//! Usage: nv-portal-proto-client [--name BUS_NAME] [--set NAME=VALUE]... [--output FILE.pnm]
//!
//! Selects a scanner, opens it, lists options, applies `--set`, scans every frame and
//! writes a PNM file (8 or 16 bit, gray or RGB, joining separate colour frames).

use futures_util::StreamExt;
use nv_portal_proto::proxy::{FrameEventArgs, RequestProxy, ScannerProxy, SessionProxy};
use nv_portal_proto::{DEFAULT_BUS_NAME, PORTAL_PATH, request_path};
use std::collections::HashMap;
use std::io::{Read, Write};
use zbus::zvariant::{OwnedValue, Value};

type Err = Box<dyn std::error::Error>;

struct Frame {
    format: String,
    depth: u32,
    pixels: u32,
    bytes_per_line: u32,
    lines: i32,
    last: bool,
    data: Vec<u8>,
}

fn get<'a, T: TryFrom<&'a OwnedValue>>(m: &'a HashMap<String, OwnedValue>, k: &str) -> Option<T> {
    m.get(k).and_then(|v| T::try_from(v).ok())
}

fn main() -> Result<(), Err> {
    let mut name = DEFAULT_BUS_NAME.to_owned();
    let mut sets: Vec<(String, String)> = Vec::new();
    let mut output = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--name" => name = args.next().ok_or("--name needs a value")?,
            "--output" => output = Some(args.next().ok_or("--output needs a value")?),
            "--set" => {
                let kv = args.next().ok_or("--set needs NAME=VALUE")?;
                let (k, v) = kv.split_once('=').ok_or("--set needs NAME=VALUE")?;
                sets.push((k.to_owned(), v.to_owned()));
            }
            _ => return Err(format!("unknown argument {a}").into()),
        }
    }
    zbus::block_on(run(name, sets, output))
}

async fn run(name: String, sets: Vec<(String, String)>, output: Option<String>) -> Result<(), Err> {
    let conn = zbus::Connection::session().await?;
    let me = conn.unique_name().ok_or("no unique name")?.to_string();
    let portal = ScannerProxy::builder(&conn).destination(name.as_str())?.path(PORTAL_PATH)?.build().await?;
    println!("portal version {}", portal.version().await?);

    // SelectScanners: subscribe to the Request's Response first, at the predicted path.
    let token = "select1";
    let rpath = request_path(&me, token);
    let request = RequestProxy::builder(&conn).destination(name.as_str())?.path(rpath.as_str())?.build().await?;
    let mut responses = request.receive_response().await?;
    let mut opts: HashMap<&str, Value<'_>> = HashMap::new();
    opts.insert("handle_token", token.into());
    let handle = portal.select_scanners("", opts).await?;
    assert_eq!(handle.as_str(), rpath, "handle must match the predicted path");
    let resp = responses.next().await.ok_or("no Response")?;
    let rargs = resp.args()?;
    if *rargs.response() != 0 {
        return Err(format!("SelectScanners failed with response {}", rargs.response()).into());
    }
    let picked = rargs.results().get("scanners").ok_or("no scanners selected")?;
    let picked: Vec<(String, HashMap<String, OwnedValue>)> = Vec::<(String, HashMap<String, OwnedValue>)>::try_from(picked.try_clone()?)?;
    let (id, info) = picked.first().ok_or("no scanner selected")?;
    println!("selected {id}: {} ({}) caps {:?}", get::<&str>(info, "name").unwrap_or("?"), get::<&str>(info, "kind").unwrap_or("?"), info.get("capabilities"));
    println!("listed after grant: {}", portal.list_scanners(HashMap::new()).await?.len());

    let session_path = portal.open_scanner(id, HashMap::new()).await?;
    let session = SessionProxy::builder(&conn).destination(name.as_str())?.path(session_path.as_str())?.build().await?;
    println!("opened {session_path}");

    let descs = portal.get_options(&session_path, HashMap::new()).await?;
    println!("{} options:", descs.len());
    for d in &descs {
        let n: &str = get(d, "name").unwrap_or("");
        let ty: &str = get(d, "type").unwrap_or("?");
        println!("  {:<22} {:<7} {}", if n.is_empty() { "(group)" } else { n }, ty, get::<&str>(d, "title").unwrap_or(""));
    }

    if !sets.is_empty() {
        let mut values: Vec<(&str, Value<'_>)> = Vec::new();
        for (k, v) in &sets {
            let desc = descs.iter().find(|d| get::<&str>(d, "name") == Some(k.as_str())).ok_or_else(|| format!("no option {k}"))?;
            let value: Value<'_> = match get::<&str>(desc, "type").unwrap_or("") {
                "bool" => (v == "true" || v == "1").into(),
                "int" => v.parse::<i32>()?.into(),
                "fixed" => v.parse::<f64>()?.into(),
                _ => v.as_str().into(),
            };
            values.push((k.as_str(), value));
        }
        let (applied, flags) = portal.set_options(&session_path, values, HashMap::new()).await?;
        println!("applied {applied:?} flags {flags:?}");
    }

    // Frames: StartFrame returns the pipe at once; wait for the events for state.
    let mut events = portal.receive_frame_event().await?;
    let mut frames: Vec<Frame> = Vec::new();
    loop {
        let (fd, _frame) = portal.start_frame(&session_path, HashMap::new()).await?;
        let mut file = std::fs::File::from(std::os::fd::OwnedFd::from(fd));
        let reader = std::thread::spawn(move || {
            let mut data = Vec::new();
            file.read_to_end(&mut data).map(|_| data)
        });
        let mut frame: Option<Frame> = None;
        loop {
            let ev = events.next().await.ok_or("signal stream ended")?;
            let a: FrameEventArgs = ev.args()?;
            match *a.state() {
                "started" => {
                    let d = a.details();
                    let u = |k: &str| d.get(k).and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
                    frame = Some(Frame {
                        format: d.get("format").and_then(|v| <&str>::try_from(v).ok()).unwrap_or("").to_owned(),
                        depth: u("depth"),
                        pixels: u("pixels_per_line"),
                        bytes_per_line: u("bytes_per_line"),
                        lines: d.get("lines").and_then(|v| i32::try_from(v).ok()).unwrap_or(-1),
                        last: d.get("last_frame").and_then(|v| bool::try_from(v).ok()).unwrap_or(true),
                        data: Vec::new(),
                    });
                }
                "finished" => break,
                other => return Err(format!("frame ended with '{other}': {:?}", a.details()).into()),
            }
        }
        let mut f = frame.ok_or("finished without started")?;
        f.data = reader.join().map_err(|_| "reader thread panicked")??;
        println!("frame: {} {}bit {}x{} bytes {} last {}", f.format, f.depth, f.pixels, f.lines, f.data.len(), f.last);
        let last = f.last;
        frames.push(f);
        if last {
            break;
        }
    }

    session.close().await?;
    println!("session closed");

    if let Some(path) = output {
        write_pnm(&path, &frames)?;
        println!("wrote {path}");
    }
    Ok(())
}

/// Rows without the padding SANE may add, as 8 or 16 bit samples.
fn rows(f: &Frame) -> Result<Vec<&[u8]>, Err> {
    let bpp = (f.depth.max(8) / 8) as usize;
    let channels = if f.format == "rgb" { 3 } else { 1 };
    let row_bytes = f.pixels as usize * bpp * channels;
    if f.lines < 0 || f.depth == 1 || f.bytes_per_line as usize != row_bytes && (f.bytes_per_line as usize) < row_bytes {
        return Err("unsupported frame layout for PNM output".into());
    }
    let n = f.data.len() / f.bytes_per_line as usize;
    Ok((0..n).map(|i| &f.data[i * f.bytes_per_line as usize..][..row_bytes]).collect())
}

fn write_pnm(path: &str, frames: &[Frame]) -> Result<(), Err> {
    let first = frames.first().ok_or("no frames")?;
    let bpp = (first.depth.max(8) / 8) as usize;
    let maxval = if first.depth > 8 { 65535 } else { 255 };
    let (magic, w, h) = (if first.format == "gray" { "P5" } else { "P6" }, first.pixels as usize, first.lines.max(0) as usize);
    let mut out = std::fs::File::create(path)?;
    write!(out, "{magic}\n{w} {h}\n{maxval}\n")?;
    let swap = |chunk: &mut Vec<u8>| {
        // SANE 16-bit samples are host-endian; PNM is big-endian.
        if bpp == 2 && cfg!(target_endian = "little") {
            for p in chunk.as_chunks_mut::<2>().0 {
                p.swap(0, 1);
            }
        }
    };
    if first.format == "red" {
        // Separate planes: interleave R, G, B.
        let plane = |i: usize| rows(&frames[i]);
        let (r, g, b) = (plane(0)?, plane(1)?, plane(2)?);
        for y in 0..r.len() {
            let mut line = Vec::with_capacity(w * 3 * bpp);
            for x in 0..w {
                for p in [r[y], g[y], b[y]] {
                    line.extend_from_slice(&p[x * bpp..(x + 1) * bpp]);
                }
            }
            swap(&mut line);
            out.write_all(&line)?;
        }
    } else {
        for row in rows(first)? {
            let mut line = row.to_vec();
            swap(&mut line);
            out.write_all(&line)?;
        }
    }
    Ok(())
}
