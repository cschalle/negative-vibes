//! SANE types to and from the draft interface's D-Bus types.

use crate::curate::Scanner;
use crate::sane_thread::Setting;
use nv_sane::{DeviceInfo, FrameFormat, OptionConstraint, OptionDescriptor, OptionKind, OptionUnit, OptionValue};
use nv_sane_sys as sys;
use std::collections::HashMap;
use zbus::zvariant::{OwnedValue, Value};

pub type Vardict = HashMap<String, OwnedValue>;

/// Make an owned variant. Only fd-carrying values can fail, and we never build those.
pub fn ov<'a>(v: impl Into<Value<'a>>) -> OwnedValue {
    OwnedValue::try_from(v.into()).expect("value without fds")
}

/// Stable opaque id for a SANE device name: apps never see or build SANE names.
pub fn scanner_id(sane_name: &str) -> String {
    // FNV-1a, 64 bit. Not security relevant, only an opaque stable token.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in sane_name.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Prototype heuristic. The real portal would know the connection from the backend.
fn device_kind(name: &str) -> &'static str {
    if name.starts_with("net:") || name.starts_with("airscan:") || name.contains("/net/") || name.contains("ip=") {
        "network"
    } else if name.contains("usb") {
        "usb"
    } else {
        "other"
    }
}

/// Webcams show up through SANE's v4l backend. Type "virtual device" is not used: the
/// SANE test backend has it too, and the portal should offer that for testing.
pub fn is_scanner(dev: &DeviceInfo) -> bool {
    !dev.is_webcam()
}

/// What the device is, in the portal's words: `film` for a scanner that is only for film, `other`
/// for what is not a scanner, such as a camera or SANE's test device, `scanner` for the rest. The
/// driver's own type says little, differs between drivers and can hold an address.
pub fn portal_type(raw: &str, capabilities: &[String]) -> &'static str {
    let raw = raw.to_lowercase();
    let has = |c: &str| capabilities.iter().any(|x| x == c);
    if raw.contains("camera") || raw.contains("webcam") || raw.contains("virtual") {
        "other"
    } else if has("film") && !has("flatbed") && !has("feeder") {
        "film"
    } else {
        "scanner"
    }
}

pub fn device_to_dict(scanner: &Scanner) -> Vardict {
    let dev = &scanner.device;
    let mut d = Vardict::new();
    d.insert("name".into(), ov(format!("{} {}", dev.vendor, dev.model).trim().to_owned()));
    d.insert("vendor".into(), ov(dev.vendor.clone()));
    d.insert("model".into(), ov(dev.model.clone()));
    d.insert("backend".into(), ov(dev.backend.clone()));
    d.insert("kind".into(), ov(device_kind(&dev.name)));
    d.insert("type".into(), ov(portal_type(&dev.kind, &scanner.capabilities)));
    d.insert("capabilities".into(), ov(scanner.capabilities.clone()));
    d.insert("available".into(), ov(true));
    d
}

/// Has every capability in `required`, and, if `any` is not empty, at least one of those
pub fn wants_capabilities(scanner: &Scanner, required: &[String], any: &[String]) -> bool {
    let have = &scanner.capabilities;
    required.iter().all(|r| have.contains(r)) && (any.is_empty() || any.iter().any(|a| have.contains(a)))
}

/// Options an app must not see or set. SANE has no filename option type, so this
/// is a name heuristic; the real list needs a pass over the backends (proposal, open
/// question 3).
fn is_hidden(desc: &OptionDescriptor) -> bool {
    if desc.kind != OptionKind::String {
        return false;
    }
    let n = desc.name.to_lowercase();
    n.contains("file") || n.contains("path") || n.contains("dir")
}

/// Number of SANE words (4 bytes) for numeric options. nv-sane reads one word only, so
/// array-valued options (gamma tables) cannot be carried yet.
fn is_array(desc: &OptionDescriptor) -> bool {
    matches!(desc.kind, OptionKind::Bool | OptionKind::Int | OptionKind::Fixed) && desc.size > 4
}

pub fn is_exposed(desc: &OptionDescriptor) -> bool {
    !is_hidden(desc) && !is_array(desc)
}

fn kind_str(k: &OptionKind) -> &'static str {
    match k {
        OptionKind::Bool => "bool",
        OptionKind::Int => "int",
        OptionKind::Fixed => "fixed",
        OptionKind::String => "string",
        OptionKind::Button => "button",
        OptionKind::Group => "group",
    }
}

fn unit_str(u: &OptionUnit) -> &'static str {
    match u {
        OptionUnit::None => "none",
        OptionUnit::Pixel => "pixel",
        OptionUnit::Bit => "bit",
        OptionUnit::Mm => "mm",
        OptionUnit::Dpi => "dpi",
        OptionUnit::Percent => "percent",
        OptionUnit::Microsecond => "microsecond",
    }
}

fn capabilities(cap: i32) -> Vec<String> {
    let mut v = Vec::new();
    let mut add = |bit: i32, name: &str| {
        if cap & bit != 0 {
            v.push(name.to_owned());
        }
    };
    add(sys::SANE_CAP_SOFT_SELECT, "settable");
    add(sys::SANE_CAP_SOFT_DETECT, "readable");
    add(sys::SANE_CAP_AUTOMATIC, "automatic");
    add(sys::SANE_CAP_INACTIVE, "inactive");
    add(sys::SANE_CAP_ADVANCED, "advanced");
    add(sys::SANE_CAP_EMULATED, "emulated");
    add(sys::SANE_CAP_HARD_SELECT, "hardware");
    v
}

fn constraint(desc: &OptionDescriptor) -> (String, OwnedValue) {
    match &desc.constraint {
        OptionConstraint::None => ("none".into(), ov(false)),
        OptionConstraint::Range { min, max, quant } => ("range".into(), ov((*min, *max, *quant))),
        OptionConstraint::WordList(v) => {
            let value = if desc.kind == OptionKind::Fixed { ov(v.clone()) } else { ov(v.iter().map(|x| *x as i32).collect::<Vec<i32>>()) };
            ("list".into(), value)
        }
        OptionConstraint::StringList(v) => ("list".into(), ov(v.clone())),
    }
}

pub fn descriptor_to_dict(desc: &OptionDescriptor) -> Vardict {
    let mut d = Vardict::new();
    d.insert("name".into(), ov(desc.name.clone()));
    d.insert("title".into(), ov(desc.title.clone()));
    d.insert("description".into(), ov(desc.desc.clone()));
    d.insert("type".into(), ov(kind_str(&desc.kind)));
    d.insert("unit".into(), ov(unit_str(&desc.unit)));
    d.insert("size".into(), ov(1u32));
    let (kind, data) = constraint(desc);
    d.insert("constraint".into(), ov((kind, Value::from(data))));
    d.insert("capabilities".into(), ov(capabilities(desc.cap)));
    d
}

pub fn value_to_dbus(v: &OptionValue) -> OwnedValue {
    match v {
        OptionValue::Bool(b) => ov(*b),
        OptionValue::Int(i) => ov(*i),
        OptionValue::Fixed(f) => ov(*f),
        OptionValue::String(s) => ov(s.clone()),
        OptionValue::Button => ov(false),
    }
}

/// Convert an app-supplied variant to the type the option needs. Integers are accepted
/// for fixed options, since D-Bus clients often send `2` for `2.0`.
pub fn dbus_to_value(kind: &OptionKind, v: &Value<'_>) -> Result<OptionValue, String> {
    let bad = || format!("value {v:?} does not fit a {} option", kind_str(kind));
    match kind {
        OptionKind::Bool => bool::try_from(v).map(OptionValue::Bool).map_err(|_| bad()),
        OptionKind::Int => i32::try_from(v).map(OptionValue::Int).or_else(|_| u32::try_from(v).ok().and_then(|n| i32::try_from(n).ok()).map(OptionValue::Int).ok_or_else(bad)),
        OptionKind::Fixed => f64::try_from(v).map(OptionValue::Fixed).or_else(|_| i32::try_from(v).map(|n| OptionValue::Fixed(n as f64))).map_err(|_| bad()),
        OptionKind::String => <&str>::try_from(v).map(|s| OptionValue::String(s.to_owned())).map_err(|_| bad()),
        OptionKind::Button => Ok(OptionValue::Button),
        OptionKind::Group => Err("groups have no value".into()),
    }
}

/// What a `SetOptions` entry asks for: a value of the option's type, or, as a vardict with
/// `auto` set to true, the option's automatic value. The latter is only valid for options that
/// have the `automatic` capability.
pub fn dbus_to_setting(desc: &OptionDescriptor, v: &Value<'_>) -> Result<Setting, String> {
    if let Value::Dict(d) = v {
        let auto = d.get::<_, bool>(&"auto").ok().flatten();
        return match auto {
            Some(true) if desc.cap & sys::SANE_CAP_AUTOMATIC != 0 => Ok(Setting::Auto),
            Some(true) => Err(format!("option '{}' has no automatic value", desc.name)),
            _ => Err("a vardict value needs 'auto' set to true".into()),
        };
    }
    dbus_to_value(&desc.kind, v).map(Setting::Value)
}

/// Frame layout as the portal reports it, for the `started` event and GetFrameParameters.
pub fn parameters_to_dict(p: &nv_sane::Parameters) -> Vardict {
    let mut d = Vardict::new();
    d.insert("format".into(), ov(format_str(&p.format)));
    d.insert("depth".into(), ov(p.depth as u32));
    d.insert("pixels_per_line".into(), ov(p.pixels_per_line as u32));
    d.insert("bytes_per_line".into(), ov(p.bytes_per_line as u32));
    d.insert("lines".into(), ov(p.lines.map_or(-1i32, |l| l as i32)));
    d.insert("last_frame".into(), ov(p.last_frame));
    d
}

pub fn format_str(f: &FrameFormat) -> &'static str {
    match f {
        FrameFormat::Gray => "gray",
        FrameFormat::Rgb => "rgb",
        FrameFormat::Red => "red",
        FrameFormat::Green => "green",
        FrameFormat::Blue => "blue",
        FrameFormat::Rgbi => "rgbi",
    }
}

#[cfg(test)]
mod tests {
    use super::portal_type;

    fn caps(c: &[&str]) -> Vec<String> {
        c.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn type_is_scanner_film_or_other() {
        assert_eq!(portal_type("flatbed scanner", &caps(&["flatbed"])), "scanner");
        assert_eq!(portal_type("film scanner", &caps(&["film", "color"])), "film");
        assert_eq!(portal_type("sheetfed scanner", &caps(&["feeder"])), "scanner");
        // A scanner with a transparency unit and a document glass is not only for film
        assert_eq!(portal_type("flatbed scanner", &caps(&["film", "flatbed"])), "scanner");
        assert_eq!(portal_type("film scanner", &caps(&["film", "feeder"])), "scanner");
        assert_eq!(portal_type("virtual device", &caps(&["flatbed"])), "other");
        assert_eq!(portal_type("video camera", &caps(&[])), "other");
        assert_eq!(portal_type("webcam", &caps(&[])), "other");
    }
}
