//! File names from a template (F§10: auto-numbered names, a chosen pattern).
//!
//! Tokens: `{roll}`, `{frame}` or `{frame:03}` (zero-padded to the width),
//! `{label}` (the frame's label, or its number if it has none), `{film}`,
//! `{date}` (`YYYY-MM-DD`). Everything else is copied as typed. Names that
//! already exist get " (2)", " (3)" … so nothing is overwritten.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct NameContext {
    pub roll: String,
    pub frame: u32,
    pub label: Option<String>,
    pub film: Option<String>,
    /// `YYYY-MM-DD`.
    pub date: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    #[error("unknown token {{{0}}}")]
    UnknownToken(String),
    #[error("a {{ is not closed")]
    Unclosed,
    #[error("the template makes an empty file name")]
    Empty,
}

/// Longest stem produced, in bytes: leaves room for " (99)" and an extension.
const MAX_STEM: usize = 120;

/// Fill in a template. The result is safe as a file name: path separators and
/// control characters become `_`, and it is neither empty nor hidden.
pub fn render_template(template: &str, ctx: &NameContext) -> Result<String, NameError> {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let close = rest[open..].find('}').ok_or(NameError::Unclosed)? + open;
        let token = &rest[open + 1..close];
        let (name, spec) = token.split_once(':').map_or((token, None), |(n, s)| (n, Some(s)));
        match name {
            "roll" => out.push_str(&ctx.roll),
            "frame" => {
                let width = spec.and_then(|s| s.parse::<usize>().ok()).unwrap_or(0).min(8);
                out.push_str(&format!("{:0width$}", ctx.frame));
            }
            "label" => match &ctx.label {
                Some(l) if !l.is_empty() => out.push_str(l),
                _ => out.push_str(&ctx.frame.to_string()),
            },
            "film" => out.push_str(ctx.film.as_deref().unwrap_or("")),
            "date" => out.push_str(&ctx.date),
            other => return Err(NameError::UnknownToken(other.to_owned())),
        }
        rest = &rest[close + 1..];
    }
    out.push_str(rest);

    let mut name: String = out.chars().map(|c| if c == '/' || c == '\\' || c == '\0' || c.is_control() { '_' } else { c }).collect();
    name = name.trim().trim_start_matches('.').to_owned();
    if name.len() > MAX_STEM {
        let mut cut = MAX_STEM;
        while !name.is_char_boundary(cut) {
            cut -= 1;
        }
        name.truncate(cut);
    }
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    Ok(name)
}

/// `dir/stem.ext`, or the first of `stem (2).ext`, `stem (3).ext` … that
/// does not exist yet.
pub fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}).{ext}")))
        .find(|p| !p.exists())
        .expect("an unused number exists")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> NameContext {
        NameContext { roll: "Portra-1".into(), frame: 7, label: Some("tulips".into()), film: Some("Portra 400".into()), date: "2026-10-08".into() }
    }

    #[test]
    fn tokens_are_filled_and_padded() {
        assert_eq!(render_template("{roll}-{frame:03}", &ctx()).unwrap(), "Portra-1-007");
        assert_eq!(render_template("{date} {label} ({film})", &ctx()).unwrap(), "2026-10-08 tulips (Portra 400)");
        assert_eq!(render_template("{frame}", &ctx()).unwrap(), "7");
        assert_eq!(render_template("plain", &ctx()).unwrap(), "plain");
    }

    #[test]
    fn a_frame_without_a_label_uses_its_number() {
        let c = NameContext { label: None, ..ctx() };
        assert_eq!(render_template("{label}", &c).unwrap(), "7");
    }

    #[test]
    fn bad_templates_say_what_is_wrong() {
        assert_eq!(render_template("{nope}", &ctx()), Err(NameError::UnknownToken("nope".into())));
        assert_eq!(render_template("{roll", &ctx()), Err(NameError::Unclosed));
        assert_eq!(render_template("{label}", &NameContext { label: None, frame: 0, ..ctx() }).unwrap(), "0");
        assert_eq!(render_template("  ", &ctx()), Err(NameError::Empty));
    }

    #[test]
    fn the_result_cannot_escape_the_folder_or_hide() {
        let c = NameContext { roll: "../../etc/passwd".into(), ..ctx() };
        let n = render_template("{roll}", &c).unwrap();
        assert!(!n.contains('/') && !n.starts_with('.'), "{n}");
        assert!(!render_template(".hidden", &ctx()).unwrap().starts_with('.'));
        let long = render_template(&"x".repeat(500), &ctx()).unwrap();
        assert_eq!(long.len(), MAX_STEM);
        // Never cuts a multi-byte character in half.
        assert!(render_template(&"é".repeat(200), &ctx()).unwrap().len() <= MAX_STEM);
    }

    #[test]
    fn existing_names_get_a_number() {
        let dir = std::env::temp_dir().join(format!("nv-naming-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_path(&dir, "scan", "tif"), dir.join("scan.tif"));
        std::fs::write(dir.join("scan.tif"), b"").unwrap();
        assert_eq!(unique_path(&dir, "scan", "tif"), dir.join("scan (2).tif"));
        std::fs::write(dir.join("scan (2).tif"), b"").unwrap();
        assert_eq!(unique_path(&dir, "scan", "tif"), dir.join("scan (3).tif"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
