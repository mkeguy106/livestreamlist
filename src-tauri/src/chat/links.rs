use crate::chat::models::{EmoteRange, LinkRange};

/// Scan `text` for clickable URLs. Skip ranges that overlap any of `existing`
/// (so emote codes that happen to look URL-shaped don't double-tokenize).
///
/// Returns ranges sorted by `start`.
pub fn scan_links(_text: &str, _existing: &[EmoteRange]) -> Vec<LinkRange> {
    Vec::new()
}
