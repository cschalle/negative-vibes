//! What the portal knows about scanners that SANE does not say: which are for film, which
//! drivers found the same physical scanner, and which driver to try first. This mirrors
//! `scanner-devices.h` and the curation in `scanner.c` of the C portal.

use nv_sane::DeviceInfo;
use std::collections::HashMap;

/// A scanner or a product line the portal knows about
struct Known {
    usb_id: Option<String>,
    model_contains: Option<String>,
    name: Option<String>,
    /// The backend to open the scanner with before any other
    driver: Option<String>,
    capabilities: Vec<String>,
}

/// What the portal knows, from `scanner-devices.json`: the C portal reads the same file from the
/// data directories. When there is none, the copy that came with this crate is used.
struct Table {
    known: Vec<Known>,
    ranks: HashMap<String, i64>,
    default_rank: i64,
}

const EMBEDDED: &str = include_str!("../data/scanner-devices.json");
const DEFAULT_RANK: i64 = 2;
const PREFERRED_RANK: i32 = -1;

impl Table {
    fn parse(text: &str) -> Option<Table> {
        let root: serde_json::Value = serde_json::from_str(text).ok()?;
        let str_of = |e: &serde_json::Value, k: &str| e.get(k).and_then(|v| v.as_str()).map(str::to_lowercase);
        let known = root
            .get("scanners")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|e| {
                let (usb_id, model_contains) = (str_of(e, "usb_id"), str_of(e, "model_contains"));
                // An entry with nothing to match on is ignored, not taken for every scanner
                if usb_id.is_none() && model_contains.is_none() {
                    return None;
                }
                let capabilities = e.get("capabilities")?.as_array()?.iter().filter_map(|c| c.as_str().map(str::to_owned)).collect();
                Some(Known {
                    usb_id,
                    model_contains,
                    name: e.get("name").and_then(|v| v.as_str()).map(str::to_owned),
                    driver: e.get("driver").and_then(|v| v.as_str()).map(str::to_owned),
                    capabilities,
                })
            })
            .collect();
        let ranks = root.get("driver_ranks").and_then(|v| v.as_object()).into_iter().flatten().filter_map(|(k, v)| Some((k.clone(), v.as_i64()?))).collect();
        let default_rank = root.get("default_rank").and_then(|v| v.as_i64()).unwrap_or(DEFAULT_RANK);
        Some(Table { known, ranks, default_rank })
    }

    fn load() -> Table {
        std::env::var("XDG_DATA_DIRS")
            .unwrap_or_else(|_| "/usr/local/share:/usr/share".into())
            .split(':')
            .filter(|d| !d.is_empty())
            .find_map(|d| std::fs::read_to_string(format!("{d}/xdg-desktop-portal/scanner-devices.json")).ok().and_then(|t| Table::parse(&t)))
            .or_else(|| Table::parse(EMBEDDED))
            .expect("the scanner-devices.json that came with the crate parses")
    }

    fn rank(&self, backend: &str) -> i64 {
        self.ranks.get(backend).copied().unwrap_or(self.default_rank)
    }

    fn lookup(&self, usb_id: Option<&str>, vendor: &str, model: &str) -> Option<&Known> {
        let full = format!("{vendor} {model}").to_lowercase();
        self.known
            .iter()
            .find(|k| matches!((&k.usb_id, usb_id), (Some(a), Some(b)) if a.eq_ignore_ascii_case(b)))
            .or_else(|| self.known.iter().find(|k| k.model_contains.as_ref().is_some_and(|m| full.contains(m.as_str()))))
    }
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
        caps.push("feeder".to_owned());
    }
    caps
}

/// Opaque stable id. For a scanner with a known physical key it is the same for every driver.
fn id_for(key: &str) -> String {
    // FNV-1a, 64 bit, as `convert::scanner_id`
    crate::convert::scanner_id(key)
}

fn curate_one(table: &Table, dev: &DeviceInfo) -> (Option<String>, i32, Scanner) {
    let (phys, usb_id) = physical(dev);
    let known = table.lookup(usb_id.as_deref(), &dev.vendor, &dev.model);
    let mut caps: Vec<String> = match known {
        Some(k) => k.capabilities.clone(),
        None => type_hints(&dev.kind),
    };
    // What is not known to be only for film is for documents
    if !caps.iter().any(|c| c == "film" || c == "flatbed" || c == "feeder") {
        caps.push("flatbed".to_owned());
    }
    let mut device = dev.clone();
    if let Some(name) = known.and_then(|k| k.name.as_deref()) {
        // The name shown is vendor and model: carry the clean name in the model
        device.vendor = String::new();
        device.model = name.to_owned();
    }
    let id = match &phys {
        Some(p) => id_for(p),
        None => crate::convert::scanner_id(&dev.name),
    };
    // A scanner whose entry names a driver is opened with it first when it is among the ones that
    // found it, ahead of every rank
    let rank = match known.and_then(|k| k.driver.as_deref()) {
        Some(driver) if driver == dev.backend => PREFERRED_RANK,
        _ => table.rank(&dev.backend) as i32,
    };
    (phys, rank, Scanner { id, device, capabilities: caps, candidates: vec![dev.name.clone()] })
}

/// One scanner for each physical scanner. Scanners not known to be the same stay separate.
pub fn curate(devices: &[DeviceInfo]) -> Vec<Scanner> {
    let table = Table::load();
    let mut out: Vec<Scanner> = Vec::new();
    // Candidate ranks kept beside each scanner, to order the names
    let mut ranks: Vec<Vec<i32>> = Vec::new();
    let mut groups: HashMap<String, usize> = HashMap::new();
    for dev in devices {
        let (phys, rank, s) = curate_one(&table, dev);
        match phys.as_ref().and_then(|p| groups.get(p).copied()) {
            None => {
                if let Some(p) = phys {
                    groups.insert(p, out.len());
                }
                ranks.push(vec![rank]);
                out.push(s);
            }
            Some(i) => {
                // By rank, and for equal ranks by the driver's name, never by which one answered first
                let at = (0..ranks[i].len()).find(|&j| (ranks[i][j], out[i].candidates[j].as_str()) > (rank, dev.name.as_str())).unwrap_or(ranks[i].len());
                ranks[i].insert(at, rank);
                out[i].candidates.insert(at, dev.name.clone());
                // The scanner is described by the driver that will open it
                if at == 0 {
                    out[i].device.backend = dev.backend.clone();
                }
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
    fn equal_ranks_are_ordered_by_name_whichever_answered_first() {
        let a = dev("fakey:tie", "fakey", "Fake", "Tie Pair", "all-in-one");
        let b = dev("fakez:tie", "fakez", "Fake", "Tie Pair", "all-in-one");
        // Same host, so the same scanner
        let (a, b) = (DeviceInfo { kind: "ip=192.0.2.30".into(), ..a }, DeviceInfo { kind: "ip=192.0.2.30".into(), ..b });
        for order in [[a.clone(), b.clone()], [b, a]] {
            let list = curate(&order);
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].candidates, vec!["fakey:tie", "fakez:tie"]);
            assert_eq!(list[0].device.backend, "fakey");
        }
    }

    #[test]
    fn the_film_scanner_driver_ranks_ahead_of_genesys() {
        // genesys finds the 8200i SE too. Without the scanner on this machine there is no USB id to
        // match the table's entry that names the driver, so this is the rank that decides.
        let list = curate(&[
            dev("genesys:libusb:003:002", "genesys", "Plustek", "OpticFilm 8200i", "flatbed scanner"),
            dev("plustek8200i:libusb:003:002", "plustek8200i", "Plustek", "OpticFilm 8200i SE", "film scanner"),
        ]);
        assert_eq!(list.len(), 1);
        assert!(list[0].candidates[0].starts_with("plustek8200i:"), "{:?}", list[0].candidates);
        assert_eq!(list[0].device.backend, "plustek8200i");
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
