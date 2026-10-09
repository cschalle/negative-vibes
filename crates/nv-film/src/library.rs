//! The film profile library: the shipped profiles plus the user's own, which
//! live as one JSON file each in `$XDG_DATA_HOME/negative-vibes/film/`
//! (DESIGN §7.4). Profiles can be imported from and exported to a file, so
//! people can share them.

use std::path::{Path, PathBuf};

use crate::{FilmProfile, builtin_profiles};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("not a film profile: {0}")]
    Parse(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn user_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("negative-vibes/film")
}

/// A profile is usable when its numbers make sense and its id is a plain file
/// name, since the id names the file.
pub fn validate(p: &FilmProfile) -> Result<()> {
    let bad = |m: &str| Err(Error::Invalid(m.to_owned()));
    if p.id.is_empty() || p.id.len() > 80 || !p.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') {
        return bad("the id must be lowercase letters, digits, - and _");
    }
    if p.name.trim().is_empty() {
        return bad("the profile needs a name");
    }
    if p.schema != 1 {
        return bad("unknown profile schema");
    }
    let finite = |v: &[f32]| v.iter().all(|x| x.is_finite());
    if !finite(&p.base_rgb) || !finite(&p.gamma) || !finite(&p.density_max) || !p.crosstalk.iter().all(|r| finite(r)) || !p.curve_toe.is_finite() || !p.curve_shoulder.is_finite() {
        return bad("the profile has values that are not numbers");
    }
    if p.base_rgb.iter().any(|v| !(0.0..=1.0).contains(v) || *v <= 0.0) {
        return bad("the film base must be between 0 and 1");
    }
    if p.gamma.iter().any(|v| !(0.3..=3.0).contains(v)) {
        return bad("the gamma must be between 0.3 and 3");
    }
    if p.density_max.iter().any(|v| !(0.0..=6.0).contains(v)) {
        return bad("the maximum density must be between 0 and 6");
    }
    if !(0.0..=0.5).contains(&p.curve_toe) || !(0.0..=0.5).contains(&p.curve_shoulder) {
        return bad("the toe and shoulder must be between 0 and 0.5");
    }
    Ok(())
}

fn read_dir(dir: &Path) -> Vec<FilmProfile> {
    let mut out: Vec<FilmProfile> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_slice::<FilmProfile>(&std::fs::read(e.path()).ok()?).ok())
        .filter(|p| validate(p).is_ok())
        .collect();
    out.sort_by(|a, b| (&a.vendor, &a.name).cmp(&(&b.vendor, &b.name)));
    out
}

/// The user's profiles in `dir`, the ones that read and make sense.
pub fn user_profiles_in(dir: &Path) -> Vec<FilmProfile> {
    read_dir(dir)
}

/// Shipped profiles first (in their order), then the user's. A user profile
/// cannot take over a shipped id.
pub fn all_profiles() -> Vec<FilmProfile> {
    all_profiles_in(&user_dir())
}

pub fn all_profiles_in(dir: &Path) -> Vec<FilmProfile> {
    let mut all = builtin_profiles();
    for p in read_dir(dir) {
        if !all.iter().any(|a| a.id == p.id) {
            all.push(p);
        }
    }
    all
}

pub fn find_profile(id: &str) -> Option<FilmProfile> {
    all_profiles().into_iter().find(|p| p.id == id)
}

/// A readable id from a name: lowercase, words joined by dashes.
pub fn id_for(vendor: &str, name: &str) -> String {
    let text = format!("{vendor} {name}");
    let mut id = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            id.push(c.to_ascii_lowercase());
        } else if !id.ends_with('-') && !id.is_empty() {
            id.push('-');
        }
    }
    id.trim_end_matches('-').to_owned()
}

/// Save a user profile in `dir`. Shipped ids are refused.
pub fn save_in(dir: &Path, p: &FilmProfile) -> Result<PathBuf> {
    let mut p = p.clone();
    p.source = "user".into();
    validate(&p)?;
    if builtin_profiles().iter().any(|b| b.id == p.id) {
        return Err(Error::Invalid(format!("'{}' is one of the shipped profiles; choose another name", p.id)));
    }
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.json", p.id));
    std::fs::write(&path, serde_json::to_vec_pretty(&p)?)?;
    Ok(path)
}

pub fn save(p: &FilmProfile) -> Result<PathBuf> {
    save_in(&user_dir(), p)
}

/// Read a profile file and add it to the user's library.
pub fn import_in(dir: &Path, file: &Path) -> Result<FilmProfile> {
    let p: FilmProfile = serde_json::from_slice(&std::fs::read(file)?)?;
    save_in(dir, &p)?;
    Ok(FilmProfile { source: "user".into(), ..p })
}

pub fn import(file: &Path) -> Result<FilmProfile> {
    import_in(&user_dir(), file)
}

/// Write any profile, shipped or the user's, to a file.
pub fn export(id: &str, file: &Path) -> Result<()> {
    let p = find_profile(id).ok_or_else(|| Error::Invalid(format!("no profile '{id}'")))?;
    std::fs::write(file, serde_json::to_vec_pretty(&p)?)?;
    Ok(())
}

/// Remove one of the user's profiles. Shipped ones cannot be deleted.
pub fn delete_in(dir: &Path, id: &str) -> Result<()> {
    if builtin_profiles().iter().any(|b| b.id == id) {
        return Err(Error::Invalid("shipped profiles cannot be deleted".into()));
    }
    if id.contains(['/', '\\', '.']) {
        return Err(Error::Invalid("not a profile id".into()));
    }
    std::fs::remove_file(dir.join(format!("{id}.json")))?;
    Ok(())
}

pub fn delete(id: &str) -> Result<()> {
    delete_in(&user_dir(), id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nv-film-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn mine() -> FilmProfile {
        FilmProfile { id: "my-film-400".into(), vendor: "Me".into(), name: "Film 400".into(), iso: 400, ..FilmProfile::identity() }
    }

    #[test]
    fn a_saved_profile_is_listed_after_the_shipped_ones_and_can_be_deleted() {
        let d = dir("lib");
        save_in(&d, &mine()).unwrap();
        let all = all_profiles_in(&d);
        assert_eq!(all.last().unwrap().id, "my-film-400");
        assert_eq!(all.last().unwrap().source, "user");
        assert_eq!(all.len(), builtin_profiles().len() + 1);
        delete_in(&d, "my-film-400").unwrap();
        assert_eq!(all_profiles_in(&d).len(), builtin_profiles().len());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn shipped_ids_cannot_be_taken_or_deleted_and_bad_profiles_are_refused() {
        let d = dir("refuse");
        let shipped = FilmProfile { id: "kodak-portra-400".into(), ..mine() };
        assert!(save_in(&d, &shipped).is_err());
        assert!(delete_in(&d, "kodak-portra-400").is_err());
        assert!(delete_in(&d, "../x").is_err());
        for broken in [
            FilmProfile { id: "Has Space".into(), ..mine() },
            FilmProfile { base_rgb: [0.0, 0.5, 0.5], ..mine() },
            FilmProfile { gamma: [f32::NAN, 1.0, 1.0], ..mine() },
            FilmProfile { curve_toe: 0.9, ..mine() },
            FilmProfile { name: " ".into(), ..mine() },
        ] {
            assert!(validate(&broken).is_err(), "{:?}", broken.id);
            assert!(save_in(&d, &broken).is_err());
        }
        assert!(!d.exists() || read_dir(&d).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn export_then_import_round_trips_and_garbage_is_refused() {
        let d = dir("share");
        let file = d.join("p.json");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(&file, serde_json::to_vec(&mine()).unwrap()).unwrap();
        let lib = d.join("lib");
        let got = import_in(&lib, &file).unwrap();
        assert_eq!(got.id, "my-film-400");
        assert_eq!(read_dir(&lib), vec![got]);
        let junk = d.join("junk.json");
        std::fs::write(&junk, "{\"hello\": 1}").unwrap();
        assert!(matches!(import_in(&lib, &junk), Err(Error::Parse(_))));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn ids_are_made_from_names() {
        assert_eq!(id_for("Kodak", "Ektar 100"), "kodak-ektar-100");
        assert_eq!(id_for("  Ilford ", "HP5+ (pushed)"), "ilford-hp5-pushed");
        assert!(validate(&FilmProfile { id: id_for("Fuji", "Pro 400H"), ..mine() }).is_ok());
    }

    #[test]
    fn old_profile_files_without_measured_on_load() {
        let mut v = serde_json::to_value(mine()).unwrap();
        v.as_object_mut().unwrap().remove("measured_on");
        assert!(serde_json::from_value::<FilmProfile>(v).is_ok());
    }
}
