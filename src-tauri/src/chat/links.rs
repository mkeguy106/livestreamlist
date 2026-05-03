use std::sync::LazyLock;

use regex::Regex;

use crate::chat::models::{EmoteRange, LinkRange};

static SCHEMED_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\bhttps?://[^\s<>"]+"#).expect("schemed URL regex compiles")
});

/// Scan `text` for clickable URLs. Skip ranges that overlap any of `existing`
/// (so emote codes that happen to look URL-shaped don't double-tokenize).
///
/// Returns ranges sorted by `start`.
pub fn scan_links(text: &str, existing: &[EmoteRange]) -> Vec<LinkRange> {
    let mut out = Vec::new();
    for m in SCHEMED_URL_RE.find_iter(text) {
        let start = m.start();
        let end = m.end();
        if existing.iter().any(|r| start < r.end && end > r.start) {
            continue;
        }
        let raw = &text[start..end];
        if let Ok(parsed) = url::Url::parse(raw) {
            out.push(LinkRange { start, end, url: parsed.to_string() });
        }
    }
    out.sort_by_key(|r| r.start);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(start: usize, end: usize, url: &str) -> LinkRange {
        LinkRange { start, end, url: url.to_string() }
    }

    #[test]
    fn schemed_https_basic() {
        let got = scan_links("check https://example.com", &[]);
        assert_eq!(got, vec![r(6, 25, "https://example.com/")]);
    }

    #[test]
    fn schemed_http_basic() {
        let got = scan_links("see http://example.com here", &[]);
        assert_eq!(got, vec![r(4, 22, "http://example.com/")]);
    }

    #[test]
    fn no_match_plain_text() {
        let got = scan_links("hello world", &[]);
        assert!(got.is_empty());
    }
}
