//! Personal dictionary — words the user has explicitly marked as "not a
//! misspelling" via the right-click menu's "Add to dictionary" item.
//!
//! Persisted at `~/.config/livestreamlist/personal_dict.json` via the
//! existing `atomic_write` helper from `config::atomic_write`. Lowercase
//! normalized; case-insensitive lookup. Not language-scoped.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersonalDictFile {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    words: Vec<String>,
}

fn default_version() -> u32 { SCHEMA_VERSION }

#[derive(Debug, Clone, Default)]
pub struct PersonalDict {
    /// Lowercase-normalized.
    set: HashSet<String>,
    path: PathBuf,
}

impl PersonalDict {
    /// Load from `path`. Missing or malformed file yields an empty dict
    /// (preserving the path for the next `save`).
    pub fn load(path: PathBuf) -> Self {
        let set = match std::fs::read_to_string(&path) {
            Ok(s) => match serde_json::from_str::<PersonalDictFile>(&s) {
                Ok(file) => file.words.into_iter().map(|w| w.to_lowercase()).collect(),
                Err(_) => HashSet::new(),
            },
            Err(_) => HashSet::new(),
        };
        Self { set, path }
    }

    pub fn contains(&self, word: &str) -> bool {
        self.set.contains(&word.to_lowercase())
    }

    /// Add a word. Returns `Ok(true)` if newly inserted, `Ok(false)` if
    /// already present. On `true` the file is rewritten; on `false` no
    /// disk I/O occurs.
    pub fn add(&mut self, word: &str) -> Result<bool> {
        let normalized = word.to_lowercase();
        // Insert first; if save() fails the in-memory set is ahead by one
        // entry, which is acceptable — the next successful save will catch up.
        if !self.set.insert(normalized) {
            return Ok(false);
        }
        self.save()?;
        Ok(true)
    }

    pub fn len(&self) -> usize { self.set.len() }
    pub fn is_empty(&self) -> bool { self.set.is_empty() }

    fn save(&self) -> Result<()> {
        let mut words: Vec<String> = self.set.iter().cloned().collect();
        words.sort();
        let file = PersonalDictFile {
            version: SCHEMA_VERSION,
            words,
        };
        let json = serde_json::to_string_pretty(&file)
            .context("serializing personal dict")?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating dir {:?}", parent))?;
        }
        crate::config::atomic_write(&self.path, json.as_bytes())
            .with_context(|| format!("writing personal dict to {:?}", self.path))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp(td: &TempDir) -> PathBuf {
        td.path().join("personal_dict.json")
    }

    #[test]
    fn empty_when_missing() {
        let td = TempDir::new().unwrap();
        let d = PersonalDict::load(tmp(&td));
        assert!(d.is_empty());
        assert!(!d.contains("anything"));
    }

    #[test]
    fn malformed_json_yields_empty() {
        let td = TempDir::new().unwrap();
        let p = tmp(&td);
        std::fs::write(&p, "not json {{").unwrap();
        let d = PersonalDict::load(p);
        assert!(d.is_empty());
    }

    #[test]
    fn add_persists_and_round_trips() {
        let td = TempDir::new().unwrap();
        let p = tmp(&td);
        let mut d = PersonalDict::load(p.clone());
        assert_eq!(d.add("Kappa").unwrap(), true);
        assert_eq!(d.add("Kappa").unwrap(), false); // duplicate
        assert_eq!(d.len(), 1);

        let d2 = PersonalDict::load(p);
        assert!(d2.contains("Kappa"));
    }

    #[test]
    fn lookup_is_case_insensitive() {
        let td = TempDir::new().unwrap();
        let mut d = PersonalDict::load(tmp(&td));
        d.add("StreamerName").unwrap();
        assert!(d.contains("streamername"));
        assert!(d.contains("STREAMERNAME"));
        assert!(d.contains("StreamerName"));
    }

    #[test]
    fn load_existing_file() {
        let td = TempDir::new().unwrap();
        let p = tmp(&td);
        std::fs::write(
            &p,
            r#"{"version":1,"words":["alpha","BETA","Gamma"]}"#,
        ).unwrap();
        let d = PersonalDict::load(p);
        assert_eq!(d.len(), 3);
        assert!(d.contains("alpha"));
        assert!(d.contains("beta"));
        assert!(d.contains("gamma"));
    }

    #[test]
    fn duplicate_add_is_noop_no_io() {
        // Calling add() with an existing word must NOT touch the file
        // (we'd see this in the mtime). Important so that
        // spellcheck_check doesn't re-write the file on every flagged
        // word that the user has already added.
        let td = TempDir::new().unwrap();
        let p = tmp(&td);
        let mut d = PersonalDict::load(p.clone());
        d.add("hello").unwrap();
        let mtime1 = std::fs::metadata(&p).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        d.add("hello").unwrap();
        let mtime2 = std::fs::metadata(&p).unwrap().modified().unwrap();
        assert_eq!(mtime1, mtime2, "second add should not have rewritten the file");
    }
}
