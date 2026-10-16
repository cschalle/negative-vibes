use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_channel::{Receiver, Sender};
use nv_device::{AcquisitionPlan, DeviceCapabilities, IrStyle, Quirks};
use nv_sane::{ScannerBackend, ScannerHandle, Session, parameters::FrameFormat};

use crate::event::{ScanEvent, WorkerCmd};
use crate::frame::FrameMeta;
use crate::passes::{self, Accumulator, Layout, PassPlan};

const READ_BUF: usize = 65536;

/// Owns the SANE worker thread.
///
/// Create with `AcquireWorker::spawn`.  All SANE calls happen on that thread.
pub struct AcquireWorker {
    cmd_tx: Sender<WorkerCmd>,
    pub evt_rx: Receiver<ScanEvent>,
    /// Joined on drop; see the `Drop` impl.
    thread: Option<std::thread::JoinHandle<()>>,
}

impl AcquireWorker {
    /// Spawn the worker thread.  `quirks_path` is the path to `devices.toml`.
    pub fn spawn(quirks_path: PathBuf) -> Self {
        let (cmd_tx, cmd_rx) = async_channel::unbounded::<WorkerCmd>();
        let (evt_tx, evt_rx) = async_channel::unbounded::<ScanEvent>();

        let thread =
            std::thread::Builder::new().name("nv-sane-worker".to_owned()).spawn(move || run_worker(cmd_rx, evt_tx, quirks_path)).expect("failed to spawn SANE worker thread");

        Self { cmd_tx, evt_rx, thread: Some(thread) }
    }

    pub fn probe(&self) {
        let _ = self.cmd_tx.send_blocking(WorkerCmd::Probe);
    }

    pub fn start_scan(&self, device_name: String, resolution_dpi: u32, with_ir: bool, area: Option<[f64; 4]>) {
        self.start_scan_with(device_name, resolution_dpi, with_ir, area, PassPlan::SINGLE);
    }

    /// [`start_scan`](Self::start_scan) with several passes over the frame.
    pub fn start_scan_with(&self, device_name: String, resolution_dpi: u32, with_ir: bool, area: Option<[f64; 4]>, passes: PassPlan) {
        let _ = self.cmd_tx.send_blocking(WorkerCmd::StartScan { device_name, resolution_dpi, with_ir, area, passes });
    }

    pub fn cancel(&self) {
        let _ = self.cmd_tx.send_blocking(WorkerCmd::Cancel);
    }

    pub fn quit(&self) {
        let _ = self.cmd_tx.send_blocking(WorkerCmd::Quit);
    }
}

impl Drop for AcquireWorker {
    /// Stop the worker and wait for it to finish.
    ///
    /// The worker owns the SANE session, so `sane_exit` (which unloads the backend
    /// libraries) runs on its thread when it ends. If the process were allowed to
    /// exit first, the C library's exit-time teardown would race that unloading and
    /// abort inside some backend's destructor. A running scan treats `Quit` as a
    /// cancel, so this does not wait for the scan to complete.
    fn drop(&mut self) {
        let _ = self.cmd_tx.send_blocking(WorkerCmd::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

// ── Worker thread ─────────────────────────────────────────────────────────────

fn run_worker(cmd_rx: Receiver<WorkerCmd>, evt_tx: Sender<ScanEvent>, quirks_path: PathBuf) {
    let quirks = Quirks::load(&quirks_path);

    let session = match Session::new() {
        Ok(s) => s,
        Err(e) => {
            let _ = evt_tx.send_blocking(ScanEvent::Error(format!("sane_init failed: {e}")));
            return;
        }
    };

    // Cancelled flag shared between the scan loop and the Quit/Cancel command.
    let cancelled = Arc::new(AtomicBool::new(false));

    // The loop ends when the channel closes.
    while let Ok(cmd) = cmd_rx.recv_blocking() {
        match cmd {
            WorkerCmd::Quit => break,

            WorkerCmd::Probe => {
                do_probe(&session, &quirks, &evt_tx);
            }

            WorkerCmd::StartScan { device_name, resolution_dpi, with_ir, area, passes } => {
                cancelled.store(false, Ordering::SeqCst);
                do_scan(&session, &quirks, &evt_tx, &cmd_rx, cancelled.clone(), &device_name, resolution_dpi, with_ir, area, passes);
            }

            WorkerCmd::Cancel => {
                // If no scan is running, Cancel is a no-op here; the atomic is
                // checked inside do_scan's read loop.
                cancelled.store(true, Ordering::SeqCst);
            }
        }
    }
}

fn do_probe(session: &dyn ScannerBackend, quirks: &Quirks, evt_tx: &Sender<ScanEvent>) {
    let devices = match session.get_devices(true) {
        Ok(d) => d,
        Err(e) => {
            let _ = evt_tx.send_blocking(ScanEvent::Error(format!("sane_get_devices: {e}")));
            return;
        }
    };

    let mut caps_list = Vec::new();

    for dev in &devices {
        // Skip v4l (webcam) devices: they report type "virtual device" and
        // their names start with "v4l:". They can't scan film.
        let kind_lc = dev.kind.to_lowercase();
        if dev.name.starts_with("v4l:") || kind_lc.contains("camera") || kind_lc.contains("virtual device") {
            eprintln!("nv-acquire: skipping non-scanner device '{}' (type: {})", dev.name, dev.kind);
            continue;
        }

        let handle = match session.open(&dev.name) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("nv-acquire: could not open {}: {e}", dev.name);
                continue;
            }
        };

        let backend = nv_device::quirks::backend_name(&dev.name);
        let qk = quirks.find(backend, &dev.model);
        match DeviceCapabilities::detect(&dev.name, &dev.vendor, &dev.model, &handle, qk) {
            Ok(caps) => caps_list.push(caps),
            Err(e) => eprintln!("nv-acquire: capability detect failed for {}: {e}", dev.name),
        }
        // handle is dropped here → sane_close
    }

    let _ = evt_tx.send_blocking(ScanEvent::DeviceList(caps_list));
}

fn do_scan(
    session: &dyn ScannerBackend,
    quirks: &Quirks,
    evt_tx: &Sender<ScanEvent>,
    cmd_rx: &Receiver<WorkerCmd>,
    cancelled: Arc<AtomicBool>,
    device_name: &str,
    resolution_dpi: u32,
    with_ir: bool,
    area: Option<[f64; 4]>,
    passes: PassPlan,
) {
    // Drain any stray Cancel commands before we start.
    while cmd_rx.try_recv().is_ok() {}

    let handle = match session.open(device_name) {
        Ok(h) => h,
        Err(e) => {
            let _ = evt_tx.send_blocking(ScanEvent::Error(format!("sane_open: {e}")));
            return;
        }
    };

    // Detect capabilities again so we can apply the plan.
    let dev_info = session.get_devices(true).unwrap_or_default();
    let dev = dev_info.iter().find(|d| d.name == device_name);
    let (vendor, model) = dev.map(|d| (d.vendor.as_str(), d.model.as_str())).unwrap_or(("", ""));

    let backend = nv_device::quirks::backend_name(device_name);
    let qk = quirks.find(backend, model);
    let caps = match DeviceCapabilities::detect(device_name, vendor, model, &handle, qk) {
        Ok(c) => c,
        Err(e) => {
            let _ = evt_tx.send_blocking(ScanEvent::Error(format!("capability detect: {e}")));
            return;
        }
    };

    let plan = AcquisitionPlan::scan(&caps, resolution_dpi, with_ir);

    // Apply options from the plan.
    if let Err(e) = apply_plan_options(&handle, &plan) {
        let _ = evt_tx.send_blocking(ScanEvent::Error(format!("apply options: {e}")));
        return;
    }

    if let Some(area) = area
        && let Err(e) = apply_area(&handle, area)
    {
        let _ = evt_tx.send_blocking(ScanEvent::Error(format!("set scan area: {e}")));
        return;
    }

    // Emit device ready.
    let _ = evt_tx.send_blocking(ScanEvent::DeviceReady { caps: Box::new(caps.clone()) });

    let ir_follows = with_ir && matches!(caps.ir, IrStyle::MultiFrame | IrStyle::Rgbi | IrStyle::SeparateScan);

    let passes = passes.normalised();
    if !passes.is_single() {
        let exposure = caps.exposure_option.clone().filter(|n| n.starts_with("exposure"));
        if passes.long_exposure.is_some() && exposure.is_none() {
            let _ = evt_tx.send_blocking(ScanEvent::Error("This scanner has no exposure control, so a long exposure cannot be added. Use several samples instead.".into()));
            return;
        }
        do_multi_pass(&handle, &MultiCtx { evt_tx, cmd_rx, cancelled: cancelled.clone(), ir_follows, exposure_option: exposure, passes });
        return;
    }

    // Scan all frames.
    let mut frame_index = 0u32;
    loop {
        if cancelled.load(Ordering::SeqCst) {
            handle.cancel();
            let _ = evt_tx.send_blocking(ScanEvent::Cancelled);
            return;
        }

        if let Err(e) = handle.start() {
            let _ = evt_tx.send_blocking(ScanEvent::Error(format!("Could not start scan: {e}\nCheck that the scanner lid is closed and film is loaded.")));
            return;
        }

        let params = match handle.get_parameters() {
            Ok(p) => p,
            Err(e) => {
                let _ = evt_tx.send_blocking(ScanEvent::Error(format!("sane_get_parameters: {e}")));
                return;
            }
        };

        let meta = FrameMeta {
            format: params.format.clone(),
            width_px: params.pixels_per_line as u32,
            height_px: params.lines.map(|l| l as u32),
            bytes_per_line: params.bytes_per_line,
            depth: params.depth,
            is_last: params.last_frame,
        };

        if frame_index > 0 && ir_follows && matches!(meta.format, FrameFormat::Gray) {
            let _ = evt_tx.send_blocking(ScanEvent::IrFrameStarted { width_px: meta.width_px, height_px: meta.height_px, bytes_per_line: meta.bytes_per_line, depth: meta.depth });
        }

        if frame_index == 0 {
            eprintln!("nv-acquire: scan parameters: {}×{:?} depth={} ch={} bytes_per_line={}", meta.width_px, meta.height_px, meta.depth, meta.channels(), meta.bytes_per_line);
            let _ = evt_tx.send_blocking(ScanEvent::ScanStarted {
                width_px: meta.width_px,
                height_px: meta.height_px,
                bytes_per_line: meta.bytes_per_line,
                depth: meta.depth,
                channels: meta.channels(),
            });
        }

        // Read the frame.
        let mut sink = |offset: usize, data: &[u8]| {
            let _ = evt_tx.send_blocking(ScanEvent::DataChunk { offset, data: data.to_vec() });
        };
        let mut progress = |f: f32| {
            let _ = evt_tx.send_blocking(ScanEvent::Progress(f));
        };
        if let Err(cancelled_flag) = read_frame(&handle, &meta, evt_tx, cmd_rx, cancelled.clone(), &mut sink, &mut progress) {
            if cancelled_flag {
                let _ = evt_tx.send_blocking(ScanEvent::Cancelled);
            }
            return;
        }

        // Report frame completion.
        let is_ir_frame = matches!(params.format, FrameFormat::Gray | FrameFormat::Rgbi) && frame_index > 0;

        if is_ir_frame {
            let _ = evt_tx.send_blocking(ScanEvent::IrFrameDone);
        } else if ir_follows && !params.last_frame {
            let _ = evt_tx.send_blocking(ScanEvent::ColorFrameDone);
        }

        if params.last_frame {
            break;
        }

        // Handle three-pass (Red/Green/Blue separate) — those have last_frame=false on
        // the first two; each pass needs another sane_start.
        frame_index += 1;
    }

    let _ = evt_tx.send_blocking(ScanEvent::ScanDone);
}

/// Read a single SANE frame, handing each chunk (with its byte offset) to
/// `sink` and the fraction done to `progress`.
/// Returns Err(true) if cancelled, Err(false) on SANE error.
fn read_frame(
    handle: &dyn ScannerHandle,
    meta: &FrameMeta,
    evt_tx: &Sender<ScanEvent>,
    cmd_rx: &Receiver<WorkerCmd>,
    cancelled: Arc<AtomicBool>,
    sink: &mut dyn FnMut(usize, &[u8]),
    progress: &mut dyn FnMut(f32),
) -> Result<(), bool> {
    let mut buf = vec![0u8; READ_BUF];
    let mut offset = 0usize;
    let total = meta.bytes_total();

    loop {
        // Check for cancel command between reads.
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                WorkerCmd::Cancel => {
                    cancelled.store(true, Ordering::SeqCst);
                }
                WorkerCmd::Quit => {
                    cancelled.store(true, Ordering::SeqCst);
                }
                _ => {}
            }
        }

        if cancelled.load(Ordering::SeqCst) {
            handle.cancel();
            return Err(true);
        }

        let n = match handle.read(&mut buf) {
            Ok(0) => break, // EOF
            Ok(n) => n,
            Err(e) => {
                let _ = evt_tx.send_blocking(ScanEvent::Error(format!("sane_read: {e}")));
                return Err(false);
            }
        };

        sink(offset, &buf[..n]);
        offset += n;

        if let Some(total) = total
            && total > 0
        {
            progress(offset as f32 / total as f32);
        }
    }

    Ok(())
}

// ── Several passes over one frame ─────────────────────────────────────────────

struct MultiCtx<'a> {
    evt_tx: &'a Sender<ScanEvent>,
    cmd_rx: &'a Receiver<WorkerCmd>,
    cancelled: Arc<AtomicBool>,
    ir_follows: bool,
    exposure_option: Option<String>,
    passes: PassPlan,
}

/// What one pass delivered.
struct PassData {
    colour: Vec<u8>,
    colour_layout: Layout,
    ir: Option<(Vec<u8>, Layout)>,
}

/// Scan the frame once, keeping the bytes instead of streaming them.
/// `fraction` maps the pass's own progress into the whole job's.
fn collect_pass(handle: &dyn ScannerHandle, ctx: &MultiCtx, fraction: &dyn Fn(f32) -> f32) -> Result<PassData, bool> {
    let mut colour: Option<(Vec<u8>, Layout)> = None;
    let mut ir: Option<(Vec<u8>, Layout)> = None;
    let mut frame_index = 0;
    loop {
        if ctx.cancelled.load(Ordering::SeqCst) {
            handle.cancel();
            return Err(true);
        }
        if let Err(e) = handle.start() {
            let _ = ctx.evt_tx.send_blocking(ScanEvent::Error(format!("Could not start scan: {e}\nCheck that the scanner lid is closed and film is loaded.")));
            return Err(false);
        }
        let params = match handle.get_parameters() {
            Ok(p) => p,
            Err(e) => {
                let _ = ctx.evt_tx.send_blocking(ScanEvent::Error(format!("sane_get_parameters: {e}")));
                return Err(false);
            }
        };
        let (Some(h), channels) = (params.lines, params.format.channels()) else {
            let _ = ctx.evt_tx.send_blocking(ScanEvent::Error("Several passes need a scanner that reports the frame height up front.".into()));
            return Err(false);
        };
        let meta = FrameMeta {
            format: params.format.clone(),
            width_px: params.pixels_per_line as u32,
            height_px: Some(h as u32),
            bytes_per_line: params.bytes_per_line,
            depth: params.depth,
            is_last: params.last_frame,
        };
        let layout = Layout { width: meta.width_px as usize, height: h, channels: channels as usize, depth: meta.depth, bytes_per_line: meta.bytes_per_line };
        let mut data = vec![0u8; meta.bytes_per_line * h];
        let mut sink = |offset: usize, chunk: &[u8]| {
            let end = (offset + chunk.len()).min(data.len());
            if offset < end {
                data[offset..end].copy_from_slice(&chunk[..end - offset]);
            }
        };
        let mut progress = |f: f32| {
            let _ = ctx.evt_tx.send_blocking(ScanEvent::Progress(fraction(f)));
        };
        read_frame(handle, &meta, ctx.evt_tx, ctx.cmd_rx, ctx.cancelled.clone(), &mut sink, &mut progress)?;
        if frame_index > 0 && ctx.ir_follows && matches!(meta.format, FrameFormat::Gray) {
            ir = Some((data, layout));
        } else if colour.is_none() {
            colour = Some((data, layout));
        }
        if params.last_frame {
            // The scan is over: options can only be changed once the handle is idle again.
            handle.cancel();
            break;
        }
        frame_index += 1;
    }
    let Some((colour, colour_layout)) = colour else {
        let _ = ctx.evt_tx.send_blocking(ScanEvent::Error("The scanner delivered no picture.".into()));
        return Err(false);
    };
    Ok(PassData { colour, colour_layout, ir })
}

/// N samples at the normal exposure, and the same at a long one if asked;
/// lined up, averaged, merged, and sent on as if it were one scan.
fn do_multi_pass(handle: &dyn ScannerHandle, ctx: &MultiCtx) {
    use nv_sane::OptionValue;
    let plan = ctx.passes;
    let total = plan.total_scans();
    let samples = plan.samples as u32;
    let set_exposure = |e: f32| -> Result<(), String> {
        match &ctx.exposure_option {
            Some(name) => handle.set_option_by_name(name, &OptionValue::Fixed(e as f64)).map(|_| ()).map_err(|e| format!("{name}: {e}")),
            None => Ok(()),
        }
    };

    let mut short: Option<Accumulator> = None;
    let mut long: Option<Accumulator> = None;
    let mut ir_acc: Option<Accumulator> = None;
    let mut first_layout: Option<Layout> = None;

    for index in 0..total {
        let is_long = index >= samples;
        let exposure = if is_long { plan.long_exposure.unwrap_or(1.0) } else { 1.0 };
        if (index == 0 || index == samples)
            && let Err(e) = set_exposure(exposure)
        {
            let _ = ctx.evt_tx.send_blocking(ScanEvent::Error(format!("exposure: {e}")));
            return;
        }
        let _ = ctx.evt_tx.send_blocking(ScanEvent::PassStarted { index, total, exposure });
        let fraction = move |f: f32| (index as f32 + f) / total as f32 * 0.95;
        let pass = match collect_pass(handle, ctx, &fraction) {
            Ok(p) => p,
            Err(was_cancelled) => {
                if was_cancelled {
                    let _ = ctx.evt_tx.send_blocking(ScanEvent::Cancelled);
                }
                return;
            }
        };
        let layout = *first_layout.get_or_insert(pass.colour_layout);
        if pass.colour_layout != layout {
            let _ = ctx.evt_tx.send_blocking(ScanEvent::Error("The passes came out different sizes; try again.".into()));
            return;
        }
        let shift = if is_long {
            let acc = long.get_or_insert_with(|| short.as_ref().map(|s| s.linked(plan.max_offset)).unwrap_or_else(|| Accumulator::new(layout, plan.max_offset)));
            acc.add(&pass.colour)
        } else {
            let acc = short.get_or_insert_with(|| Accumulator::new(layout, plan.max_offset));
            let shift = acc.add(&pass.colour);
            // The infrared plane comes with the colour it was scanned with and moves with it.
            if let Some((data, l)) = &pass.ir {
                ir_acc.get_or_insert_with(|| Accumulator::new(*l, 0)).add_shifted(data, shift);
            }
            shift
        };
        eprintln!("nv-acquire: pass {} of {total} (exposure {exposure}) moved by ({:.2}, {:.2})", index + 1, shift.0, shift.1);
    }

    let _ = ctx.evt_tx.send_blocking(ScanEvent::Merging);
    let (Some(layout), Some(short)) = (first_layout, short) else { return };
    let mut flat = short.mean();
    if let Some(dir) = std::env::var_os("NV_DEBUG_PASSES") {
        let dir = std::path::PathBuf::from(dir);
        let dump = |name: &str, v: &[u32]| {
            let mut out = format!("P6\n{} {}\n65535\n", layout.width, layout.height).into_bytes();
            for s in v {
                out.extend((*s as u16).to_be_bytes());
            }
            let _ = std::fs::write(dir.join(name), out);
        };
        dump("dbg-short.pnm", &flat);
        if let Some(l) = &long {
            dump("dbg-long.pnm", &l.mean());
        }
    }
    if let (Some(long), Some(e)) = (&long, plan.long_exposure) {
        flat = passes::merge_exposures(&flat, &long.mean(), layout.channels, e, layout.max_value());
    }
    let bytes = layout.from_flat(&flat);

    let _ = ctx.evt_tx.send_blocking(ScanEvent::ScanStarted {
        width_px: layout.width as u32,
        height_px: Some(layout.height as u32),
        bytes_per_line: layout.bytes_per_line,
        depth: layout.depth,
        channels: layout.channels as u8,
    });
    for (i, chunk) in bytes.chunks(READ_BUF * 4).enumerate() {
        let _ = ctx.evt_tx.send_blocking(ScanEvent::DataChunk { offset: i * READ_BUF * 4, data: chunk.to_vec() });
    }
    if let Some(ir) = ir_acc {
        // The plane has its own size and depth: take them from what was collected.
        let l = ir.layout();
        let ir_bytes = l.from_flat(&ir.mean());
        let _ = ctx.evt_tx.send_blocking(ScanEvent::ColorFrameDone);
        let _ =
            ctx.evt_tx.send_blocking(ScanEvent::IrFrameStarted { width_px: l.width as u32, height_px: Some(l.height as u32), bytes_per_line: l.bytes_per_line, depth: l.depth });
        for (i, chunk) in ir_bytes.chunks(READ_BUF * 4).enumerate() {
            let _ = ctx.evt_tx.send_blocking(ScanEvent::DataChunk { offset: i * READ_BUF * 4, data: chunk.to_vec() });
        }
        let _ = ctx.evt_tx.send_blocking(ScanEvent::IrFrameDone);
    }
    // Leave the scanner as it was found.
    if ctx.exposure_option.is_some() {
        let _ = set_exposure(1.0);
    }
    let _ = ctx.evt_tx.send_blocking(ScanEvent::Progress(1.0));
    let _ = ctx.evt_tx.send_blocking(ScanEvent::ScanDone);
}

/// Map a fractional sub-area onto a device's `[min, max]` ranges.
///
/// `ranges` is `[x_min, x_max, y_min, y_max]` in the device's unit (mm).
/// Returns `[tl-x, tl-y, br-x, br-y]`, or `None` if the area is empty or
/// reaches outside the window.
pub fn area_to_device(area: [f64; 4], ranges: [f64; 4]) -> Option<[f64; 4]> {
    let [x0, y0, x1, y1] = area;
    if !(0.0..=1.0).contains(&x0) || !(0.0..=1.0).contains(&y0) || !(0.0..=1.0).contains(&x1) || !(0.0..=1.0).contains(&y1) || x1 <= x0 || y1 <= y0 {
        return None;
    }
    let [xmin, xmax, ymin, ymax] = ranges;
    let (w, h) = (xmax - xmin, ymax - ymin);
    Some([xmin + x0 * w, ymin + y0 * h, xmin + x1 * w, ymin + y1 * h])
}

fn apply_area(handle: &dyn ScannerHandle, area: [f64; 4]) -> Result<(), String> {
    use nv_sane::{OptionConstraint, OptionValue};

    let range_of = |name: &str| -> Result<(f64, f64), String> {
        let opts = handle.all_options().map_err(|e| e.to_string())?;
        match opts.into_iter().find(|d| d.name == name) {
            Some(d) => match d.constraint {
                OptionConstraint::Range { min, max, .. } => Ok((min, max)),
                _ => Err(format!("option {name} has no range")),
            },
            None => Err(format!("device has no {name} option")),
        }
    };

    let (xmin, xmax) = range_of("tl-x")?;
    let (ymin, ymax) = range_of("tl-y")?;
    let [tlx, tly, brx, bry] = area_to_device(area, [xmin, xmax, ymin, ymax]).ok_or_else(|| format!("invalid scan area {area:?}"))?;

    // Top-left first: the bottom-right values are always beyond it.
    for (name, v) in [("tl-x", tlx), ("tl-y", tly), ("br-x", brx), ("br-y", bry)] {
        handle.set_option_by_name(name, &OptionValue::Fixed(v)).map_err(|e| format!("{name}: {e}"))?;
    }
    eprintln!("nv-acquire: scan area {tlx:.2},{tly:.2} – {brx:.2},{bry:.2} mm");
    Ok(())
}

fn apply_plan_options(handle: &dyn ScannerHandle, plan: &AcquisitionPlan) -> nv_sane::Result<()> {
    use nv_sane::OptionValue;

    // mode
    let _ = handle.set_option_by_name("mode", &OptionValue::String(plan.mode.clone()))?;

    // resolution
    let _ = handle.set_option_by_name("resolution", &OptionValue::Int(plan.resolution_dpi as i32))?;

    // depth
    let _ = handle.set_option_by_name("depth", &OptionValue::Int(plan.depth as i32))?;

    // preview
    if plan.preview_mode {
        let _ = handle.set_option_by_name("preview", &OptionValue::Bool(true))?;
    }

    // neutralise options (e.g. auto-wb, ir-clean on plustek8200i)
    let opts = handle.all_options()?;
    for name in &plan.neutralise {
        if let Some(desc) = opts.iter().find(|d| d.name == *name) {
            let val = match desc.kind {
                nv_sane::OptionKind::Bool => OptionValue::Bool(false),
                nv_sane::OptionKind::Int => OptionValue::Int(0),
                _ => continue,
            };
            let _ = handle.set_option(desc, &val)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod area_tests {
    use super::area_to_device;

    #[test]
    fn maps_fractions_onto_the_window() {
        let a = area_to_device([0.25, 0.5, 0.75, 1.0], [0.0, 36.5, 0.0, 27.2]).unwrap();
        assert!((a[0] - 9.125).abs() < 1e-9 && (a[1] - 13.6).abs() < 1e-9);
        assert!((a[2] - 27.375).abs() < 1e-9 && (a[3] - 27.2).abs() < 1e-9);
    }

    #[test]
    fn rejects_empty_or_outside_areas() {
        let r = [0.0, 36.5, 0.0, 27.2];
        assert!(area_to_device([0.5, 0.0, 0.5, 1.0], r).is_none());
        assert!(area_to_device([0.6, 0.0, 0.4, 1.0], r).is_none());
        assert!(area_to_device([-0.1, 0.0, 1.0, 1.0], r).is_none());
        assert!(area_to_device([0.0, 0.0, 1.1, 1.0], r).is_none());
    }
}
