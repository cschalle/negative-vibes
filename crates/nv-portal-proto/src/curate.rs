//! What the portal knows about scanners that SANE does not say: which are for film, which
//! drivers found the same physical scanner, and which driver to try first. This mirrors
//! `scanner-devices.h` and the curation in `scanner.c` of the C portal.

use nv_sane::DeviceInfo;
use std::collections::HashMap;

struct Known {
    usb_id: Option<&'static str>,
    model_contains: Option<&'static str>,
    name: Option<&'static str>,
    capabilities: &'static [&'static str],
}

const FILM: &[&str] = &["film", "color"];
const FLATBED_FILM: &[&str] = &["flatbed", "film", "color"];

const KNOWN: &[Known] = &[
    Known { usb_id: Some("07b3:1825"), model_contains: None, name: Some("Plustek OpticFilm 8200i SE"), capabilities: &["film", "infrared", "color"] },
    Known { usb_id: None, model_contains: Some("opticfilm"), name: None, capabilities: FILM },
    Known { usb_id: None, model_contains: Some("coolscan"), name: None, capabilities: FILM },
    Known { usb_id: None, model_contains: Some("dimage scan"), name: None, capabilities: FILM },
    Known { usb_id: None, model_contains: Some("scan elite"), name: None, capabilities: FILM },
    Known { usb_id: None, model_contains: Some("filmscan"), name: None, capabilities: FILM },
    Known { usb_id: None, model_contains: Some("primefilm"), name: None, capabilities: FILM },
    Known { usb_id: None, model_contains: Some("film scanner"), name: None, capabilities: FILM },
    Known { usb_id: None, model_contains: Some("slide scanner"), name: None, capabilities: FILM },
    Known { usb_id: None, model_contains: Some("perfection v370"), name: None, capabilities: FLATBED_FILM },
    Known { usb_id: None, model_contains: Some("perfection v500"), name: None, capabilities: FLATBED_FILM },
    Known { usb_id: None, model_contains: Some("perfection v550"), name: None, capabilities: FLATBED_FILM },
    Known { usb_id: None, model_contains: Some("perfection v600"), name: None, capabilities: FLATBED_FILM },
    Known { usb_id: None, model_contains: Some("perfection v700"), name: None, capabilities: FLATBED_FILM },
    Known { usb_id: None, model_contains: Some("perfection v750"), name: None, capabilities: FLATBED_FILM },
    Known { usb_id: None, model_contains: Some("perfection v800"), name: None, capabilities: FLATBED_FILM },
    Known { usb_id: None, model_contains: Some("perfection v850"), name: None, capabilities: FLATBED_FILM },
];

/// How a driver ranks for a scanner that more than one driver found: the lower the better.
const DEFAULT_RANK: i32 = 2;
const RANKS: &[(&str, i32)] = &[("airscan", 0), ("escl", 1), ("hpaio", 8), ("net", 9)];

fn backend_rank(backend: &str) -> i32 {
    RANKS.iter().find(|(b, _)| *b == backend).map_or(DEFAULT_RANK, |(_, r)| *r)
}

/// A scanner as applications see it: one per physical scanner, with the SANE names of every
/// driver that found it, best first. OpenScanner tries them in that order.
#[derive(Debug, Clone)]
pub struct Scanner {
    pub id: String,
    /// What `device_to_dict` shows. The first driver's entry, with the portal's name if it knows one.
    pub device: DeviceInfo,
    pub capabilities: Vec<String>,
    pub candidates: Vec<String>,
}

/// The vendor and product id of the USB device at a bus and device number
fn usb_id_for_address(bus: i64, address: i64) -> Option<String> {
    let read = |dir: &std::path::Path, file: &str| std::fs::read_to_string(dir.join(file)).ok().map(|s| s.trim().to_owned());
    for entry in std::fs::read_dir("/sys/bus/usb/devices").ok()?.flatten() {
        let base = entry.path();
        let num = |f: &str| read(&base, f).and_then(|s| s.parse::<i64>().ok());
        if num("busnum") != Some(bus) || num("devnum") != Some(address) {
            continue;
        }
        if let (Some(v), Some(p)) = (read(&base, "idVendor"), read(&base, "idProduct")) {
            return Some(format!("{v}:{p}"));
        }
    }
    None
}

/// The host of a network scanner, from the address in its name or type: `ip=192.0.2.1` (HPLIP,
/// airscan) or a URL (eSCL). SANE's net backend is not matched, since its host may have several.
fn network_host(dev: &DeviceInfo) -> Option<String> {
    let name = dev.name.as_str();
    if name.starts_with("net:") {
        return None;
    }
    let start = if let Some(i) = name.find("ip=") {
        &name[i + 3..]
    } else if let Some(t) = dev.kind.strip_prefix("ip=") {
        t
    } else if let Some(i) = name.find("://") {
        &name[i + 3..]
    } else {
        return None;
    };
    let end = start.find(['&', ':', '/', '?', ',', ' ']).unwrap_or(start.len());
    (end > 0).then(|| start[..end].to_owned())
}

/// Which physical scanner this is: the USB bus and device number, or the host. Also the USB id.
fn physical(dev: &DeviceInfo) -> (Option<String>, Option<String>) {
    if let Some(i) = dev.name.find("libusb:") {
        let mut parts = dev.name[i + 7..].split(':');
        if let (Some(Ok(bus)), Some(Ok(addr))) = (parts.next().map(str::parse::<i64>), parts.next().map(|s| s.get(..3).unwrap_or(s).parse::<i64>())) {
            return (Some(format!("usb:{bus}:{addr}")), usb_id_for_address(bus, addr));
        }
    }
    (network_host(dev).map(|h| format!("net:{h}")), None)
}

fn type_hints(kind: &str) -> Vec<String> {
    let t = kind.to_lowercase();
    let mut caps = Vec::new();
    if t.contains("film") || t.contains("slide") {
        caps.push("film".to_owned());
    }
    if t.contains("flatbed") || t.contains("multi-function") || t.contains("all-in-one") {
        caps.push("flatbed".to_owned());
    }
    if t.contains("sheet") || t.contains("adf") {
        caps.push("adf".to_owned());
    }
    caps
}

fn lookup(usb_id: Option<&str>, vendor: &str, model: &str) -> Option<&'static Known> {
    let full = format!("{vendor} {model}").to_lowercase();
    KNOWN
        .iter()
        .find(|k| matches!((k.usb_id, usb_id), (Some(a), Some(b)) if a.eq_ignore_ascii_case(b)))
        .or_else(|| KNOWN.iter().find(|k| k.model_contains.is_some_and(|m| full.contains(m))))
}

/// Opaque stable id. For a scanner with a known physical key it is the same for every driver.
fn id_for(key: &str) -> String {
    // FNV-1a, 64 bit, as `convert::scanner_id`
    crate::convert::scanner_id(key)
}

fn curate_one(dev: &DeviceInfo) -> (Option<String>, i32, Scanner) {
    let (phys, usb_id) = physical(dev);
    let known = lookup(usb_id.as_deref(), &dev.vendor, &dev.model);
    let mut caps: Vec<String> = match known {
        Some(k) => k.capabilities.iter().map(|s| (*s).to_owned()).collect(),
        None => type_hints(&dev.kind),
    };
    // What is not known to be only for film is for documents
    if !caps.iter().any(|c| c == "film" || c == "flatbed" || c == "adf") {
        caps.push("flatbed".to_owned());
    }
    let mut device = dev.clone();
    if let Some(name) = known.and_then(|k| k.name) {
        // The name shown is vendor and model: carry the clean name in the model
        device.vendor = String::new();
        device.model = name.to_owned();
    }
    let id = match &phys {
        Some(p) => id_for(p),
        None => crate::convert::scanner_id(&dev.name),
    };
    (phys, backend_rank(&dev.backend), Scanner { id, device, capabilities: caps, candidates: vec![dev.name.clone()] })
}

/// One scanner for each physical scanner. Scanners not known to be the same stay separate.
pub fn curate(devices: &[DeviceInfo]) -> Vec<Scanner> {
    let mut out: Vec<Scanner> = Vec::new();
    // Candidate ranks kept beside each scanner, to order the names
    let mut ranks: Vec<Vec<i32>> = Vec::new();
    let mut groups: HashMap<String, usize> = HashMap::new();
    for dev in devices {
        let (phys, rank, s) = curate_one(dev);
        match phys.as_ref().and_then(|p| groups.get(p).copied()) {
            None => {
                if let Some(p) = phys {
                    groups.insert(p, out.len());
                }
                ranks.push(vec![rank]);
                out.push(s);
            }
            Some(i) => {
                // After the names of equal or better rank, so that the search order breaks ties
                let at = ranks[i].iter().rposition(|r| *r <= rank).map_or(0, |p| p + 1);
                ranks[i].insert(at, rank);
                out[i].candidates.insert(at, dev.name.clone());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(name: &str, backend: &str, vendor: &str, model: &str, kind: &str) -> DeviceInfo {
        DeviceInfo { name: name.into(), backend: backend.into(), vendor: vendor.into(), model: model.into(), kind: kind.into() }
    }

    fn hp() -> Vec<DeviceInfo> {
        vec![
            dev("hpaio:/net/colorlaserjet_mfp_m282-m285?ip=192.168.68.91&queue=false", "hpaio", "Hewlett-Packard", "colorlaserjet_mfp_m282-m285", "all-in-one"),
            dev("airscan:e0:HP Color LaserJet MFP M283cdw (61F0E7)", "airscan", "eSCL", "HP Color LaserJet MFP M283cdw (61F0E7)", "ip=192.168.68.91, fd24:3d5e::1"),
        ]
    }

    #[test]
    fn one_scanner_for_two_drivers_best_first() {
        let list = curate(&hp());
        assert_eq!(list.len(), 1);
        assert!(list[0].candidates[0].starts_with("airscan:"), "{:?}", list[0].candidates);
        assert!(list[0].candidates[1].starts_with("hpaio:"));
        assert_eq!(list[0].capabilities, vec!["flatbed"]);
    }

    #[test]
    fn the_order_of_the_search_does_not_matter() {
        let mut d = hp();
        d.reverse();
        assert!(curate(&d)[0].candidates[0].starts_with("airscan:"));
    }

    #[test]
    fn different_hosts_stay_apart() {
        let mut d = hp();
        d[0].name = d[0].name.replace("192.168.68.91", "192.168.68.92");
        assert_eq!(curate(&d).len(), 2);
    }

    #[test]
    fn film_scanner_is_film_and_not_a_flatbed() {
        let s = &curate(&[dev("genesys:libusb:001:004", "genesys", "Plustek", "OpticFilm 8200i", "flatbed scanner")])[0];
        assert!(s.capabilities.iter().any(|c| c == "film"));
        assert!(!s.capabilities.iter().any(|c| c == "flatbed"));
    }

    #[test]
    fn unknown_scanner_without_hints_is_a_flatbed() {
        assert_eq!(curate(&[dev("test:0", "test", "Noname", "frontend-tester", "virtual device")])[0].capabilities, vec!["flatbed"]);
    }

    #[test]
    fn scanners_without_a_key_are_not_merged() {
        let d = [dev("net:host:a", "net", "A", "a", ""), dev("net:host:b", "net", "A", "a", "")];
        assert_eq!(curate(&d).len(), 2);
    }
}
