//! Spellcheck + autocorrect engine.
//!
//! Layout:
//! - `tokenize`  — pure tokenizer that classifies words / mentions / URLs /
//!   emote codes / all-caps shorthand. No external deps. Unit-testable.
//! - `personal`  — load/save the user's personal dictionary at
//!   `~/.config/livestreamlist/personal_dict.json`.
//! - `dict`      — enumerate installed hunspell dicts; bundled en_US fallback.

pub mod tokenize;
pub mod personal;
pub mod dict;

use crate::spellcheck::dict::DictInfo;
use crate::spellcheck::personal::PersonalDict;
use crate::spellcheck::tokenize::{tokenize, TokenClass};
use anyhow::Result;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// One misspelled word with its byte range in the input text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MisspelledRange {
    pub start: usize,
    pub end: usize,
    pub word: String,
}

/// App-wide spellcheck state. Held in `Arc<SpellChecker>` via
/// `tauri::Manager::manage`. Per-language `Hunspell` instances are
/// loaded lazily on first use.
pub struct SpellChecker {
    /// language code (`"en_US"`) → loaded Hunspell instance.
    /// Lazy: populated on first `check`/`suggest` for that language.
    by_lang: Mutex<HashMap<String, Arc<Mutex<hunspell_rs::Hunspell>>>>,
    /// All dicts discovered + bundled, for `list_dicts` and lazy load.
    available: Vec<DictInfo>,
    personal: RwLock<PersonalDict>,
}

impl SpellChecker {
    /// Construct from app-state context. `personal_dict_path` is
    /// usually `~/.config/livestreamlist/personal_dict.json` (computed
    /// by the caller using `config::data_dir()`).
    pub fn new(personal_dict_path: PathBuf) -> Self {
        let available = dict::list_dicts();
        let personal = PersonalDict::load(personal_dict_path);
        Self {
            by_lang: Mutex::new(HashMap::new()),
            available,
            personal: RwLock::new(personal),
        }
    }

    pub fn list_dicts(&self) -> Vec<DictInfo> {
        self.available.clone()
    }

    /// Load (or return cached) Hunspell instance for `code`. Returns
    /// `None` if no dict matches.
    fn dict_for(&self, code: &str) -> Option<Arc<Mutex<hunspell_rs::Hunspell>>> {
        let mut map = self.by_lang.lock();
        if let Some(d) = map.get(code) {
            return Some(d.clone());
        }
        let info = self.available.iter().find(|d| d.code == code)?;
        let aff = info.aff_path.to_string_lossy().to_string();
        let dic = info.aff_path.with_extension("dic").to_string_lossy().to_string();
        let h = hunspell_rs::Hunspell::new(&aff, &dic);
        let arc = Arc::new(Mutex::new(h));
        map.insert(code.to_string(), arc.clone());
        Some(arc)
    }

    /// Run spellcheck on `text` for `language`. Tokenizes, filters out
    /// non-Word tokens (mentions, URLs, emotes, all-caps shorthand),
    /// looks up each Word in hunspell, and skips anything in the
    /// personal dict. Returns ranges suitable for the React overlay.
    pub fn check(
        &self,
        text: &str,
        language: &str,
        channel_emotes: &[String],
    ) -> Vec<MisspelledRange> {
        let dict = match self.dict_for(language) {
            Some(d) => d,
            None => return Vec::new(),
        };
        let dict = dict.lock();
        let personal = self.personal.read();
        let mut out = Vec::new();
        for tok in tokenize(text, channel_emotes) {
            if tok.class != TokenClass::Word { continue; }
            if personal.contains(&tok.text) { continue; }
            if dict.check(&tok.text) != hunspell_rs::CheckResult::FoundInDictionary {
                out.push(MisspelledRange {
                    start: tok.start,
                    end: tok.end,
                    word: tok.text,
                });
            }
        }
        out
    }

    pub fn suggest(&self, word: &str, language: &str) -> Vec<String> {
        let Some(dict) = self.dict_for(language) else { return Vec::new(); };
        let dict = dict.lock();
        let mut s = dict.suggest(word);
        s.truncate(5);
        s
    }

    pub fn add_to_personal(&self, word: &str) -> Result<bool> {
        let mut p = self.personal.write();
        p.add(word)
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use tempfile::TempDir;

    fn checker_with_bundled() -> SpellChecker {
        let td = TempDir::new().unwrap();
        let dict_path = td.path().join("personal.json");
        // Leak td so the dir survives the test (avoids race with the
        // checker holding the path).
        std::mem::forget(td);
        SpellChecker::new(dict_path)
    }

    #[test]
    fn check_detects_obvious_misspelling() {
        // Requires Task 5's bundled en_US to be present.
        let c = checker_with_bundled();
        let result = c.check("hello wnoderful world", "en_US", &[]);
        let words: Vec<&str> = result.iter().map(|r| r.word.as_str()).collect();
        assert!(words.contains(&"wnoderful"), "expected wnoderful flagged, got {:?}", words);
        // Correct words must NOT be flagged.
        assert!(!words.contains(&"hello"));
        assert!(!words.contains(&"world"));
    }

    #[test]
    fn check_skips_mentions_urls_emotes() {
        let c = checker_with_bundled();
        let r = c.check("hi @shroud check twitch.tv/shroud Kappa LMAO",
                        "en_US",
                        &["Kappa".to_string()]);
        // None of the skip-tokens should appear as misspellings.
        for word in r.iter().map(|m| &m.word) {
            assert_ne!(word, "shroud");
            assert_ne!(word, "twitch.tv/shroud");
            assert_ne!(word, "Kappa");
            assert_ne!(word, "LMAO");
        }
    }

    #[test]
    fn personal_dict_suppresses_flag() {
        let c = checker_with_bundled();
        // First confirm "wnoderful" IS flagged…
        assert!(!c.check("wnoderful test", "en_US", &[]).is_empty());
        // …then add it and confirm it's NOT.
        c.add_to_personal("wnoderful").unwrap();
        let r = c.check("wnoderful test", "en_US", &[]);
        assert!(r.is_empty(), "personal-dict word should not be flagged: {:?}", r);
    }

    #[test]
    fn suggest_returns_corrections() {
        let c = checker_with_bundled();
        let s = c.suggest("teh", "en_US");
        assert!(!s.is_empty(), "expected at least one suggestion for 'teh'");
        assert!(s.iter().any(|s| s == "the"), "'the' should be in suggestions: {:?}", s);
    }

    #[test]
    fn unknown_language_returns_empty() {
        let c = checker_with_bundled();
        let r = c.check("anything", "xx_YY", &[]);
        assert!(r.is_empty());
        let s = c.suggest("teh", "xx_YY");
        assert!(s.is_empty());
    }
}
