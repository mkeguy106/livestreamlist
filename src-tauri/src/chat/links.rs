use std::sync::LazyLock;

use regex::Regex;

use crate::chat::models::{EmoteRange, LinkRange};

static SCHEMED_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\bhttps?://[^\s<>"]+"#).expect("schemed URL regex compiles")
});

const TLD_ALLOWLIST: &[&str] = &[
    "com", "net", "org", "io", "gg", "tv", "edu", "gov",
    "co", "uk", "us", "me", "ly", "app", "dev", "fm",
    "live", "stream", "video", "art", "news", "to", "cc",
    "so", "ai", "xyz", "info", "sh",
];

static BARE_DOMAIN_RE: LazyLock<Regex> = LazyLock::new(|| {
    let tlds = TLD_ALLOWLIST.join("|");
    let pat = format!(
        r#"(?i)\b(?:[a-z0-9-]+\.)+(?:{tlds})\b(?:/[^\s<>"]*)?"#
    );
    Regex::new(&pat).expect("bare domain regex compiles")
});

/// Scan `text` for clickable URLs. Skip ranges that overlap any of `existing`
/// (so emote codes that happen to look URL-shaped don't double-tokenize).
///
/// Returns ranges sorted by `start`.
pub fn scan_links(text: &str, existing: &[EmoteRange]) -> Vec<LinkRange> {
    // Collect all candidate (start, end, has_scheme) spans.
    let mut spans: Vec<(usize, usize, bool)> = Vec::new();
    for m in SCHEMED_URL_RE.find_iter(text) {
        spans.push((m.start(), m.end(), true));
    }
    for m in BARE_DOMAIN_RE.find_iter(text) {
        let s = m.start();
        let e = m.end();
        // Skip if overlaps a schemed match (schemed already covers it).
        if spans.iter().any(|(ss, ee, _)| s < *ee && e > *ss) {
            continue;
        }
        spans.push((s, e, false));
    }
    spans.sort_by_key(|(s, _, _)| *s);

    let mut out = Vec::new();
    for (start, raw_end, has_scheme) in spans {
        let end = start + trim_url_end(&text[start..raw_end]);
        if end <= start {
            continue;
        }
        if existing.iter().any(|r| start < r.end && end > r.start) {
            continue;
        }
        let raw = &text[start..end];
        let candidate = if has_scheme {
            raw.to_string()
        } else {
            format!("https://{raw}")
        };
        if let Ok(parsed) = url::Url::parse(&candidate) {
            out.push(LinkRange { start, end, url: parsed.to_string() });
        }
    }
    out
}

/// Returns the new candidate length (in bytes, relative to the start of `s`)
/// after stripping trailing sentence punctuation and balancing parens/brackets.
/// Implements the GFM autolink / linkify-it algorithm: peel off `.,;:!?'\"*_`
/// from the end while present; for each trailing `)` or `]`, drop it if
/// unbalanced (more closes than opens within the candidate); repeat until
/// stable.
fn trim_url_end(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut len = bytes.len();
    loop {
        let prev = len;
        // Strip trailing punctuation.
        while len > 0 {
            let c = bytes[len - 1];
            if matches!(c, b'.' | b',' | b';' | b':' | b'!' | b'?' | b'\'' | b'"' | b'*' | b'_') {
                len -= 1;
            } else {
                break;
            }
        }
        // Strip a single unbalanced trailing `)` or `]`, then loop back to
        // the punctuation pass (a closing bracket might now be followed by
        // a `.` again, etc.).
        if len > 0 {
            let last = bytes[len - 1];
            if last == b')' || last == b']' {
                let (open, close) = if last == b')' { (b'(', b')') } else { (b'[', b']') };
                let mut opens = 0i32;
                let mut closes = 0i32;
                for &c in &bytes[..len] {
                    if c == open {
                        opens += 1;
                    } else if c == close {
                        closes += 1;
                    }
                }
                if closes > opens {
                    len -= 1;
                    continue; // re-run the punctuation pass
                }
            }
        }
        if len == prev {
            break;
        }
    }
    len
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

    #[test]
    fn strip_trailing_period() {
        let got = scan_links("see https://example.com.", &[]);
        assert_eq!(got, vec![r(4, 23, "https://example.com/")]);
    }

    #[test]
    fn strip_trailing_punctuation_set() {
        // Each of .,;:!?'"*_ should not become part of the URL.
        let got = scan_links("hi https://example.com! bye", &[]);
        assert_eq!(got, vec![r(3, 22, "https://example.com/")]);
    }

    #[test]
    fn parens_unbalanced_outer() {
        // (https://example.com) — outer parens belong to the surrounding text.
        let got = scan_links("(https://example.com) bye", &[]);
        assert_eq!(got, vec![r(1, 20, "https://example.com/")]);
    }

    #[test]
    fn parens_balanced_inside_url() {
        // Wikipedia URL with balanced ( ) inside — keep the closing paren.
        // "https://en.wikipedia.org/wiki/Foo_(bar)" is 39 bytes; starts at 4 → end = 43.
        let got = scan_links("see https://en.wikipedia.org/wiki/Foo_(bar) end", &[]);
        assert_eq!(
            got,
            vec![r(4, 43, "https://en.wikipedia.org/wiki/Foo_(bar)")]
        );
    }

    #[test]
    fn bare_domain_allowlisted() {
        let got = scan_links("yo youtube.com/watch?v=abc end", &[]);
        assert_eq!(
            got,
            vec![r(3, 26, "https://youtube.com/watch?v=abc")]
        );
    }

    #[test]
    fn bare_domain_no_path() {
        let got = scan_links("visit example.com later", &[]);
        assert_eq!(got, vec![r(6, 17, "https://example.com/")]);
    }

    #[test]
    fn bare_domain_tld_not_in_allowlist() {
        // `.story` is not on the allowlist; treat as plain text.
        let got = scan_links("hey cool.story bro", &[]);
        assert!(got.is_empty(), "expected no link, got {:?}", got);
    }

    #[test]
    fn bare_domain_subdomain() {
        let got = scan_links("watch live.twitch.tv/shroud now", &[]);
        assert_eq!(
            got,
            vec![r(6, 27, "https://live.twitch.tv/shroud")]
        );
    }
}
