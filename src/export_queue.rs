//! The export queue: one worker thread that renders and writes frames one
//! after another, so an export never stalls the window or the scanner
//! (DESIGN.md §3, "Export/IO (1)"). Jobs overlap with scanning: the next
//! frame scans while the previous one exports.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_channel::{Receiver, Sender};
use nv_export::{ExportReport, ExportSettings, export_image_full, unique_path};
use nv_gphotos::{Endpoints, OAuthConfig, Tokens};
use nv_process::{DEFAULT_PROXY_PIXELS, ProcessParams, Proxy, tiles::freeze};

/// How to send an exported picture to Google Photos.
#[derive(Debug, Clone)]
pub struct UploadSpec {
    pub cfg: OAuthConfig,
    pub refresh_token: String,
    /// Shown as the picture's description in Google Photos.
    pub description: String,
    /// The album to put it in: its title, and its id if the roll already has one.
    pub album: Option<(String, Option<String>)>,
}

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
    /// Send the file to Google Photos once it is written.
    pub upload: Option<UploadSpec>,
}

#[derive(Debug)]
pub enum ExportEvent {
    Started {
        id: u64,
    },
    Progress {
        id: u64,
        fraction: f32,
    },
    Done {
        id: u64,
        report: ExportReport,
    },
    Failed {
        id: u64,
        message: String,
    },
    Cancelled {
        id: u64,
    },
    /// The picture of job `id` is being sent to Google Photos.
    Uploading {
        id: u64,
    },
    Uploaded {
        id: u64,
        url: Option<String>,
    },
    /// The upload failed; the file is still on disk. `sign_in_needed`: Google no longer accepts the saved sign-in.
    UploadFailed {
        id: u64,
        message: String,
        sign_in_needed: bool,
    },
    /// An album was made for the roll `title`.
    AlbumCreated {
        title: String,
        id: String,
    },
}

enum Cmd {
    Enqueue(Box<ExportJob>),
    /// Send a file that is already written (a retry).
    Upload(u64, PathBuf, Box<UploadSpec>),
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
        Self::spawn_with(Endpoints::google())
    }

    /// A queue that talks to these Google endpoints (a local server, in tests).
    pub fn spawn_with(endpoints: Endpoints) -> Self {
        let (tx, rx) = async_channel::unbounded::<Cmd>();
        let (evt_tx, events) = async_channel::unbounded::<ExportEvent>();
        let control = Arc::new(Control::default());
        let c = control.clone();
        std::thread::Builder::new().name("nv-export".into()).spawn(move || worker(rx, evt_tx, c, endpoints)).expect("failed to spawn the export thread");
        Self { tx, events, control }
    }

    /// Try the upload of a written file again.
    pub fn retry_upload(&self, id: u64, path: PathBuf, spec: UploadSpec) {
        let _ = self.tx.send_blocking(Cmd::Upload(id, path, Box::new(spec)));
    }

    pub fn enqueue(&self, job: ExportJob) {
        let _ = self.tx.send_blocking(Cmd::Enqueue(Box::new(job)));
    }

    /// Stop a job: before it starts it is skipped; while it runs it stops at
    /// the next band and leaves no file.
    pub fn cancel(&self, id: u64) {
        self.control.cancelled.lock().unwrap_or_else(|e| e.into_inner()).insert(id);
        if let Some((running, flag)) = self.control.running.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
            && *running == id
        {
            flag.store(true, Ordering::Relaxed);
        }
    }
}

impl Drop for ExportQueue {
    fn drop(&mut self) {
        let _ = self.tx.send_blocking(Cmd::Quit);
    }
}

/// What the worker remembers between uploads: the signed-in client (so its access token
/// is reused) and the albums made so far, by title.
struct Google {
    endpoints: Endpoints,
    client: Option<(String, nv_gphotos::Client)>,
    albums: HashMap<String, String>,
}

impl Google {
    fn client(&mut self, spec: &UploadSpec) -> &mut nv_gphotos::Client {
        if self.client.as_ref().is_none_or(|(token, _)| *token != spec.refresh_token) {
            let tokens = Tokens { refresh_token: spec.refresh_token.clone(), access_token: None, expires_at: None };
            self.client = Some((spec.refresh_token.clone(), nv_gphotos::Client::new(self.endpoints.clone(), spec.cfg.clone(), tokens)));
        }
        &mut self.client.as_mut().expect("just set").1
    }

    /// Send `path` to Google Photos, making the roll's album first if it has none.
    fn upload(&mut self, events: &Sender<ExportEvent>, id: u64, path: &std::path::Path, spec: &UploadSpec) {
        let _ = events.send_blocking(ExportEvent::Uploading { id });
        let result = self.try_upload(events, path, spec);
        let _ = events.send_blocking(match result {
            Ok(item) => ExportEvent::Uploaded { id, url: item.product_url },
            Err(nv_gphotos::Error::NotSignedIn) => ExportEvent::UploadFailed { id, message: "Sign in to Google Photos again".into(), sign_in_needed: true },
            Err(e) => ExportEvent::UploadFailed { id, message: e.to_string(), sign_in_needed: false },
        });
    }

    fn try_upload(&mut self, events: &Sender<ExportEvent>, path: &std::path::Path, spec: &UploadSpec) -> nv_gphotos::Result<nv_gphotos::UploadedItem> {
        let mut album = None;
        if let Some((title, known)) = &spec.album {
            album = match self.albums.get(title).cloned().or_else(|| known.clone()) {
                Some(a) => Some(a),
                None => Some(self.make_album(events, spec, title)?),
            };
        }
        let first = self.client(spec).upload_file(path, &spec.description, album.as_deref());
        match (first, &spec.album) {
            // The album may have been deleted in Google Photos: make another and try once more.
            (Err(nv_gphotos::Error::Http { status: 400 | 404, .. }), Some((title, _))) if album.is_some() => {
                self.albums.remove(title);
                let fresh = self.make_album(events, spec, title)?;
                self.client(spec).upload_file(path, &spec.description, Some(&fresh))
            }
            (other, _) => other,
        }
    }

    fn make_album(&mut self, events: &Sender<ExportEvent>, spec: &UploadSpec, title: &str) -> nv_gphotos::Result<String> {
        let id = self.client(spec).create_album(title)?;
        self.albums.insert(title.to_owned(), id.clone());
        let _ = events.send_blocking(ExportEvent::AlbumCreated { title: title.to_owned(), id: id.clone() });
        Ok(id)
    }
}

fn worker(rx: Receiver<Cmd>, events: Sender<ExportEvent>, control: Arc<Control>, endpoints: Endpoints) {
    let mut google = Google { endpoints, client: None, albums: HashMap::new() };
    while let Ok(cmd) = rx.recv_blocking() {
        let job = match cmd {
            Cmd::Enqueue(job) => job,
            Cmd::Upload(id, path, spec) => {
                google.upload(&events, id, &path, &spec);
                continue;
            }
            Cmd::Quit => break,
        };
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
        let uploadable = match (&result, &job.upload) {
            (Ok(report), Some(spec)) => Some((report.path.clone(), spec.clone())),
            _ => None,
        };
        let event = match result {
            Ok(report) => ExportEvent::Done { id, report },
            Err(nv_export::ExportError::Cancelled) => ExportEvent::Cancelled { id },
            Err(e) => ExportEvent::Failed { id, message: e.to_string() },
        };
        let _ = events.send_blocking(event);
        if let Some((path, spec)) = uploadable {
            google.upload(&events, id, &path, &spec);
        }
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
            upload: None,
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

    /// A stand-in for Google: answers by path, and counts what it was asked.
    fn fake_google(refuse_albums_once: bool) -> (Endpoints, Arc<Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let log = Arc::new(Mutex::new(Vec::new()));
        let seen = log.clone();
        std::thread::spawn(move || {
            let mut refuse = refuse_albums_once;
            let mut albums = 0;
            while let Ok((mut s, _)) = listener.accept() {
                let mut data = Vec::new();
                let mut buf = [0u8; 65536];
                let (end, len) = loop {
                    let n = s.read(&mut buf).unwrap_or(0);
                    data.extend_from_slice(&buf[..n]);
                    if let Some(p) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&data[..p]).to_ascii_lowercase();
                        break (p + 4, head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse().ok()).unwrap_or(0usize));
                    }
                    if n == 0 {
                        break (data.len(), 0);
                    }
                };
                while data.len() < end + len {
                    let n = s.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(&buf[..n]);
                }
                let head = String::from_utf8_lossy(&data[..end]).into_owned();
                let line = head.lines().next().unwrap_or("").to_owned();
                let body = String::from_utf8_lossy(&data[end..]).into_owned();
                let (status, reply) = if line.contains("/token") {
                    (200, r#"{"access_token":"A","expires_in":3599}"#.to_owned())
                } else if line.contains("/v1/albums") {
                    albums += 1;
                    (200, format!(r#"{{"id":"ALBUM-{albums}"}}"#))
                } else if line.contains("/v1/uploads") {
                    (200, "UPLOAD-TOKEN".to_owned())
                } else if refuse && body.contains("ALBUM-1") {
                    refuse = false;
                    (400, r#"{"error":{"message":"Invalid album"}}"#.to_owned())
                } else {
                    (200, r#"{"newMediaItemResults":[{"status":{"message":"Success"},"mediaItem":{"id":"M","productUrl":"https://photos.example/M"}}]}"#.to_owned())
                };
                seen.lock().unwrap().push(format!("{line} {}", body.chars().take(400).collect::<String>()));
                let _ = write!(s, "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len());
            }
        });
        let endpoints = Endpoints {
            auth: format!("{base}/auth"),
            token: format!("{base}/token"),
            revoke: format!("{base}/revoke"),
            upload: format!("{base}/v1/uploads"),
            batch_create: format!("{base}/v1/mediaItems:batchCreate"),
            albums: format!("{base}/v1/albums"),
        };
        (endpoints, log)
    }

    fn upload_spec() -> UploadSpec {
        UploadSpec {
            cfg: OAuthConfig { client_id: "c".into(), client_secret: "s".into() },
            refresh_token: "R".into(),
            description: "Portra 400".into(),
            album: Some(("Roll 1".into(), None)),
        }
    }

    #[test]
    fn exported_files_are_uploaded_and_the_roll_gets_one_album() {
        let dir = scratch("upload");
        let (endpoints, log) = fake_google(false);
        let q = ExportQueue::spawn_with(endpoints);
        for id in 1..=2 {
            let mut j = job(id, &dir, fixture());
            j.upload = Some(upload_spec());
            q.enqueue(j);
        }
        let events = collect(&q, |e| matches!(e, ExportEvent::Uploaded { id: 2, .. }));
        let names: Vec<String> = events
            .iter()
            .filter_map(|e| match e {
                ExportEvent::Done { id, .. } => Some(format!("done{id}")),
                ExportEvent::Uploading { id } => Some(format!("uploading{id}")),
                ExportEvent::Uploaded { id, url } => Some(format!("uploaded{id}:{}", url.as_deref().unwrap_or(""))),
                ExportEvent::AlbumCreated { title, id } => Some(format!("album {title}={id}")),
                _ => None,
            })
            .collect();
        assert_eq!(names, ["done1", "uploading1", "album Roll 1=ALBUM-1", "uploaded1:https://photos.example/M", "done2", "uploading2", "uploaded2:https://photos.example/M"]);
        assert_eq!(log.lock().unwrap().iter().filter(|l| l.contains("/v1/albums")).count(), 1, "one album for the whole roll");
        assert_eq!(log.lock().unwrap().iter().filter(|l| l.contains("/token")).count(), 1, "the access token is reused");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_deleted_album_is_made_again() {
        let dir = scratch("album-gone");
        let (endpoints, _) = fake_google(true);
        let q = ExportQueue::spawn_with(endpoints);
        let mut j = job(1, &dir, fixture());
        let mut spec = upload_spec();
        spec.album = Some(("Roll 1".into(), Some("ALBUM-1".into()))); // the id the roll remembers
        j.upload = Some(spec);
        q.enqueue(j);
        let events = collect(&q, |e| matches!(e, ExportEvent::Uploaded { .. } | ExportEvent::UploadFailed { .. }));
        assert!(events.iter().any(|e| matches!(e, ExportEvent::AlbumCreated { id, .. } if id == "ALBUM-1")), "{events:?}");
        assert!(matches!(events.last(), Some(ExportEvent::Uploaded { .. })), "{events:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_upload_keeps_the_file_and_can_be_retried() {
        let dir = scratch("retry");
        // Nothing listens here: the upload fails, the export does not.
        let dead = Endpoints { token: "http://127.0.0.1:1/token".into(), ..Endpoints::google() };
        let q = ExportQueue::spawn_with(dead);
        let mut j = job(1, &dir, fixture());
        j.upload = Some(UploadSpec { album: None, ..upload_spec() });
        q.enqueue(j);
        let events = collect(&q, |e| matches!(e, ExportEvent::UploadFailed { .. }));
        let path = events.iter().find_map(|e| if let ExportEvent::Done { report, .. } = e { Some(report.path.clone()) } else { None }).unwrap();
        assert!(path.exists());
        q.retry_upload(1, path, UploadSpec { album: None, ..upload_spec() });
        let again = collect(&q, |e| matches!(e, ExportEvent::UploadFailed { .. }));
        assert!(matches!(again.first(), Some(ExportEvent::Uploading { id: 1 })), "{again:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
