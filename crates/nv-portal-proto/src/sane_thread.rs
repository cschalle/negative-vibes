//! The one thread that owns libsane. Everything else talks to it through [`Cmd`].
//!
//! Same rule as the app's own worker: SANE backends are not thread-safe, so every call
//! happens here, and a running frame blocks other commands (a prototype limit; the real
//! portal runs one helper process per session).

use crate::convert::{self, Vardict, ov};
use async_channel::Sender;
use nv_sane::{DeviceInfo, OptionDescriptor, OptionValue, SaneError, ScannerBackend, ScannerHandle, Session, SetInfo};
use nv_sane_sys as sys;
use std::collections::{HashMap, HashSet};
use std::io::{ErrorKind, PipeWriter, Write};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

pub type Reply<T> = Sender<T>;

pub enum SetError {
    Invalid(String),
    Failed(String),
}

/// One entry of a `SetOptions` call.
pub enum Setting {
    Value(OptionValue),
    /// The option's automatic value.
    Auto,
}

pub struct SetOutcome {
    pub applied: Vec<(String, OptionValue)>,
    pub flags: Vec<&'static str>,
}

pub struct FrameJob {
    pub sid: u64,
    /// Number of this frame within the session, sent as `frame` in each of its events.
    pub serial: u32,
    /// Session object path and owner, for the FrameEvent signals.
    pub path: String,
    pub owner: String,
    /// Write end of the frame pipe; `None` once closed.
    pub out: Option<PipeWriter>,
    pub cancel: Arc<AtomicBool>,
    pub busy: Arc<AtomicBool>,
}

pub enum Cmd {
    Devices {
        local_only: bool,
        reply: Reply<Result<Vec<DeviceInfo>, String>>,
    },
    Open {
        name: String,
        reply: Reply<Result<u64, SaneError>>,
    },
    Params {
        sid: u64,
        reply: Reply<Result<Vardict, String>>,
    },
    Close {
        sid: u64,
    },
    Options {
        sid: u64,
        reply: Reply<Result<Vec<OptionDescriptor>, String>>,
    },
    Values {
        sid: u64,
        reply: Reply<Result<Vec<(String, OptionValue)>, String>>,
    },
    Set {
        sid: u64,
        values: Vec<(String, Setting)>,
        reply: Reply<Result<SetOutcome, SetError>>,
    },
    Frame(FrameJob),
    /// Abandon a multi-frame scan between frames (Cancel() with no frame running).
    Abort {
        sid: u64,
    },
}

pub struct FrameEvent {
    pub owner: String,
    pub session_path: String,
    pub state: &'static str,
    pub details: Vardict,
}

pub fn spawn(rx: mpsc::Receiver<Cmd>, events: Sender<FrameEvent>) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new().name("sane".into()).spawn(move || run(rx, events)).expect("spawn sane thread")
}

fn run(rx: mpsc::Receiver<Cmd>, events: Sender<FrameEvent>) {
    let session = match Session::new() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("nv-portal-proto: sane_init failed: {e}");
            return;
        }
    };
    let mut open: HashMap<u64, Box<dyn ScannerHandle>> = HashMap::new();
    // Scanners between frames of a multi-frame scan (colour, then infrared; or R, G, B).
    let mut mid_scan: HashSet<u64> = HashSet::new();
    let mut next_sid = 1u64;

    while let Ok(cmd) = rx.recv() {
        match cmd {
            Cmd::Devices { local_only, reply } => {
                let _ = reply.send_blocking(ScannerBackend::get_devices(&session, local_only).map_err(|e| e.to_string()));
            }
            Cmd::Open { name, reply } => {
                let r = ScannerBackend::open(&session, &name).map(|h| {
                    let sid = next_sid;
                    next_sid += 1;
                    open.insert(sid, h);
                    sid
                });
                let _ = reply.send_blocking(r);
            }
            Cmd::Params { sid, reply } => {
                let r = with_handle(&open, sid, |h| h.get_parameters().map(|p| convert::parameters_to_dict(&p)).map_err(|e| e.to_string()));
                let _ = reply.send_blocking(r);
            }
            Cmd::Close { sid } => {
                // Dropping the handle closes the scanner.
                mid_scan.remove(&sid);
                if let Some(h) = open.remove(&sid) {
                    h.cancel();
                }
            }
            Cmd::Options { sid, reply } => {
                let r = with_handle(&open, sid, |h| h.all_options().map_err(|e| e.to_string()));
                let _ = reply.send_blocking(r);
            }
            Cmd::Values { sid, reply } => {
                let r = with_handle(&open, sid, |h| {
                    let mut out = Vec::new();
                    for d in h.all_options().map_err(|e| e.to_string())? {
                        let readable = d.is_active() && d.cap & sys::SANE_CAP_SOFT_DETECT != 0;
                        if !convert::is_exposed(&d) || !readable || matches!(d.kind, nv_sane::OptionKind::Group | nv_sane::OptionKind::Button) {
                            continue;
                        }
                        if let Ok(v) = h.get_option(&d) {
                            out.push((d.name, v));
                        }
                    }
                    Ok(out)
                });
                let _ = reply.send_blocking(r);
            }
            Cmd::Set { sid, values, reply } => {
                let r = match open.get(&sid) {
                    Some(h) => set_options(h.as_ref(), values),
                    None => Err(SetError::Failed("no such scanner session".into())),
                };
                let _ = reply.send_blocking(r);
            }
            Cmd::Abort { sid } => {
                if mid_scan.remove(&sid)
                    && let Some(h) = open.get(&sid)
                {
                    h.cancel();
                }
            }
            Cmd::Frame(job) => match open.get(&sid_of(&job)) {
                Some(h) => {
                    let sid = job.sid;
                    if run_frame(h.as_ref(), job, &events, mid_scan.contains(&sid)) {
                        mid_scan.insert(sid);
                    } else {
                        mid_scan.remove(&sid);
                    }
                }
                None => finish(&events, &job, "error", error_details("org.freedesktop.portal.Error.Failed", "no such scanner session")),
            },
        }
    }
}

fn sid_of(job: &FrameJob) -> u64 {
    job.sid
}

fn with_handle<T>(open: &HashMap<u64, Box<dyn ScannerHandle>>, sid: u64, f: impl FnOnce(&dyn ScannerHandle) -> Result<T, String>) -> Result<T, String> {
    match open.get(&sid) {
        Some(h) => f(h.as_ref()),
        None => Err("no such scanner session".into()),
    }
}

fn status_code(e: &SaneError) -> Option<u32> {
    match e {
        SaneError::Status { code, .. } => Some(*code),
        _ => None,
    }
}

fn set_options(h: &dyn ScannerHandle, values: Vec<(String, Setting)>) -> Result<SetOutcome, SetError> {
    let mut flags = SetInfo::default();
    let mut names = Vec::new();
    for (name, value) in values {
        // Refetch each time: an earlier option may have changed the later ones.
        let opts = h.all_options().map_err(|e| SetError::Failed(e.to_string()))?;
        let desc = opts.iter().find(|d| d.name == name && convert::is_exposed(d)).ok_or_else(|| SetError::Invalid(format!("no option named '{name}'")))?;
        if !desc.is_settable() || !desc.is_active() {
            return Err(SetError::Invalid(format!("option '{name}' cannot be set now")));
        }
        // Check the variant type against the option before it reaches the backend.
        let result = match &value {
            Setting::Auto => h.set_option_auto(desc),
            Setting::Value(value) => {
                let wanted = matches!(
                    (&desc.kind, value),
                    (nv_sane::OptionKind::Bool, OptionValue::Bool(_))
                        | (nv_sane::OptionKind::Int, OptionValue::Int(_))
                        | (nv_sane::OptionKind::Fixed, OptionValue::Fixed(_))
                        | (nv_sane::OptionKind::String, OptionValue::String(_))
                        | (nv_sane::OptionKind::Button, OptionValue::Button)
                );
                if !wanted {
                    return Err(SetError::Invalid(format!("wrong value type for option '{name}'")));
                }
                h.set_option(desc, value)
            }
        };
        match result {
            Ok(info) => {
                flags.inexact |= info.inexact;
                flags.reload_options |= info.reload_options;
                flags.reload_params |= info.reload_params;
                names.push(name);
            }
            Err(e) if status_code(&e) == Some(sys::SANE_STATUS_INVAL) => return Err(SetError::Invalid(format!("'{name}': {e}"))),
            Err(e) => return Err(SetError::Failed(format!("'{name}': {e}"))),
        }
    }
    // Report what each option ended up as.
    let opts = h.all_options().map_err(|e| SetError::Failed(e.to_string()))?;
    let applied = names
        .into_iter()
        .filter_map(|n| {
            let d = opts.iter().find(|d| d.name == n)?;
            h.get_option(d).ok().map(|v| (n, v))
        })
        .collect();
    let mut out = Vec::new();
    if flags.inexact {
        out.push("inexact");
    }
    if flags.reload_options {
        out.push("reload-options");
    }
    if flags.reload_params {
        out.push("reload-params");
    }
    Ok(SetOutcome { applied, flags: out })
}

fn error_details(code: &str, message: &str) -> Vardict {
    let mut d = Vardict::new();
    d.insert("code".into(), ov(code.to_owned()));
    d.insert("message".into(), ov(message.to_owned()));
    d
}

fn dbus_error_for(e: &SaneError) -> &'static str {
    match status_code(e) {
        Some(sys::SANE_STATUS_ACCESS_DENIED) => "org.freedesktop.portal.Error.NotAllowed",
        Some(sys::SANE_STATUS_INVAL) => "org.freedesktop.portal.Error.InvalidArgument",
        _ => "org.freedesktop.portal.Error.Failed",
    }
}

/// End a frame: close the pipe first so the app sees EOF, free the frame slot so it can
/// start the next frame from its `finished` handler, then report.
fn finish(events: &Sender<FrameEvent>, job: &FrameJob, state: &'static str, details: Vardict) {
    job.busy.store(false, Ordering::SeqCst);
    emit(events, job, state, details);
}

/// Send a FrameEvent for `job`, numbered with the frame it belongs to so that a client can tell
/// it from the events of an earlier frame.
fn emit(events: &Sender<FrameEvent>, job: &FrameJob, state: &'static str, mut details: Vardict) {
    details.insert("frame".into(), ov(job.serial));
    let _ = events.send_blocking(FrameEvent { owner: job.owner.clone(), session_path: job.path.clone(), state, details });
}

/// Run one frame. `mid_scan` says the previous frame was not the last of its scan, so this one
/// continues it: SANE wants `sane_start` again with no `sane_cancel` in between (a cancel makes a
/// real driver begin the scan over, so a colour+infrared scan would deliver colour forever).
/// Returns true when this frame finished and more frames of the scan follow.
fn run_frame(h: &dyn ScannerHandle, mut job: FrameJob, events: &Sender<FrameEvent>, mid_scan: bool) -> bool {
    if !mid_scan {
        // A new scan: clear anything left over, as the app's own worker does before its first frame.
        h.cancel();
    }

    if let Err(e) = h.start() {
        let (state, details) = match status_code(&e) {
            Some(sys::SANE_STATUS_NO_DOCS) => ("no-documents", Vardict::new()),
            Some(sys::SANE_STATUS_CANCELLED) => ("cancelled", Vardict::new()),
            _ => ("error", error_details(dbus_error_for(&e), &e.to_string())),
        };
        job.out = None;
        finish(events, &job, state, details);
        return false;
    }
    let params = match h.get_parameters() {
        Ok(p) => p,
        Err(e) => {
            h.cancel();
            finish(events, &job, "error", error_details(dbus_error_for(&e), &e.to_string()));
            return false;
        }
    };

    let d = convert::parameters_to_dict(&params);
    emit(events, &job, "started", d);

    // Non-blocking, so a full pipe (an app that is not reading) cannot hold this thread
    // past a Cancel(). Only our end: the app's read end is a separate open file.
    if let Some(w) = &job.out {
        set_nonblocking(w);
    }
    let mut buf = vec![0u8; 64 * 1024];
    let (state, details) = loop {
        if job.cancel.load(Ordering::SeqCst) {
            h.cancel();
            break ("cancelled", Vardict::new());
        }
        match h.read(&mut buf) {
            Ok(0) => {
                if params.last_frame {
                    h.cancel();
                }
                break ("finished", Vardict::new());
            }
            Ok(n) => {
                // The app closing the pipe, or Cancel() while the app is not reading, ends the frame.
                let wrote = job.out.as_mut().is_some_and(|w| write_all_cancellable(w, &buf[..n], &job.cancel));
                if !wrote {
                    h.cancel();
                    break ("cancelled", Vardict::new());
                }
            }
            Err(e) if status_code(&e) == Some(sys::SANE_STATUS_CANCELLED) => break ("cancelled", Vardict::new()),
            Err(e) => {
                h.cancel();
                break ("error", error_details(dbus_error_for(&e), &e.to_string()));
            }
        }
    };
    job.out = None;
    finish(events, &job, state, details);
    state == "finished" && !params.last_frame
}

fn set_nonblocking(w: &PipeWriter) {
    let fd = w.as_raw_fd();
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags >= 0 {
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }
}

/// Write everything to a non-blocking pipe. Returns false if the app closed its end or
/// `cancel` was set while the pipe was full.
fn write_all_cancellable(w: &mut PipeWriter, mut data: &[u8], cancel: &AtomicBool) -> bool {
    while !data.is_empty() {
        match w.write(data) {
            Ok(0) => return false,
            Ok(n) => data = &data[n..],
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                if cancel.load(Ordering::SeqCst) {
                    return false;
                }
                let mut pfd = libc::pollfd { fd: w.as_raw_fd(), events: libc::POLLOUT, revents: 0 };
                unsafe { libc::poll(&mut pfd, 1, 100) };
            }
            Err(_) => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use nv_sane::{FrameFormat, Parameters};
    use std::cell::{Cell, RefCell};

    /// A scanner that behaves like the 8200i: a multi-frame scan (colour, then infrared) must be
    /// continued with `start` alone, and `cancel` throws the scan away so the next `start` begins
    /// again at the first frame. (SANE's test backend is forgiving about this; real drivers are not.)
    struct Strict {
        frames: Vec<FrameFormat>,
        next: Cell<usize>,
        log: RefCell<Vec<String>>,
    }

    impl ScannerHandle for Strict {
        fn all_options(&self) -> nv_sane::Result<Vec<OptionDescriptor>> {
            Ok(vec![])
        }
        fn get_option(&self, _: &OptionDescriptor) -> nv_sane::Result<OptionValue> {
            Ok(OptionValue::Button)
        }
        fn set_option(&self, _: &OptionDescriptor, _: &OptionValue) -> nv_sane::Result<SetInfo> {
            Ok(SetInfo::default())
        }
        fn get_parameters(&self) -> nv_sane::Result<Parameters> {
            let i = self.next.get();
            Ok(Parameters { format: self.frames[i].clone(), last_frame: i + 1 == self.frames.len(), bytes_per_line: 4, pixels_per_line: 4, lines: Some(2), depth: 8 })
        }
        fn start(&self) -> nv_sane::Result<()> {
            self.log.borrow_mut().push(format!("start:{:?}", self.frames[self.next.get()]));
            Ok(())
        }
        fn read(&self, buf: &mut [u8]) -> nv_sane::Result<usize> {
            // One short frame, then end of frame; after the last frame the scan is over.
            let mut log = self.log.borrow_mut();
            if log.last().is_some_and(|l| l == "data") {
                log.push("eof".into());
                let i = self.next.get() + 1;
                self.next.set(if i == self.frames.len() { 0 } else { i });
                return Ok(0);
            }
            log.push("data".into());
            buf[..8].fill(1);
            Ok(8)
        }
        fn cancel(&self) {
            self.log.borrow_mut().push("cancel".into());
            self.next.set(0);
        }
    }

    fn job() -> (FrameJob, std::io::PipeReader) {
        let (r, w) = std::io::pipe().unwrap();
        (FrameJob { sid: 1, serial: 1, path: "/s".into(), owner: ":1.1".into(), out: Some(w), cancel: Arc::new(AtomicBool::new(false)), busy: Arc::new(AtomicBool::new(true)) }, r)
    }

    #[test]
    fn a_multi_frame_scan_is_continued_not_restarted() {
        let h = Strict { frames: vec![FrameFormat::Rgb, FrameFormat::Gray], next: Cell::new(0), log: RefCell::new(vec![]) };
        let (tx, _rx) = async_channel::unbounded();
        let mut mid = false;
        let mut seen = Vec::new();
        for _ in 0..2 {
            let (j, _r) = job();
            mid = run_frame(&h, j, &tx, mid);
            seen.push(mid);
        }
        let log = h.log.borrow().clone();
        // Colour then infrared, in order: a cancel between them would have given colour twice.
        let starts: Vec<&String> = log.iter().filter(|l| l.starts_with("start")).collect();
        assert_eq!(starts, ["start:Rgb", "start:Gray"], "{log:?}");
        assert_eq!(seen, [true, false], "more frames follow the first, none the second");
        // One cancel to clear before the scan, one to end it after the last frame; none between.
        let cancels: Vec<usize> = log.iter().enumerate().filter(|(_, l)| *l == "cancel").map(|(i, _)| i).collect();
        let second_start = log.iter().position(|l| l == "start:Gray").unwrap();
        assert!(cancels.iter().all(|c| *c < log.iter().position(|l| l == "start:Rgb").unwrap() || *c > second_start), "cancel between the frames: {log:?}");
    }

    #[test]
    fn a_new_scan_after_the_last_frame_starts_from_the_first() {
        let h = Strict { frames: vec![FrameFormat::Rgb, FrameFormat::Gray], next: Cell::new(0), log: RefCell::new(vec![]) };
        let (tx, _rx) = async_channel::unbounded();
        let mut mid = false;
        for _ in 0..4 {
            let (j, _r) = job();
            mid = run_frame(&h, j, &tx, mid);
        }
        let starts: Vec<String> = h.log.borrow().iter().filter(|l| l.starts_with("start")).cloned().collect();
        assert_eq!(starts, ["start:Rgb", "start:Gray", "start:Rgb", "start:Gray"]);
    }
}
