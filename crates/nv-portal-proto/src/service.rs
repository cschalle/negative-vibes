//! The D-Bus side: `org.freedesktop.portal.Scanner` plus the Request and Session objects
//! it hands out. Stubbed here, real in the upstream portal: the permission store (grants
//! live in memory, shared by all apps) and the selection dialog (auto-selects).

use crate::convert::{self, Vardict};
use crate::curate::{self, Scanner as ScannerInfo};
use crate::sane_thread::{Cmd, FrameEvent, FrameJob, Reply, SetError, Setting};
use crate::{PORTAL_PATH, REQUEST_INTERFACE, SCANNER_INTERFACE, SESSION_INTERFACE, request_path, session_path};
use nv_sane::DeviceInfo;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use zbus::message::Header;
use zbus::zvariant::{ObjectPath, OwnedFd, OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, ObjectServer};

// The prefix is shared by all variants, so each variant names its own path below it: most are
// portal errors, but a session that is not the caller's is a D-Bus one, as in other portals.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop")]
pub enum PortalError {
    #[zbus(error)]
    ZBus(zbus::Error),
    #[zbus(name = "portal.Error.Failed")]
    Failed(String),
    #[zbus(name = "portal.Error.InvalidArgument")]
    InvalidArgument(String),
    #[zbus(name = "portal.Error.NotAllowed")]
    NotAllowed(String),
    /// The scanner or session is in use. Not in xdg-desktop-portal's error set yet.
    #[zbus(name = "portal.Error.Busy")]
    Busy(String),
    /// A session that does not exist, has closed, or belongs to another application.
    #[zbus(name = "DBus.Error.AccessDenied")]
    AccessDenied(String),
}

fn access_denied(message: &str) -> PortalError {
    PortalError::AccessDenied(message.to_owned())
}

/// What the stub dialog picks.
pub struct Policy {
    /// Only offer devices whose SANE name contains this (the dialog stand-in).
    pub select: Option<String>,
    /// Include network scanners (slow: mDNS).
    pub network: bool,
}

#[derive(Clone)]
struct SessionRec {
    owner: String,
    /// `None` for a monitoring session from CreateSession.
    scanner: Option<ScannerRec>,
}

#[derive(Clone)]
struct ScannerRec {
    sid: u64,
    id: String,
    busy: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    /// Frames started in this session; the number of the latest is the `frame` of its events.
    frames: Arc<AtomicU32>,
}

pub struct Shared {
    cmd: mpsc::Sender<Cmd>,
    policy: Policy,
    /// Granted scanners by opaque id, with the device the id stands for.
    granted: Mutex<HashMap<String, ScannerInfo>>,
    sessions: Mutex<HashMap<String, SessionRec>>,
    /// Scanner ids open right now: the exclusive lock.
    open_ids: Mutex<HashSet<String>>,
    counter: AtomicU64,
}

impl Shared {
    pub fn new(cmd: mpsc::Sender<Cmd>, policy: Policy) -> Arc<Self> {
        Arc::new(Self { cmd, policy, granted: Default::default(), sessions: Default::default(), open_ids: Default::default(), counter: AtomicU64::new(1) })
    }

    fn token(&self, prefix: &str) -> String {
        format!("{prefix}{}", self.counter.fetch_add(1, Ordering::SeqCst))
    }

    async fn ask<T>(&self, make: impl FnOnce(Reply<T>) -> Cmd) -> Result<T, PortalError> {
        let (tx, rx) = async_channel::bounded(1);
        self.cmd.send(make(tx)).map_err(|_| PortalError::Failed("SANE thread is gone".into()))?;
        rx.recv().await.map_err(|_| PortalError::Failed("SANE thread is gone".into()))
    }

    /// One scanner for each physical scanner, however many drivers found it
    async fn devices(&self) -> Result<Vec<ScannerInfo>, PortalError> {
        let local_only = !self.policy.network;
        let devs = self.ask(|reply| Cmd::Devices { local_only, reply }).await?.map_err(PortalError::Failed)?;
        let devs: Vec<DeviceInfo> = devs.into_iter().filter(convert::is_scanner).collect();
        Ok(curate::curate(&devs))
    }

    /// The session, if `sender` owns it. Others get the same error as for a missing one.
    fn session(&self, path: &ObjectPath<'_>, sender: &str) -> Result<SessionRec, PortalError> {
        match self.sessions.lock().unwrap().get(path.as_str()) {
            Some(rec) if rec.owner == sender => Ok(rec.clone()),
            _ => Err(access_denied("Invalid session")),
        }
    }

    fn scanner_session(&self, path: &ObjectPath<'_>, sender: &str) -> Result<ScannerRec, PortalError> {
        self.session(path, sender)?.scanner.ok_or_else(|| PortalError::InvalidArgument("not a scanner session".into()))
    }

    /// End a session: cancel its frame, release the scanner, drop the object, tell the owner.
    pub async fn close_session(&self, conn: &Connection, path: &str, notify: bool) {
        let Some(rec) = self.sessions.lock().unwrap().remove(path) else { return };
        if let Some(s) = &rec.scanner {
            s.cancel.store(true, Ordering::SeqCst);
            let _ = self.cmd.send(Cmd::Close { sid: s.sid });
            self.open_ids.lock().unwrap().remove(&s.id);
        }
        let _ = conn.object_server().remove::<SessionObj, _>(path).await;
        if notify {
            let details: HashMap<&str, Value<'_>> = HashMap::new();
            let _ = conn.emit_signal(Some(rec.owner.as_str()), path, SESSION_INTERFACE, "Closed", &(details,)).await;
        }
    }

    /// An app left the bus: release everything it held.
    pub async fn close_all_for(&self, conn: &Connection, owner: &str) {
        let paths: Vec<String> = self.sessions.lock().unwrap().iter().filter(|(_, r)| r.owner == owner).map(|(p, _)| p.clone()).collect();
        for p in paths {
            eprintln!("nv-portal-proto: {owner} left the bus, closing {p}");
            self.close_session(conn, &p, false).await;
        }
    }
}

/// Forward FrameEvent signals from the SANE thread to the app that owns the session.
pub async fn forward_events(conn: Connection, rx: async_channel::Receiver<FrameEvent>) {
    while let Ok(ev) = rx.recv().await {
        let Ok(path) = ObjectPath::try_from(ev.session_path.as_str()) else { continue };
        let _ = conn.emit_signal(Some(ev.owner.as_str()), PORTAL_PATH, SCANNER_INTERFACE, "FrameEvent", &(path, ev.state, ev.details)).await;
    }
}

fn sender_of(hdr: &Header<'_>) -> Result<String, PortalError> {
    hdr.sender().map(|s| s.to_string()).ok_or_else(|| PortalError::Failed("message has no sender".into()))
}

fn opt_str(options: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    options.get(key).and_then(|v| <&str>::try_from(v).ok()).map(str::to_owned)
}

fn opt_bool(options: &HashMap<String, OwnedValue>, key: &str) -> Option<bool> {
    options.get(key).and_then(|v| bool::try_from(v).ok())
}

fn valid_token(t: &str) -> bool {
    !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn to_path(p: String) -> Result<OwnedObjectPath, PortalError> {
    OwnedObjectPath::try_from(p).map_err(|e| PortalError::Failed(e.to_string()))
}

pub struct Scanner(pub Arc<Shared>);

#[zbus::interface(name = "org.freedesktop.portal.Scanner")]
impl Scanner {
    async fn select_scanners(
        &self,
        _parent_window: &str,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(connection)] conn: &Connection,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> Result<OwnedObjectPath, PortalError> {
        let sender = sender_of(&hdr)?;
        let token = opt_str(&options, "handle_token").unwrap_or_else(|| self.0.token("req"));
        if !valid_token(&token) {
            return Err(PortalError::InvalidArgument("bad handle_token".into()));
        }
        let path = request_path(&sender, &token);
        if !server.at(path.as_str(), RequestObj).await? {
            return Err(PortalError::InvalidArgument("handle_token already in use".into()));
        }

        let multiple = opt_bool(&options, "multiple").unwrap_or(false);
        let strv = |key: &str| -> Vec<String> { options.get(key).and_then(|v| Vec::<String>::try_from(v.try_clone().ok()?).ok()).unwrap_or_default() };
        let required = strv("required_capabilities");
        let any = strv("any_capabilities");

        let shared = self.0.clone();
        let conn2 = conn.clone();
        let path2 = path.clone();
        conn.executor()
            .spawn(
                async move {
                    // The dialog stand-in: offer scanners, auto-pick per the policy.
                    let mut picked = Vec::new();
                    let response = match shared.devices().await {
                        Ok(devs) => {
                            for d in devs {
                                let wanted = shared.policy.select.as_ref().is_none_or(|s| d.candidates.iter().any(|c| c.contains(s.as_str())));
                                if wanted && convert::wants_capabilities(&d, &required, &any) {
                                    picked.push(d);
                                }
                            }
                            if !multiple {
                                picked.truncate(1);
                            }
                            0u32
                        }
                        Err(e) => {
                            eprintln!("nv-portal-proto: SelectScanners: {e:?}");
                            2u32
                        }
                    };
                    // As in the C frontend: nothing granted is not a success
                    let response = if picked.is_empty() { 2u32 } else { response };
                    let mut out: Vec<(String, Vardict)> = Vec::new();
                    {
                        let mut granted = shared.granted.lock().unwrap();
                        for d in &picked {
                            let id = d.id.clone();
                            out.push((id.clone(), convert::device_to_dict(d)));
                            granted.insert(id, d.clone());
                        }
                    }
                    let mut results: HashMap<&str, Value<'_>> = HashMap::new();
                    if response == 0 {
                        let list: Vec<(String, HashMap<String, Value<'_>>)> =
                            out.iter().map(|(id, d)| (id.clone(), d.iter().map(|(k, v)| (k.clone(), Value::from(v.try_clone().unwrap()))).collect())).collect();
                        results.insert("scanners", Value::from(list));
                    }
                    let _ = conn2.emit_signal(Some(sender.as_str()), path2.as_str(), REQUEST_INTERFACE, "Response", &(response, results)).await;
                    let _ = conn2.object_server().remove::<RequestObj, _>(path2.as_str()).await;
                },
                "select-scanners",
            )
            .detach();
        to_path(path)
    }

    async fn list_scanners(&self, _options: HashMap<String, OwnedValue>) -> Result<Vec<(String, Vardict)>, PortalError> {
        let granted = self.0.granted.lock().unwrap().clone();
        let devs = self.0.devices().await?;
        Ok(devs
            .iter()
            .filter_map(|d| {
                granted.contains_key(&d.id).then(|| (d.id.clone(), convert::device_to_dict(d)))
            })
            .collect())
    }

    async fn create_session(
        &self,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> Result<OwnedObjectPath, PortalError> {
        let sender = sender_of(&hdr)?;
        let token = opt_str(&options, "session_handle_token").unwrap_or_else(|| self.0.token("mon"));
        if !valid_token(&token) {
            return Err(PortalError::InvalidArgument("bad session_handle_token".into()));
        }
        let path = session_path(&sender, &token);
        self.register_session(server, &path, SessionRec { owner: sender, scanner: None }).await?;
        to_path(path)
    }

    async fn open_scanner(
        &self,
        scanner_id: &str,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(connection)] conn: &Connection,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> Result<OwnedObjectPath, PortalError> {
        let sender = sender_of(&hdr)?;
        let dev = self.0.granted.lock().unwrap().get(scanner_id).cloned().ok_or_else(|| PortalError::NotAllowed("scanner was not granted".into()))?;
        let session_token = opt_str(&options, "session_handle_token").unwrap_or_else(|| self.0.token("scanner"));
        if !valid_token(&session_token) {
            return Err(PortalError::InvalidArgument("bad session_handle_token".into()));
        }
        // The call is a request, answered at once: opening takes seconds, and on a network scanner
        // more than D-Bus's default call timeout. The outcome is the Response.
        let token = opt_str(&options, "handle_token").unwrap_or_else(|| self.0.token("req"));
        if !valid_token(&token) {
            return Err(PortalError::InvalidArgument("bad handle_token".into()));
        }
        let path = request_path(&sender, &token);
        if !server.at(path.as_str(), RequestObj).await? {
            return Err(PortalError::InvalidArgument("handle_token already in use".into()));
        }

        let shared = self.0.clone();
        let conn2 = conn.clone();
        let path2 = path.clone();
        let scanner_id = scanner_id.to_owned();
        conn.executor()
            .spawn(
                async move {
                    let outcome = open_session(&shared, &conn2, sender.clone(), &dev, &scanner_id, &session_token).await;
                    let mut results: HashMap<&str, Value<'_>> = HashMap::new();
                    let response = match &outcome {
                        Ok(session) => {
                            results.insert("session_handle", Value::from(session.as_str()));
                            0u32
                        }
                        Err(e) => {
                            let (kind, message) = match e {
                                PortalError::Busy(m) => ("busy", m.clone()),
                                PortalError::Failed(m) | PortalError::NotAllowed(m) | PortalError::InvalidArgument(m) | PortalError::AccessDenied(m) => ("failed", m.clone()),
                                other => ("failed", format!("{other:?}")),
                            };
                            results.insert("error", Value::from(kind));
                            results.insert("message", Value::from(message));
                            2u32
                        }
                    };
                    let _ = conn2.emit_signal(Some(sender.as_str()), path2.as_str(), REQUEST_INTERFACE, "Response", &(response, results)).await;
                    let _ = conn2.object_server().remove::<RequestObj, _>(path2.as_str()).await;
                },
                "open-scanner",
            )
            .detach();
        to_path(path)
    }

    async fn get_options(&self, session_handle: ObjectPath<'_>, _options: HashMap<String, OwnedValue>, #[zbus(header)] hdr: Header<'_>) -> Result<Vec<Vardict>, PortalError> {
        let s = self.0.scanner_session(&session_handle, &sender_of(&hdr)?)?;
        let opts = self.0.ask(|reply| Cmd::Options { sid: s.sid, reply }).await?.map_err(PortalError::Failed)?;
        Ok(opts.iter().filter(|d| convert::is_exposed(d)).map(convert::descriptor_to_dict).collect())
    }

    async fn get_option_values(&self, session_handle: ObjectPath<'_>, _options: HashMap<String, OwnedValue>, #[zbus(header)] hdr: Header<'_>) -> Result<Vardict, PortalError> {
        let s = self.0.scanner_session(&session_handle, &sender_of(&hdr)?)?;
        let vals = self.0.ask(|reply| Cmd::Values { sid: s.sid, reply }).await?.map_err(PortalError::Failed)?;
        Ok(vals.iter().map(|(n, v)| (n.clone(), convert::value_to_dbus(v))).collect())
    }

    async fn set_options(
        &self,
        session_handle: ObjectPath<'_>,
        values: Vec<(String, OwnedValue)>,
        _options: HashMap<String, OwnedValue>,
        #[zbus(header)] hdr: Header<'_>,
    ) -> Result<(Vardict, Vec<String>), PortalError> {
        let s = self.0.scanner_session(&session_handle, &sender_of(&hdr)?)?;
        if s.busy.load(Ordering::SeqCst) {
            return Err(PortalError::Busy("a frame is in progress".into()));
        }
        // The kind of each option decides how a variant is read, so convert on the SANE thread's
        // view of the options: look them up first.
        let opts = self.0.ask(|reply| Cmd::Options { sid: s.sid, reply }).await?.map_err(PortalError::Failed)?;
        let mut conv: Vec<(String, Setting)> = Vec::new();
        for (name, v) in &values {
            let desc = opts.iter().find(|d| d.name == *name && convert::is_exposed(d)).ok_or_else(|| PortalError::InvalidArgument(format!("no option named '{name}'")))?;
            conv.push((name.clone(), convert::dbus_to_setting(desc, v).map_err(PortalError::InvalidArgument)?));
        }
        let out = self.0.ask(|reply| Cmd::Set { sid: s.sid, values: conv, reply }).await?;
        match out {
            Ok(o) => Ok((o.applied.iter().map(|(n, v)| (n.clone(), convert::value_to_dbus(v))).collect(), o.flags.iter().map(|f| f.to_string()).collect())),
            Err(SetError::Invalid(m)) => Err(PortalError::InvalidArgument(m)),
            Err(SetError::Failed(m)) => Err(PortalError::Failed(m)),
        }
    }

    async fn get_frame_parameters(&self, session_handle: ObjectPath<'_>, _options: HashMap<String, OwnedValue>, #[zbus(header)] hdr: Header<'_>) -> Result<Vardict, PortalError> {
        let s = self.0.scanner_session(&session_handle, &sender_of(&hdr)?)?;
        // The SANE thread is busy for the length of a frame, and the `started` event already
        // carries the final values, so there is nothing to estimate then.
        if s.busy.load(Ordering::SeqCst) {
            return Err(PortalError::Busy("a frame is in progress".into()));
        }
        self.0.ask(|reply| Cmd::Params { sid: s.sid, reply }).await?.map_err(PortalError::Failed)
    }

    async fn start_frame(&self, session_handle: ObjectPath<'_>, _options: HashMap<String, OwnedValue>, #[zbus(header)] hdr: Header<'_>) -> Result<OwnedFd, PortalError> {
        let sender = sender_of(&hdr)?;
        let s = self.0.scanner_session(&session_handle, &sender)?;
        if s.busy.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            return Err(PortalError::Busy("a frame is in progress".into()));
        }
        s.cancel.store(false, Ordering::SeqCst);
        let (reader, writer) = match std::io::pipe() {
            Ok(p) => p,
            Err(e) => {
                s.busy.store(false, Ordering::SeqCst);
                return Err(PortalError::Failed(e.to_string()));
            }
        };
        let serial = s.frames.fetch_add(1, Ordering::SeqCst) + 1;
        let job = FrameJob { sid: s.sid, serial, path: session_handle.to_string(), owner: sender, out: Some(writer), cancel: s.cancel.clone(), busy: s.busy.clone() };
        if self.0.cmd.send(Cmd::Frame(job)).is_err() {
            s.busy.store(false, Ordering::SeqCst);
            return Err(PortalError::Failed("SANE thread is gone".into()));
        }
        Ok(OwnedFd::from(std::os::fd::OwnedFd::from(reader)))
    }

    async fn cancel(&self, session_handle: ObjectPath<'_>, #[zbus(header)] hdr: Header<'_>) -> Result<(), PortalError> {
        let s = self.0.scanner_session(&session_handle, &sender_of(&hdr)?)?;
        if s.busy.load(Ordering::SeqCst) {
            s.cancel.store(true, Ordering::SeqCst);
        } else {
            // Between frames of a multi-frame scan this abandons the scan; otherwise nothing to do.
            let _ = self.0.cmd.send(Cmd::Abort { sid: s.sid });
        }
        Ok(())
    }

    #[zbus(signal)]
    async fn frame_event(emitter: &zbus::object_server::SignalEmitter<'_>, session_handle: ObjectPath<'_>, state: &str, details: HashMap<&str, Value<'_>>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn scanner_events(emitter: &zbus::object_server::SignalEmitter<'_>, session_handle: ObjectPath<'_>, events: Vec<(String, String, Vardict)>) -> zbus::Result<()>;

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}

impl Scanner {
    async fn register_session(&self, server: &ObjectServer, path: &str, rec: SessionRec) -> Result<(), PortalError> {
        if !server.at(path, SessionObj { shared: self.0.clone(), path: path.to_owned() }).await? {
            return Err(PortalError::InvalidArgument("session_handle_token already in use".into()));
        }
        self.0.sessions.lock().unwrap().insert(path.to_owned(), rec);
        Ok(())
    }
}

/// Opens a granted scanner for an application and registers its session, giving the path of the
/// session. Another driver is tried only when the one before said it does not drive the scanner:
/// not after a busy scanner, which another driver would only meet too, and not after an I/O error,
/// since the driver may have talked to the device and a second one on a device in an unknown state
/// is how a scanner gets reset.
async fn open_session(shared: &Arc<Shared>, conn: &Connection, owner: String, dev: &crate::curate::Scanner, scanner_id: &str, token: &str) -> Result<String, PortalError> {
    if !shared.open_ids.lock().unwrap().insert(scanner_id.to_owned()) {
        return Err(PortalError::Busy("scanner is in use".into()));
    }
    let mut last: Option<PortalError> = None;
    let mut opened = None;
    for name in &dev.candidates {
        match shared.ask(|reply| Cmd::Open { name: name.clone(), reply }).await {
            Ok(Ok(sid)) => {
                opened = Some(sid);
                break;
            }
            Ok(Err(e)) => {
                let code = match &e {
                    nv_sane::SaneError::Status { code, .. } => Some(*code),
                    _ => None,
                };
                last = Some(match code {
                    Some(c) if c == nv_sane_sys::SANE_STATUS_DEVICE_BUSY => PortalError::Busy(e.to_string()),
                    Some(c) if c == nv_sane_sys::SANE_STATUS_ACCESS_DENIED => PortalError::NotAllowed(e.to_string()),
                    _ => PortalError::Failed(e.to_string()),
                });
                // Only the driver saying it does not drive the scanner sends the search on
                let unsupported = matches!(code, Some(c) if c == nv_sane_sys::SANE_STATUS_INVAL || c == nv_sane_sys::SANE_STATUS_UNSUPPORTED);
                if !unsupported {
                    break;
                }
            }
            Err(e) => {
                last = Some(e);
                break;
            }
        }
    }
    let Some(sid) = opened else {
        shared.open_ids.lock().unwrap().remove(scanner_id);
        return Err(last.unwrap_or_else(|| PortalError::Failed("no driver for the scanner".into())));
    };
    let path = session_path(&owner, token);
    let rec = SessionRec {
        owner,
        scanner: Some(ScannerRec { sid, id: scanner_id.to_owned(), busy: Default::default(), cancel: Default::default(), frames: Default::default() }),
    };
    let registered = conn.object_server().at(path.as_str(), SessionObj { shared: shared.clone(), path: path.clone() }).await;
    if !matches!(registered, Ok(true)) {
        let _ = shared.cmd.send(Cmd::Close { sid });
        shared.open_ids.lock().unwrap().remove(scanner_id);
        return Err(PortalError::InvalidArgument("session_handle_token already in use".into()));
    }
    shared.sessions.lock().unwrap().insert(path.clone(), rec);
    Ok(path)
}

/// `org.freedesktop.portal.Request`: only `Close` and the `Response` signal.
pub struct RequestObj;

#[zbus::interface(name = "org.freedesktop.portal.Request")]
impl RequestObj {
    /// The stub answers at once, so there is nothing left to close.
    async fn close(&self) {}

    #[zbus(signal)]
    async fn response(emitter: &zbus::object_server::SignalEmitter<'_>, response: u32, results: HashMap<&str, Value<'_>>) -> zbus::Result<()>;
}

/// `org.freedesktop.portal.Session`.
pub struct SessionObj {
    shared: Arc<Shared>,
    path: String,
}

#[zbus::interface(name = "org.freedesktop.portal.Session")]
impl SessionObj {
    async fn close(&self, #[zbus(header)] hdr: Header<'_>, #[zbus(connection)] conn: &Connection) -> Result<(), PortalError> {
        let sender = sender_of(&hdr)?;
        let owned = self.shared.sessions.lock().unwrap().get(&self.path).is_some_and(|r| r.owner == sender);
        if !owned {
            return Err(access_denied("Invalid session"));
        }
        self.shared.close_session(conn, &self.path, false).await;
        Ok(())
    }

    #[zbus(signal)]
    async fn closed(emitter: &zbus::object_server::SignalEmitter<'_>, details: HashMap<&str, Value<'_>>) -> zbus::Result<()>;

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}
