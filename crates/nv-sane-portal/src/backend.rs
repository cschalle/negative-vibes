//! `ScannerBackend` and `ScannerHandle` over the scanner portal.
//!
//! The portal is async D-Bus; the traits are blocking, like libsane. Each call waits with
//! `zbus::block_on`. SANE's sequence is kept: `start` waits for the portal's `started`
//! event (the scanner is ready and the frame layout is known), `read` returns pipe data,
//! and the end of the pipe is only taken as the end of the frame once the portal says
//! `finished`.

use crate::proxy::{RequestProxy, ScannerProxy, SessionProxy};
use crate::{PORTAL_PATH, request_path};
use futures_util::future::{Either, select};
use futures_util::{FutureExt, Stream, StreamExt};
use nv_sane::{ButtonPress, DeviceInfo, FrameFormat, OptionConstraint, OptionDescriptor, OptionKind, OptionUnit, OptionValue, Parameters, SaneError, ScannerBackend, ScannerHandle, SetInfo};
use nv_sane_sys as sys;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

type Result<T> = std::result::Result<T, SaneError>;
type Vardict = HashMap<String, OwnedValue>;

/// How long `sane_start` may take: calibration and carriage moves are slow.
const START_TIMEOUT: Duration = Duration::from_secs(180);
/// How long the portal may take to report the end of a frame once its data has ended.
const END_TIMEOUT: Duration = Duration::from_secs(15);

fn status(code: u32, message: impl Into<String>) -> SaneError {
    SaneError::Status { code, message: message.into() }
}

/// Map a portal D-Bus error to the SANE condition the worker already understands.
fn map_error(e: zbus::Error) -> SaneError {
    if let zbus::Error::MethodError(name, desc, _) = &e {
        let message = desc.clone().unwrap_or_else(|| name.to_string());
        return match name.as_str().rsplit('.').next().unwrap_or("") {
            "InvalidArgument" => status(sys::SANE_STATUS_INVAL, message),
            "NotAllowed" | "AccessDenied" => status(sys::SANE_STATUS_ACCESS_DENIED, message),
            "Busy" => status(sys::SANE_STATUS_DEVICE_BUSY, message),
            "Failed" => status(sys::SANE_STATUS_IO_ERROR, message),
            _ => SaneError::Portal(message),
        };
    }
    SaneError::Portal(e.to_string())
}

fn portal_err(e: impl std::fmt::Display) -> SaneError {
    SaneError::Portal(e.to_string())
}

fn str_of(m: &Vardict, key: &str) -> String {
    m.get(key).and_then(|v| <&str>::try_from(v).ok()).unwrap_or("").to_owned()
}

fn strings_of(m: &Vardict, key: &str) -> Vec<String> {
    m.get(key).and_then(|v| v.try_clone().ok()).and_then(|v| Vec::<String>::try_from(v).ok()).unwrap_or_default()
}

fn device_info(id: &str, info: &Vardict) -> DeviceInfo {
    // The portal's id stands in for SANE's device name: it is what `open` takes. `type` is SANE's
    // device type string, as with libsane (the connection `kind` is only a fallback for a portal
    // that does not send it). The backend name comes from the portal because quirks are matched
    // on it and the id is opaque.
    let type_ = str_of(info, "type");
    let kind = if type_.is_empty() { str_of(info, "kind") } else { type_ };
    DeviceInfo { name: id.to_owned(), backend: str_of(info, "backend"), vendor: str_of(info, "vendor"), model: str_of(info, "model"), kind }
}

pub struct PortalBackend {
    proxy: ScannerProxy<'static>,
    bus_name: String,
    request_counter: AtomicU32,
}

impl PortalBackend {
    /// Connect to the session bus and the portal at `bus_name`
    /// (`org.freedesktop.portal.Desktop` once this is upstream).
    pub fn connect(bus_name: &str) -> Result<Self> {
        zbus::block_on(async {
            let conn = zbus::Connection::session().await.map_err(portal_err)?;
            let proxy =
                ScannerProxy::builder(&conn).destination(bus_name.to_owned()).map_err(portal_err)?.path(PORTAL_PATH).map_err(portal_err)?.build().await.map_err(portal_err)?;
            // Fail now, with a clear error, if nothing answers.
            proxy.version().await.map_err(|e| SaneError::Portal(format!("no scanner portal at {bus_name}: {e}")))?;
            Ok(Self { proxy, bus_name: bus_name.to_owned(), request_counter: AtomicU32::new(1) })
        })
    }

    /// Ask the portal to show its scanner selection dialog and return what the user chose.
    /// The grant is remembered by the portal, so later `get_devices` calls list it. The dialog only
    /// offers scanners that have all of `required_capabilities`, as the portal names them.
    pub fn select_scanners(&self, parent_window: &str, multiple: bool, required_capabilities: &[&str]) -> Result<Vec<DeviceInfo>> {
        zbus::block_on(async {
            let conn = self.proxy.inner().connection().clone();
            let me = conn.unique_name().ok_or_else(|| portal_err("no unique bus name"))?.to_string();
            let token = format!("nvsel{}", self.request_counter.fetch_add(1, Ordering::SeqCst));
            // Subscribe at the predicted Request path before calling, so Response cannot be missed.
            let rpath = request_path(&me, &token);
            let request =
                RequestProxy::builder(&conn).destination(self.bus_name.clone()).map_err(portal_err)?.path(rpath.clone()).map_err(portal_err)?.build().await.map_err(portal_err)?;
            let mut responses = request.receive_response().await.map_err(portal_err)?;

            let mut opts: HashMap<&str, Value<'_>> = HashMap::new();
            opts.insert("handle_token", token.as_str().into());
            opts.insert("multiple", multiple.into());
            if !required_capabilities.is_empty() {
                opts.insert("required_capabilities", required_capabilities.to_vec().into());
            }
            self.proxy.select_scanners(parent_window, opts).await.map_err(map_error)?;

            let resp = responses.next().await.ok_or_else(|| portal_err("no Response from the portal"))?;
            let args = resp.args().map_err(portal_err)?;
            match *args.response() {
                0 => {}
                1 => return Ok(Vec::new()), // the user cancelled
                other => return Err(portal_err(format!("SelectScanners ended with response {other}"))),
            }
            let Some(list) = args.results().get("scanners") else { return Ok(Vec::new()) };
            let list = Vec::<(String, Vardict)>::try_from(list.try_clone().map_err(portal_err)?).map_err(portal_err)?;
            Ok(list.iter().map(|(id, info)| device_info(id, info)).collect())
        })
    }
}

impl ScannerBackend for PortalBackend {
    /// Scanners this app was granted. Only lists: choosing is [`select_devices`](Self::select_devices),
    /// because it needs a window. `local_only` is ignored: the portal decides what the app may see.
    fn get_devices(&self, _local_only: bool) -> Result<Vec<DeviceInfo>> {
        let granted = zbus::block_on(self.proxy.list_scanners(HashMap::new())).map_err(map_error)?;
        Ok(granted.iter().map(|(id, info)| device_info(id, info)).collect())
    }

    fn select_devices(&self, parent_window: &str) -> Result<Vec<DeviceInfo>> {
        // Negative Vibes scans film, so a flatbed or a document feeder is of no use to it
        self.select_scanners(parent_window, true, &["film"])
    }

    fn open(&self, name: &str) -> Result<Box<dyn ScannerHandle>> {
        zbus::block_on(async {
            let session = self.proxy.open_scanner(name, HashMap::new()).await.map_err(map_error)?;
            let conn = self.proxy.inner().connection().clone();
            let close = SessionProxy::builder(&conn)
                .destination(self.bus_name.clone())
                .map_err(portal_err)?
                .path(session.clone())
                .map_err(portal_err)?
                .build()
                .await
                .map_err(portal_err)?;
            // Subscribe before any StartFrame so no event is lost.
            let events = self
                .proxy
                .receive_frame_event()
                .await
                .map_err(portal_err)?
                .map(|m| {
                    let a = m.args().ok();
                    Event {
                        session: a.as_ref().map(|a| a.session_handle().to_string()).unwrap_or_default(),
                        state: a.as_ref().map(|a| a.state().to_string()).unwrap_or_default(),
                        details: a.map(|a| a.details().iter().filter_map(|(k, v)| Some((k.to_string(), v.try_to_owned().ok()?))).collect()).unwrap_or_default(),
                    }
                })
                .boxed_local();
            let buttons = self
                .proxy
                .receive_button_event()
                .await
                .map_err(portal_err)?
                .filter_map(|m| async move {
                    let a = m.args().ok()?;
                    let option = a.details().get("option").and_then(|v| String::try_from(v.try_to_owned().ok()?).ok()).unwrap_or_default();
                    Some((a.session_handle().to_string(), ButtonPress { button: a.button().to_string(), option }))
                })
                .boxed_local();
            let handle: Box<dyn ScannerHandle> = Box::new(PortalHandle {
                buttons: RefCell::new(buttons),
                proxy: self.proxy.clone(),
                close,
                session,
                events: RefCell::new(events),
                values: RefCell::new(None),
                frame: RefCell::new(None),
                active: Cell::new(false),
                done_frame: Cell::new(0),
                mid_scan: Cell::new(false),
                params: RefCell::new(None),
            });
            Ok(handle)
        })
    }
}

struct Event {
    session: String,
    state: String,
    details: Vardict,
}

/// An open scanner session on the portal.
pub struct PortalHandle {
    proxy: ScannerProxy<'static>,
    close: SessionProxy<'static>,
    session: OwnedObjectPath,
    events: RefCell<Pin<Box<dyn Stream<Item = Event>>>>,
    /// ButtonEvents of every session of this app, as (session, press)
    buttons: RefCell<Pin<Box<dyn Stream<Item = (String, ButtonPress)>>>>,
    /// The values of all options as `GetOptionValues` gave them, until an option changes.
    /// The portal returns every value in one call, and the app reads the options one by one.
    values: RefCell<Option<HashMap<String, OwnedValue>>>,
    /// Read end of the pipe for the frame in progress.
    frame: RefCell<Option<File>>,
    /// A frame was started and its end has not been seen yet.
    active: Cell<bool>,
    /// Number of the latest frame whose end was seen. Events numbered at or below it belong to
    /// a frame that is over and are dropped.
    done_frame: Cell<u32>,
    /// The last frame finished but its scan has more (colour then infrared; R, G, B). The next
    /// `start` continues that scan; `cancel` between the frames abandons it.
    mid_scan: Cell<bool>,
    params: RefCell<Option<Parameters>>,
}

impl PortalHandle {
    fn path(&self) -> ObjectPath<'_> {
        self.session.as_ref()
    }

    /// Next FrameEvent for this session, or `None` on timeout or if the portal is gone.
    fn next_event(&self, timeout: Duration) -> Option<Event> {
        let mut events = self.events.borrow_mut();
        loop {
            let ev = zbus::block_on(async {
                match select(events.next(), Box::pin(async_io::Timer::after(timeout))).await {
                    Either::Left((ev, _)) => ev,
                    Either::Right(_) => None,
                }
            })?;
            if ev.session != self.session.as_str() {
                continue;
            }
            let serial = ev.details.get("frame").and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
            if serial != 0 && serial <= self.done_frame.get() {
                continue;
            }
            if serial != 0 && matches!(ev.state.as_str(), "finished" | "cancelled" | "error" | "no-documents") {
                self.done_frame.set(serial);
            }
            return Some(ev);
        }
    }

    /// Throw away events left over from an earlier frame.
    fn drain_stale_events(&self) {
        let mut events = self.events.borrow_mut();
        while events.next().now_or_never().flatten().is_some() {}
    }

    fn end_frame(&self) {
        // The next get_parameters describes the next frame, not this one.
        *self.params.borrow_mut() = None;
        self.active.set(false);
        *self.frame.borrow_mut() = None;
    }

    /// Turn a non-`started`, non-`finished` frame state into the SANE error the worker expects.
    fn state_error(ev: &Event) -> SaneError {
        let message = str_of(&ev.details, "message");
        match ev.state.as_str() {
            "no-documents" => status(sys::SANE_STATUS_NO_DOCS, "no documents (feeder or film holder empty)"),
            "cancelled" => status(sys::SANE_STATUS_CANCELLED, "cancelled"),
            "error" => match str_of(&ev.details, "code").rsplit('.').next().unwrap_or("") {
                "NotAllowed" => status(sys::SANE_STATUS_ACCESS_DENIED, message),
                "InvalidArgument" => status(sys::SANE_STATUS_INVAL, message),
                _ => status(sys::SANE_STATUS_IO_ERROR, message),
            },
            other => SaneError::Portal(format!("unexpected frame state '{other}'")),
        }
    }
}

impl Drop for PortalHandle {
    fn drop(&mut self) {
        // Closing the session cancels a running frame and releases the scanner.
        let _ = zbus::block_on(self.close.close());
    }
}

fn kind_of(s: &str) -> Option<OptionKind> {
    Some(match s {
        "bool" => OptionKind::Bool,
        "int" => OptionKind::Int,
        "fixed" => OptionKind::Fixed,
        "string" => OptionKind::String,
        "button" => OptionKind::Button,
        "group" => OptionKind::Group,
        _ => return None,
    })
}

fn unit_of(s: &str) -> OptionUnit {
    match s {
        "pixel" => OptionUnit::Pixel,
        "bit" => OptionUnit::Bit,
        "mm" => OptionUnit::Mm,
        "dpi" => OptionUnit::Dpi,
        "percent" => OptionUnit::Percent,
        "microsecond" => OptionUnit::Microsecond,
        _ => OptionUnit::None,
    }
}

fn cap_bits(caps: &[String]) -> i32 {
    caps.iter()
        .map(|c| match c.as_str() {
            "settable" => sys::SANE_CAP_SOFT_SELECT,
            "readable" => sys::SANE_CAP_SOFT_DETECT,
            "automatic" => sys::SANE_CAP_AUTOMATIC,
            "inactive" => sys::SANE_CAP_INACTIVE,
            "advanced" => sys::SANE_CAP_ADVANCED,
            "emulated" => sys::SANE_CAP_EMULATED,
            "hardware" => sys::SANE_CAP_HARD_SELECT,
            _ => 0,
        })
        .fold(0, |a, b| a | b)
}

/// `(sv)`: constraint kind and its data.
fn constraint_of(d: &Vardict, kind: &OptionKind) -> OptionConstraint {
    let Some(Value::Structure(s)) = d.get("constraint").map(|v| &**v) else { return OptionConstraint::None };
    let [Value::Str(ck), data] = s.fields() else { return OptionConstraint::None };
    let data = match data {
        Value::Value(inner) => &**inner,
        other => other,
    };
    match ck.as_str() {
        "range" => match data {
            Value::Structure(r) => match r.fields().iter().map(|f| f64::try_from(f).ok()).collect::<Vec<_>>()[..] {
                [Some(min), Some(max), Some(quant)] => OptionConstraint::Range { min, max, quant },
                _ => OptionConstraint::None,
            },
            _ => OptionConstraint::None,
        },
        "list" => match kind {
            OptionKind::Int => Vec::<i32>::try_from(data.clone()).map(|v| OptionConstraint::WordList(v.into_iter().map(f64::from).collect())).unwrap_or(OptionConstraint::None),
            OptionKind::Fixed => Vec::<f64>::try_from(data.clone()).map(OptionConstraint::WordList).unwrap_or(OptionConstraint::None),
            OptionKind::String => Vec::<String>::try_from(data.clone()).map(OptionConstraint::StringList).unwrap_or(OptionConstraint::None),
            _ => OptionConstraint::None,
        },
        _ => OptionConstraint::None,
    }
}

/// The portal's options have no index; position stands in for SANE's slot number.
fn descriptor_of(d: &Vardict, index: usize) -> Option<OptionDescriptor> {
    let kind = kind_of(&str_of(d, "type"))?;
    let size = match kind {
        OptionKind::String => 256,
        OptionKind::Bool | OptionKind::Int | OptionKind::Fixed => 4,
        _ => 0,
    };
    Some(OptionDescriptor {
        index,
        name: str_of(d, "name"),
        title: str_of(d, "title"),
        desc: str_of(d, "description"),
        unit: unit_of(&str_of(d, "unit")),
        size,
        cap: cap_bits(&strings_of(d, "capabilities")),
        constraint: constraint_of(d, &kind),
        kind,
    })
}

fn value_to_variant(v: &OptionValue) -> Value<'static> {
    match v {
        OptionValue::Bool(b) => Value::from(*b),
        OptionValue::Int(i) => Value::from(*i),
        OptionValue::Fixed(f) => Value::from(*f),
        OptionValue::String(s) => Value::from(s.clone()),
        OptionValue::Button => Value::from(true),
    }
}

fn value_from_variant(kind: &OptionKind, v: &OwnedValue) -> Option<OptionValue> {
    match kind {
        OptionKind::Bool => bool::try_from(v).ok().map(OptionValue::Bool),
        OptionKind::Int => i32::try_from(v).ok().map(OptionValue::Int),
        OptionKind::Fixed => f64::try_from(v).ok().map(OptionValue::Fixed),
        OptionKind::String => <&str>::try_from(v).ok().map(|s| OptionValue::String(s.to_owned())),
        OptionKind::Button | OptionKind::Group => Some(OptionValue::Button),
    }
}

fn format_of(s: &str) -> Option<FrameFormat> {
    Some(match s {
        "gray" => FrameFormat::Gray,
        "rgb" => FrameFormat::Rgb,
        "red" => FrameFormat::Red,
        "green" => FrameFormat::Green,
        "blue" => FrameFormat::Blue,
        "rgbi" => FrameFormat::Rgbi,
        _ => return None,
    })
}

fn parameters_of(d: &Vardict) -> Option<Parameters> {
    let u = |k: &str| d.get(k).and_then(|v| u32::try_from(v).ok());
    let lines = d.get("lines").and_then(|v| i32::try_from(v).ok())?;
    Some(Parameters {
        format: format_of(&str_of(d, "format"))?,
        last_frame: d.get("last_frame").and_then(|v| bool::try_from(v).ok())?,
        bytes_per_line: u("bytes_per_line")? as usize,
        pixels_per_line: u("pixels_per_line")? as usize,
        lines: (lines >= 0).then_some(lines as usize),
        depth: u("depth")? as u8,
    })
}

impl ScannerHandle for PortalHandle {
    fn all_options(&self) -> Result<Vec<OptionDescriptor>> {
        let list = zbus::block_on(self.proxy.get_options(&self.path(), HashMap::new())).map_err(map_error)?;
        Ok(list.iter().enumerate().filter_map(|(i, d)| descriptor_of(d, i + 1)).collect())
    }

    fn get_option(&self, desc: &OptionDescriptor) -> Result<OptionValue> {
        if matches!(desc.kind, OptionKind::Button | OptionKind::Group) {
            return Ok(OptionValue::Button);
        }
        if self.values.borrow().is_none() {
            let all = zbus::block_on(self.proxy.get_option_values(&self.path(), HashMap::new())).map_err(map_error)?;
            *self.values.borrow_mut() = Some(all);
        }
        let values = self.values.borrow();
        values
            .as_ref()
            .and_then(|v| v.get(&desc.name))
            .and_then(|v| value_from_variant(&desc.kind, v))
            .ok_or_else(|| status(sys::SANE_STATUS_INVAL, format!("option '{}' has no readable value", desc.name)))
    }

    fn set_option(&self, desc: &OptionDescriptor, value: &OptionValue) -> Result<SetInfo> {
        *self.params.borrow_mut() = None;
        *self.values.borrow_mut() = None;
        let (_applied, flags) = zbus::block_on(self.proxy.set_options(&self.path(), vec![(desc.name.as_str(), value_to_variant(value))], HashMap::new())).map_err(map_error)?;
        Ok(SetInfo {
            inexact: flags.iter().any(|f| f == "inexact"),
            reload_options: flags.iter().any(|f| f == "reload-options"),
            reload_params: flags.iter().any(|f| f == "reload-params"),
        })
    }

    fn set_option_auto(&self, desc: &OptionDescriptor) -> Result<SetInfo> {
        *self.params.borrow_mut() = None;
        *self.values.borrow_mut() = None;
        let auto = Value::from(HashMap::from([("auto", Value::from(true))]));
        let (_applied, flags) = zbus::block_on(self.proxy.set_options(&self.path(), vec![(desc.name.as_str(), auto)], HashMap::new())).map_err(map_error)?;
        Ok(SetInfo {
            inexact: flags.iter().any(|f| f == "inexact"),
            reload_options: flags.iter().any(|f| f == "reload-options"),
            reload_params: flags.iter().any(|f| f == "reload-params"),
        })
    }

    /// The final values once a frame has started (from its `started` event), otherwise the
    /// portal's estimate for the current options, as libsane gives before `sane_start`.
    fn get_parameters(&self) -> Result<Parameters> {
        if let Some(p) = self.params.borrow().clone() {
            return Ok(p);
        }
        let d = zbus::block_on(self.proxy.get_frame_parameters(&self.path(), HashMap::new())).map_err(map_error)?;
        parameters_of(&d).ok_or_else(|| SaneError::Portal("malformed frame parameters".into()))
    }

    fn start(&self) -> Result<()> {
        if self.active.get() {
            self.cancel();
        }
        self.drain_stale_events();
        *self.params.borrow_mut() = None;
        *self.values.borrow_mut() = None;

        let fd = zbus::block_on(self.proxy.start_frame(&self.path(), HashMap::new())).map_err(map_error)?;
        *self.frame.borrow_mut() = Some(File::from(std::os::fd::OwnedFd::from(fd)));
        self.active.set(true);

        match self.next_event(START_TIMEOUT) {
            Some(ev) if ev.state == "started" => {
                let p = parameters_of(&ev.details).ok_or_else(|| SaneError::Portal("malformed 'started' event".into()))?;
                *self.params.borrow_mut() = Some(p);
                Ok(())
            }
            Some(ev) => {
                self.end_frame();
                self.mid_scan.set(false);
                Err(Self::state_error(&ev))
            }
            None => {
                // Timed out or the portal vanished: stop the frame rather than leave it running.
                self.cancel();
                Err(SaneError::Portal("no answer from the portal while starting a frame".into()))
            }
        }
    }

    /// `Ok(0)` is the end of the frame, once the portal has confirmed it finished.
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        loop {
            let n = match self.frame.borrow_mut().as_mut() {
                Some(f) => f.read(buf),
                None => return Err(SaneError::Portal("no frame in progress".into())),
            };
            match n {
                Ok(0) => break,
                Ok(n) => return Ok(n),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    self.cancel();
                    return Err(status(sys::SANE_STATUS_IO_ERROR, e.to_string()));
                }
            }
        }
        // End of data: it only counts as the end of the frame if the portal says it finished.
        let ev = self.next_event(END_TIMEOUT);
        let more_frames = self.params.borrow().as_ref().is_some_and(|p| !p.last_frame);
        self.end_frame();
        match ev {
            Some(ev) if ev.state == "finished" => {
                self.mid_scan.set(more_frames);
                Ok(0)
            }
            Some(ev) => Err(Self::state_error(&ev)),
            None => Err(SaneError::Portal("pipe closed but the portal did not report the frame's end".into())),
        }
    }

    /// Cancel the frame in progress, if any, and wait until the portal has let go of it so
    /// the next `start` is not refused. Does nothing when no frame is running.
    fn next_button(&self, timeout: Duration) -> Option<ButtonPress> {
        let mut buttons = self.buttons.borrow_mut();
        let end = std::time::Instant::now() + timeout;
        loop {
            let left = end.saturating_duration_since(std::time::Instant::now());
            let (session, press) = zbus::block_on(async {
                match select(buttons.next(), Box::pin(async_io::Timer::after(left))).await {
                    Either::Left((press, _)) => press,
                    Either::Right(_) => None,
                }
            })?;
            if session == self.session.as_str() {
                return Some(press);
            }
        }
    }

    fn cancel(&self) {
        if !self.active.get() {
            // Between the frames of a multi-frame scan this abandons the scan, so the next
            // `start` begins a new one. With nothing in progress there is nothing to do.
            if self.mid_scan.replace(false) {
                let _ = zbus::block_on(self.proxy.cancel(&self.path()));
            }
            return;
        }
        self.mid_scan.set(false);
        let _ = zbus::block_on(self.proxy.cancel(&self.path()));
        // Closing our end cancels as well, and unblocks the portal if it is waiting to write.
        *self.frame.borrow_mut() = None;
        while let Some(ev) = self.next_event(END_TIMEOUT) {
            if matches!(ev.state.as_str(), "finished" | "cancelled" | "error" | "no-documents") {
                break;
            }
        }
        self.active.set(false);
    }
}
