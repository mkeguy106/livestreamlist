//! pronouns.alejo.io fetcher with in-memory LRU.
//!
//! alejo's API:
//!   GET https://api.pronouns.alejo.io/v1/users/{login}
//!   200 → JSON object: { "channel_id": "...", "channel_login": "...",
//!                        "pronoun_id": "hehim", ... }
//!   404 → user has not set pronouns; we cache that as None
//!
//! Display strings are mapped from pronoun_id (we keep a small lookup table;
//! unknown ids fall back to the raw id).

use std::sync::Arc;
use std::time::{Duration, Instant};

use lru::LruCache;
use parking_lot::Mutex;
use serde::Deserialize;

const TTL: Duration = Duration::from_secs(60 * 60);
const CAPACITY: usize = 200;

#[derive(Debug, Deserialize)]
struct AlejoUser {
    pronoun_id: Option<String>,
}

pub struct PronounsCache {
    inner: Mutex<LruCache<String, (Option<String>, Instant)>>,
    http: reqwest::Client,
}

impl PronounsCache {
    pub fn new(http: reqwest::Client) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(LruCache::new(
                std::num::NonZeroUsize::new(CAPACITY).unwrap(),
            )),
            http,
        })
    }

    /// Returns Some(display) when the user has set pronouns,
    /// None when they haven't (or the lookup failed). Errors are swallowed —
    /// pronouns are best-effort decoration.
    pub async fn lookup(&self, login: &str) -> Option<String> {
        let key = login.to_ascii_lowercase();
        if let Some((cached, when)) = self.inner.lock().get(&key).cloned() {
            if when.elapsed() < TTL {
                return cached;
            }
        }
        let resolved = fetch_pronoun(&self.http, &key).await;
        self.inner
            .lock()
            .put(key, (resolved.clone(), Instant::now()));
        resolved
    }

    #[cfg(test)]
    pub fn insert_for_test(&self, login: &str, value: Option<String>) {
        self.inner
            .lock()
            .put(login.to_ascii_lowercase(), (value, Instant::now()));
    }

    #[cfg(test)]
    pub fn get_cached_for_test(&self, login: &str) -> Option<Option<String>> {
        self.inner
            .lock()
            .peek(&login.to_ascii_lowercase())
            .map(|(v, _)| v.clone())
    }
}

async fn fetch_pronoun(http: &reqwest::Client, login: &str) -> Option<String> {
    let url = format!("https://api.pronouns.alejo.io/v1/users/{login}");
    let resp = match http.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            log::warn!("pronouns lookup failed for {login}: {e:#}");
            return None;
        }
    };
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return None;
    }
    let body: AlejoUser = match resp.json().await {
        Ok(b) => b,
        Err(e) => {
            log::warn!("pronouns decode failed for {login}: {e:#}");
            return None;
        }
    };
    body.pronoun_id.map(|id| display_for(&id))
}

/// Map alejo's pronoun_id codes to display strings.
fn display_for(id: &str) -> String {
    match id {
        "hehim" => "he/him".into(),
        "shehir" => "she/her".into(),
        "theythem" => "they/them".into(),
        "ithem" => "it/them".into(),
        "shehe" => "she/he".into(),
        "shethem" => "she/they".into(),
        "hethem" => "he/they".into(),
        "anyany" => "any/any".into(),
        "other" => "other".into(),
        "askme" => "ask me".into(),
        _ => id.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_table_known_ids() {
        assert_eq!(display_for("hehim"), "he/him");
        assert_eq!(display_for("shehir"), "she/her");
        assert_eq!(display_for("theythem"), "they/them");
    }

    #[test]
    fn display_table_unknown_falls_back_to_id() {
        assert_eq!(display_for("custom_pronoun"), "custom_pronoun");
    }

    #[test]
    fn cache_holds_negative_results() {
        // We use the test-only inserter to avoid hitting the network.
        let http = reqwest::Client::new();
        let c = PronounsCache::new(http);
        c.insert_for_test("nobody", None);
        assert_eq!(c.get_cached_for_test("nobody"), Some(None));
        assert_eq!(c.get_cached_for_test("NOBODY"), Some(None)); // case-insensitive
    }
}
