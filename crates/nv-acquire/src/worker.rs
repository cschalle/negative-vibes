use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_channel::{Receiver, Sender};
use nv_device::{AcquisitionPlan, DeviceCapabilities, IrStyle, Quirks};
use nv_sane::{parameters::FrameFormat, Session};

use crate::event::{ScanEvent, WorkerCmd};
use crate::frame::FrameMeta;

const READ_BUF: usize = 65536;

/// Owns the SANE worker thread.
///
/// Create with `AcquireWorker::spawn`.  All SANE calls happen on that thread.
pub struct AcquireWorker {
    cmd_tx: Sender<WorkerCmd>,
    pub evt_rx: Receiver<ScanEvent>,
}

impl AcquireWorker {
    /// Spawn the worker thread.  `quirks_path` is the path to `devices.toml`.
    pub fn spawn(quirks_path: PathBuf) -> Self {
        let (cmd_tx, cmd_rx) = async_channel::unbounded::<WorkerCmd>();
        let (evt_tx, evt_rx) = async_channel::unbounded::<ScanEvent>();

        std::thread::Builder::new()
            .name("nv-sane-worker".to_owned())
            .spawn(move || run_worker(cmd_rx, evt_tx, quirks_path))
            .expect("failed to spawn SANE worker thread");

        Self { cmd_tx, evt_rx }
    }

    pub fn probe(&self) {
        let _ = self.cmd_tx.send_blocking(WorkerCmd::Probe);
    }

    pub fn start_scan(
        &self,
        device_name: String,
        resolution_dpi: u32,
        with_ir: bool,
        area: Option<[f64; 4]>,
    ) {
        let _ = self.cmd_tx.send_blocking(WorkerCmd::StartScan {
            device_name,
            resolution_dpi,
            with_ir,
            area,
        });
    }

    pub fn cancel(&self) {
        let _ = self.cmd_tx.send_blocking(WorkerCmd::Cancel);
    }

    pub fn quit(&self) {
        let _ = self.cmd_tx.send_blocking(WorkerCmd::Quit);
    }
}

// ── Worker thread ─────────────────────────────────────────────────────────────

fn run_worker(
    cmd_rx: Receiver<WorkerCmd>,
    evt_tx: Sender<ScanEvent>,
    quirks_path: PathBuf,
) {
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

    loop {
        let Ok(cmd) = cmd_rx.recv_blocking() else {
            break; // channel closed
        };

        match cmd {
            WorkerCmd::Quit => break,

            WorkerCmd::Probe => {
                do_probe(&session, &quirks, &evt_tx);
            }

            WorkerCmd::StartScan { device_name, resolution_dpi, with_ir, area } => {
                cancelled.store(false, Ordering::SeqCst);
                do_scan(
                    &session,
                    &quirks,
                    &evt_tx,
                    &cmd_rx,
                    cancelled.clone(),
                    &device_name,
                    resolution_dpi,
                    with_ir,
                    area,
                );
            }

            WorkerCmd::Cancel => {
                // If no scan is running, Cancel is a no-op here; the atomic is
                // checked inside do_scan's read loop.
                cancelled.store(true, Ordering::SeqCst);
            }
        }
    }
}

fn do_probe(session: &Session, quirks: &Quirks, evt_tx: &Sender<ScanEvent>) {
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
        if dev.name.starts_with("v4l:")
            || kind_lc.contains("camera")
            || kind_lc.contains("virtual device")
        {
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
    session: &Session,
    quirks: &Quirks,
    evt_tx: &Sender<ScanEvent>,
    cmd_rx: &Receiver<WorkerCmd>,
    cancelled: Arc<AtomicBool>,
    device_name: &str,
    resolution_dpi: u32,
    with_ir: bool,
    area: Option<[f64; 4]>,
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
    let dev_info = match session.get_devices(true) {
        Ok(d) => d,
        Err(_) => vec![],
    };
    let dev = dev_info.iter().find(|d| d.name == device_name);
    let (vendor, model) = dev
        .map(|d| (d.vendor.as_str(), d.model.as_str()))
        .unwrap_or(("", ""));

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

    if let Some(area) = area {
        if let Err(e) = apply_area(&handle, area) {
            let _ = evt_tx.send_blocking(ScanEvent::Error(format!("set scan area: {e}")));
            return;
        }
    }

    // Emit device ready.
    let _ = evt_tx.send_blocking(ScanEvent::DeviceReady { caps: Box::new(caps.clone()) });

    let ir_follows = with_ir
        && matches!(caps.ir, IrStyle::MultiFrame | IrStyle::Rgbi | IrStyle::SeparateScan);

    // Scan all frames.
    let mut frame_index = 0u32;
    loop {
        if cancelled.load(Ordering::SeqCst) {
            handle.cancel();
            let _ = evt_tx.send_blocking(ScanEvent::Cancelled);
            return;
        }

        if let Err(e) = handle.start() {
            let _ = evt_tx.send_blocking(ScanEvent::Error(format!(
                "Could not start scan: {e}\nCheck that the scanner lid is closed and film is loaded."
            )));
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
            let _ = evt_tx.send_blocking(ScanEvent::IrFrameStarted {
                width_px: meta.width_px,
                height_px: meta.height_px,
                bytes_per_line: meta.bytes_per_line,
                depth: meta.depth,
            });
        }

        if frame_index == 0 {
            eprintln!(
                "nv-acquire: scan parameters: {}×{:?} depth={} ch={} bytes_per_line={}",
                meta.width_px, meta.height_px, meta.depth, meta.channels(), meta.bytes_per_line
            );
            let _ = evt_tx.send_blocking(ScanEvent::ScanStarted {
                width_px: meta.width_px,
                height_px: meta.height_px,
                bytes_per_line: meta.bytes_per_line,
                depth: meta.depth,
                channels: meta.channels(),
            });
        }

        // Read the frame.
        if let Err(cancelled_flag) = read_frame(
            &handle,
            &meta,
            evt_tx,
            cmd_rx,
            cancelled.clone(),
        ) {
            if cancelled_flag {
                let _ = evt_tx.send_blocking(ScanEvent::Cancelled);
            }
            return;
        }

        // Report frame completion.
        let is_ir_frame = matches!(
            params.format,
            FrameFormat::Gray | FrameFormat::Rgbi
        ) && frame_index > 0;

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

/// Read a single SANE frame and stream it as DataChunk events.
/// Returns Err(true) if cancelled, Err(false) on SANE error.
fn read_frame(
    handle: &nv_sane::Handle,
    meta: &FrameMeta,
    evt_tx: &Sender<ScanEvent>,
    cmd_rx: &Receiver<WorkerCmd>,
    cancelled: Arc<AtomicBool>,
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

        let _ = evt_tx.send_blocking(ScanEvent::DataChunk {
            offset,
            data: buf[..n].to_vec(),
        });

        offset += n;

        if let Some(total) = total {
            if total > 0 {
                let _ = evt_tx.send_blocking(ScanEvent::Progress(offset as f32 / total as f32));
            }
        }
    }

    Ok(())
}

/// Map a fractional sub-area onto a device's `[min, max]` ranges.
///
/// `ranges` is `[x_min, x_max, y_min, y_max]` in the device's unit (mm).
/// Returns `[tl-x, tl-y, br-x, br-y]`, or `None` if the area is empty or
/// reaches outside the window.
pub fn area_to_device(area: [f64; 4], ranges: [f64; 4]) -> Option<[f64; 4]> {
    let [x0, y0, x1, y1] = area;
    if !(0.0..=1.0).contains(&x0) || !(0.0..=1.0).contains(&y0)
        || !(0.0..=1.0).contains(&x1) || !(0.0..=1.0).contains(&y1)
        || x1 <= x0 || y1 <= y0
    {
        return None;
    }
    let [xmin, xmax, ymin, ymax] = ranges;
    let (w, h) = (xmax - xmin, ymax - ymin);
    Some([xmin + x0 * w, ymin + y0 * h, xmin + x1 * w, ymin + y1 * h])
}

fn apply_area(handle: &nv_sane::Handle, area: [f64; 4]) -> Result<(), String> {
    use nv_sane::{OptionConstraint, OptionValue};

    let range_of = |name: &str| -> Result<(f64, f64), String> {
        let count = handle.option_count().map_err(|e| e.to_string())?;
        for i in 1..count {
            if let Some(d) = handle.option_descriptor(i) {
                if d.name == name {
                    return match d.constraint {
                        OptionConstraint::Range { min, max, .. } => Ok((min, max)),
                        _ => Err(format!("option {name} has no range")),
                    };
                }
            }
        }
        Err(format!("device has no {name} option"))
    };

    let (xmin, xmax) = range_of("tl-x")?;
    let (ymin, ymax) = range_of("tl-y")?;
    let [tlx, tly, brx, bry] = area_to_device(area, [xmin, xmax, ymin, ymax])
        .ok_or_else(|| format!("invalid scan area {area:?}"))?;

    // Top-left first: the bottom-right values are always beyond it.
    for (name, v) in [("tl-x", tlx), ("tl-y", tly), ("br-x", brx), ("br-y", bry)] {
        handle
            .set_option_by_name(name, &OptionValue::Fixed(v))
            .map_err(|e| format!("{name}: {e}"))?;
    }
    eprintln!("nv-acquire: scan area {tlx:.2},{tly:.2} – {brx:.2},{bry:.2} mm");
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

fn apply_plan_options(
    handle: &nv_sane::Handle,
    plan: &AcquisitionPlan,
) -> nv_sane::Result<()> {
    use nv_sane::OptionValue;

    // mode
    let _ = handle.set_option_by_name("mode", &OptionValue::String(plan.mode.clone()))?;

    // resolution
    let _ = handle.set_option_by_name(
        "resolution",
        &OptionValue::Int(plan.resolution_dpi as i32),
    )?;

    // depth
    let _ = handle.set_option_by_name("depth", &OptionValue::Int(plan.depth as i32))?;

    // preview
    if plan.preview_mode {
        let _ = handle.set_option_by_name("preview", &OptionValue::Bool(true))?;
    }

    // neutralise options (e.g. auto-wb, ir-clean on plustek8200i)
    for name in &plan.neutralise {
        let count = handle.option_count()?;
        for i in 1..count {
            if let Some(desc) = handle.option_descriptor(i) {
                if desc.name == *name {
                    let val = match desc.kind {
                        nv_sane::OptionKind::Bool => OptionValue::Bool(false),
                        nv_sane::OptionKind::Int => OptionValue::Int(0),
                        _ => continue,
                    };
                    let _ = handle.set_option(&desc, &val)?;
                    break;
                }
            }
        }
    }

    Ok(())
}
