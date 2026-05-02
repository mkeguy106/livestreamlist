//! Enumerate hunspell dictionaries available on the host.
//!
//! Linux: scans `/usr/share/hunspell`, `/usr/share/myspell/dicts`, and
//! the Flatpak paths (`/run/host/usr/share/hunspell`, `/app/share/hunspell`).
//! Pairs `.aff` and `.dic` files by basename.
//!
//! macOS / Windows: returns the bundled `en_US` only (no system enchant
//! integration in this PR).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DictInfo {
    /// Locale code, e.g. `"en_US"`, `"de_DE"`.
    pub code: String,
    /// Human-readable name for the dropdown, e.g. `"English (US)"`.
    pub name: String,
    /// Absolute path to the `.aff` file. The matching `.dic` is at the
    /// same path with `.dic` extension. Internal — not part of the IPC
    /// contract; frontend only needs `code` and `name`.
    #[serde(skip, default = "default_path")]
    pub aff_path: PathBuf,
}

fn default_path() -> PathBuf {
    PathBuf::new()
}

/// Discover dictionaries on the system, plus the bundled fallback.
/// Returns a deduplicated list keyed by `code` — system entries take
/// precedence over the bundle.
pub fn list_dicts() -> Vec<DictInfo> {
    let mut found: Vec<DictInfo> = Vec::new();
    for dir in search_paths() {
        scan_dir(&dir, &mut found);
    }
    if let Some(bundled) = bundled_en_us_path() {
        if !found.iter().any(|d| d.code == "en_US") {
            found.push(DictInfo {
                code: "en_US".to_string(),
                name: "English (US) — bundled".to_string(),
                aff_path: bundled,
            });
        }
    }
    found.sort_by(|a, b| a.code.cmp(&b.code));
    found
}

/// Where to look on the current OS. Public for testing.
pub fn search_paths() -> Vec<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        vec![
            PathBuf::from("/usr/share/hunspell"),
            PathBuf::from("/usr/share/myspell/dicts"),
            PathBuf::from("/run/host/usr/share/hunspell"),
            PathBuf::from("/app/share/hunspell"),
        ]
    }
    #[cfg(not(target_os = "linux"))]
    {
        vec![]
    }
}

/// Path to the bundled fallback `en_US.aff`. `None` if not present.
pub fn bundled_en_us_path() -> Option<PathBuf> {
    // Caller (SpellChecker::new in lib.rs) must set LIVESTREAMLIST_RESOURCE_DIR
    // BEFORE calling SpellChecker::new in production builds; otherwise the
    // production resource path branch silently falls through to the dev path.
    // Production: Tauri exposes resources via the resource_dir at runtime.
    // We can't reach AppHandle from this pure function, so we consult an
    // env var that's set by SpellChecker::new (which DOES have AppHandle).
    if let Ok(resolved) = std::env::var("LIVESTREAMLIST_RESOURCE_DIR") {
        let p = PathBuf::from(resolved).join("dictionaries/en_US.aff");
        if p.exists() {
            return Some(p);
        }
    }
    // Dev: walk up from CARGO_MANIFEST_DIR.
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("dictionaries/en_US.aff");
    if dev.exists() { Some(dev) } else { None }
}

/// Scan a directory for `.aff` files, pair them with their matching
/// `.dic`, and append `DictInfo` entries to `out`. Public for testing
/// (so unit tests can hand a tempdir).
pub fn scan_dir(dir: &Path, out: &mut Vec<DictInfo>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) != Some("aff") {
            continue;
        }
        let stem = match p.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let dic = p.with_extension("dic");
        if !dic.exists() {
            continue;
        }
        if out.iter().any(|d| d.code == stem) {
            continue; // earlier path took precedence
        }
        out.push(DictInfo {
            code: stem.clone(),
            name: pretty_name(&stem),
            aff_path: p,
        });
    }
}

/// Map `en_US` → `English (US)`, `de_DE` → `German (Germany)`, etc.
/// Falls back to the raw code when unknown.
fn pretty_name(code: &str) -> String {
    let (lang, region) = match code.split_once('_') {
        Some((l, r)) => (l, Some(r)),
        None => (code, None),
    };
    let lang_name = match lang {
        "en" => "English",
        "es" => "Spanish",
        "de" => "German",
        "fr" => "French",
        "it" => "Italian",
        "pt" => "Portuguese",
        "nl" => "Dutch",
        "pl" => "Polish",
        "ru" => "Russian",
        "sv" => "Swedish",
        "no" => "Norwegian",
        "da" => "Danish",
        "fi" => "Finnish",
        "cs" => "Czech",
        "hu" => "Hungarian",
        "tr" => "Turkish",
        "ja" => "Japanese",
        "ko" => "Korean",
        "zh" => "Chinese",
        _ => return code.to_string(),
    };
    let region_name = region.and_then(|r| match r {
        "US" => Some("US"),
        "GB" | "UK" => Some("UK"),
        "CA" => Some("Canada"),
        "AU" => Some("Australia"),
        "DE" => Some("Germany"),
        "AT" => Some("Austria"),
        "CH" => Some("Switzerland"),
        "ES" => Some("Spain"),
        "MX" => Some("Mexico"),
        "FR" => Some("France"),
        "BR" => Some("Brazil"),
        "PT" => Some("Portugal"),
        _ => None,
    });
    match region_name {
        Some(r) => format!("{} ({})", lang_name, r),
        None => lang_name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn touch(p: &Path) {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, "").unwrap();
    }

    #[test]
    fn scan_pairs_aff_and_dic() {
        let td = TempDir::new().unwrap();
        touch(&td.path().join("en_US.aff"));
        touch(&td.path().join("en_US.dic"));
        touch(&td.path().join("de_DE.aff"));
        touch(&td.path().join("de_DE.dic"));
        let mut out = Vec::new();
        scan_dir(td.path(), &mut out);
        out.sort_by(|a, b| a.code.cmp(&b.code));
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].code, "de_DE");
        assert_eq!(out[1].code, "en_US");
    }

    #[test]
    fn scan_skips_aff_without_matching_dic() {
        let td = TempDir::new().unwrap();
        touch(&td.path().join("en_US.aff")); // no .dic
        touch(&td.path().join("de_DE.aff"));
        touch(&td.path().join("de_DE.dic"));
        let mut out = Vec::new();
        scan_dir(td.path(), &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, "de_DE");
    }

    #[test]
    fn scan_dedupes_within_one_call() {
        let td = TempDir::new().unwrap();
        touch(&td.path().join("en_US.aff"));
        touch(&td.path().join("en_US.dic"));
        let mut out = Vec::new();
        scan_dir(td.path(), &mut out);
        // A second scan over the same dir must NOT add a duplicate.
        scan_dir(td.path(), &mut out);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn scan_missing_dir_is_noop() {
        let mut out = Vec::new();
        scan_dir(Path::new("/nonexistent/path/that/does/not/exist"), &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn pretty_name_known_locales() {
        assert_eq!(pretty_name("en_US"), "English (US)");
        assert_eq!(pretty_name("de_DE"), "German (Germany)");
        assert_eq!(pretty_name("fr"), "French");
    }

    #[test]
    fn pretty_name_unknown_falls_back() {
        assert_eq!(pretty_name("xx_YY"), "xx_YY");
    }

    #[test]
    fn bundled_en_us_path_resolves_in_dev() {
        // After Task 5, the bundled dict files exist under
        // CARGO_MANIFEST_DIR/dictionaries/. This test verifies dev-mode
        // fallback. (Production resource resolution is tested by
        // SpellChecker::new in Task 6.)
        let path = bundled_en_us_path()
            .expect("bundled en_US.aff should be present after Task 5");
        assert!(path.ends_with("en_US.aff"));
        assert!(path.exists());
        let dic = path.with_extension("dic");
        assert!(dic.exists(), "matching .dic should also exist");
    }
}
