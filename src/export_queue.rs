//! The export queue: one worker thread that renders and writes frames one
//! after another, so an export never stalls the window or the scanner
//! (DESIGN.md §3, "Export/IO (1)"). Jobs overlap with scanning: the next
//! frame scans while the previous one exports.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_channel::{Receiver, Sender};
use nv_export::{ExportReport, ExportSettings, export_image_full, unique_path};
use nv_process::{DEFAULT_PROXY_PIXELS, ProcessParams, Proxy, tiles::freeze};

/// One frame to export.
#[derive(Debug, Clone)]
pub struct ExportJob {
    pub id: u64,
    pub frame: u32,
    /// Shown in the queue: "Frame 3".
    pub label: String,
    /// The raw scan to render.
    pub raw: PathBuf,
    /// What to render it with: the frame's effective settings.
    pub params: ProcessParams,
    /// The frame's rotation and flips.
    pub orientation: nv_process::Orientation,
    /// Fine straightening, degrees clockwise.
    pub angle: f32,
    /// The XMP packet to embed, built from the roll's metadata and the frame.
    pub xmp: Option<Vec<u8>>,
    /// What a DNG says about the picture and its scan.
    pub dng: Option<nv_export::dng::DngInfo>,
    pub settings: ExportSettings,
    /// Folder (created if missing) and file name without extension.
    pub dir: PathBuf,
    pub stem: String,
}

#[derive(Debug)]
pub enum ExportEvent {
    Started { id: u64 },
    Progress { id: u64, fraction: f32 },
    Done { id: u64, report: ExportReport },
    Failed { id: u64, message: String },
    Cancelled { id: u64 },
}

enum Cmd {
    Enqueue(Box<ExportJob>),
    Quit,
}

#[derive(Default)]
struct Control {
    /// Jobs cancelled before they started.
    cancelled: Mutex<HashSet<u64>>,
    /// The job in progress, and its cancel flag.
    running: Mutex<Option<(u64, Arc<AtomicBool>)>>,
}

pub struct ExportQueue {
    tx: Sender<Cmd>,
    pub events: Receiver<ExportEvent>,
    control: Arc<Control>,
}

impl ExportQueue {
    pub fn spawn() -> Self {
        let (tx, rx) = async_channel::unbounded::<Cmd>();
        let (evt_tx, events) = async_channel::unbounded::<ExportEvent>();
        let control = Arc::new(Control::default());
        let c = control.clone();
        std::thread::Builder::new().name("nv-export".into()).spawn(move || worker(rx, evt_tx, c)).expect("failed to spawn the export thread");
        Self { tx, events, control }
    }

    pub fn enqueue(&self, job: ExportJob) {
        let _ = self.tx.send_blocking(Cmd::Enqueue(Box::new(job)));
    }

    /// Stop a job: before it starts it is skipped; while it runs it stops at
    /// the next band and leaves no file.
    pub fn cancel(&self, id: u64) {
        self.control.cancelled.lock().unwrap_or_else(|e| e.into_inner()).insert(id);
        if let Some((running, flag)) = self.control.running.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            if *running == id {
                flag.store(true, Ordering::Relaxed);
            }
        }
    }
}

impl Drop for ExportQueue {
    fn drop(&mut self) {
        let _ = self.tx.send_blocking(Cmd::Quit);
    }
}

fn worker(rx: Receiver<Cmd>, events: Sender<ExportEvent>, control: Arc<Control>) {
    while let Ok(Cmd::Enqueue(job)) = rx.recv_blocking() {
        let id = job.id;
        if control.cancelled.lock().unwrap_or_else(|e| e.into_inner()).remove(&id) {
            let _ = events.send_blocking(ExportEvent::Cancelled { id });
            continue;
        }
        let flag = Arc::new(AtomicBool::new(false));
        *control.running.lock().unwrap_or_else(|e| e.into_inner()) = Some((id, flag.clone()));
        let _ = events.send_blocking(ExportEvent::Started { id });

        let result = run(&job, &events, &flag);
        *control.running.lock().unwrap_or_else(|e| e.into_inner()) = None;
        control.cancelled.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        let event = match result {
            Ok(report) => ExportEvent::Done { id, report },
            Err(nv_export::ExportError::Cancelled) => ExportEvent::Cancelled { id },
            Err(e) => ExportEvent::Failed { id, message: e.to_string() },
        };
        let _ = events.send_blocking(event);
    }
}

fn run(job: &ExportJob, events: &Sender<ExportEvent>, cancel: &AtomicBool) -> Result<ExportReport, nv_export::ExportError> {
    let io = |e: String| nv_export::ExportError::Io(std::io::Error::other(e));
    std::fs::create_dir_all(&job.dir)?;
    let raw = nv_store::RawReader::read(&job.raw).map_err(|e| io(format!("could not read the scan: {e}")))?;

    // The scan is already the frame's rectangle, so nothing is left to crop;
    // the measured values are fixed from a reduced copy, as in the editor.
    let proxy = Proxy::from_raw(&raw, DEFAULT_PROXY_PIXELS);
    let mut params = job.params.clone();
    params.conversion.region = None;
    let frozen = freeze(&proxy, &params);

    let dest = unique_path(&job.dir, &job.stem, job.settings.format.extension());
    let id = job.id;
    let last = std::cell::Cell::new(-1.0f32);
    let progress = |f: f32| {
        if f - last.get() >= 0.01 || f >= 1.0 {
            last.set(f);
            let _ = events.send_blocking(ExportEvent::Progress { id, fraction: f });
        }
    };
    export_image_full(&raw, &frozen, job.orientation, job.angle, job.xmp.as_deref(), job.dng.as_ref(), &job.settings, &dest, &progress, cancel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nv_film::builtin_profiles;
    use std::time::{Duration, Instant};

    fn fixture() -> PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/plustek8200i/tulip-600.pnm")
    }

    fn job(id: u64, dir: &std::path::Path, raw: PathBuf) -> ExportJob {
        ExportJob {
            id,
            frame: id as u32,
            label: format!("Frame {id}"),
            raw,
            params: nv_process::default_params_for(&builtin_profiles()[0]),
            orientation: nv_process::Orientation::IDENTITY,
            angle: 0.0,
            xmp: None,
            dng: None,
            settings: ExportSettings { bit_depth: 8, ..Default::default() },
            dir: dir.to_owned(),
            stem: "roll-01".into(),
        }
    }

    /// Events until `done` says stop, or fail after 60 s.
    fn collect(q: &ExportQueue, mut done: impl FnMut(&ExportEvent) -> bool) -> Vec<ExportEvent> {
        let (mut out, start) = (Vec::new(), Instant::now());
        while start.elapsed() < Duration::from_secs(60) {
            match q.events.try_recv() {
                Ok(e) => {
                    let stop = done(&e);
                    out.push(e);
                    if stop {
                        return out;
                    }
                }
                Err(_) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        panic!("timed out; saw {out:?}");
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nv-queue-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn jobs_run_in_order_and_equal_names_get_numbers() {
        let dir = scratch("order"); // does not exist yet: the worker creates it
        let q = ExportQueue::spawn();
        q.enqueue(job(1, &dir, fixture()));
        q.enqueue(job(2, &dir, fixture()));
        let events = collect(&q, |e| matches!(e, ExportEvent::Done { id: 2, .. }));

        let order: Vec<String> = events
            .iter()
            .map(|e| match e {
                ExportEvent::Started { id } => format!("start{id}"),
                ExportEvent::Done { id, .. } => format!("done{id}"),
                ExportEvent::Progress { .. } => "p".into(),
                other => format!("{other:?}"),
            })
            .filter(|s| s != "p")
            .collect();
        assert_eq!(order, ["start1", "done1", "start2", "done2"]);
        assert!(events.iter().any(|e| matches!(e, ExportEvent::Progress { id: 1, .. })));
        assert!(dir.join("roll-01.tif").exists() && dir.join("roll-01 (2).tif").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_scan_fails_that_job_and_the_queue_carries_on() {
        let dir = scratch("fail");
        let q = ExportQueue::spawn();
        q.enqueue(job(1, &dir, PathBuf::from("/nonexistent/scan.tif")));
        q.enqueue(job(2, &dir, fixture()));
        let events = collect(&q, |e| matches!(e, ExportEvent::Done { id: 2, .. }));
        assert!(events.iter().any(|e| matches!(e, ExportEvent::Failed { id: 1, message } if message.contains("could not read"))), "{events:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_job_cancelled_before_it_starts_is_skipped() {
        let dir = scratch("cancel");
        let q = ExportQueue::spawn();
        // Job 1 keeps the worker busy while job 2 is cancelled in the queue.
        q.enqueue(job(1, &dir, fixture()));
        q.enqueue(job(2, &dir, fixture()));
        q.cancel(2);
        q.enqueue(job(3, &dir, fixture()));
        let events = collect(&q, |e| matches!(e, ExportEvent::Done { id: 3, .. }));
        assert!(events.iter().any(|e| matches!(e, ExportEvent::Cancelled { id: 2 })), "{events:?}");
        assert!(!events.iter().any(|e| matches!(e, ExportEvent::Started { id: 2 })));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2, "only jobs 1 and 3 write files");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
