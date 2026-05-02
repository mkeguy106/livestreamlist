//! Pure tokenizer. Splits a chat message into typed token ranges so the
//! caller can decide which tokens to spellcheck.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenClass {
    /// A regular word — should be spellchecked.
    Word,
    /// `@username` — skip.
    Mention,
    /// `https://…` or bare `domain.tld[/path]` — skip.
    Url,
    /// Emote code — either `:colon-form:` or an exact match against the
    /// per-channel emote list. Skip.
    Emote,
    /// 3+ uppercase letters with no internal punctuation (`LOL`, `LMAO`).
    /// Skip.
    AllCaps,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenRange {
    pub class: TokenClass,
    /// Byte offset (start, end) into the input text. UTF-8 safe.
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// Split `text` into typed tokens. `channel_emotes` is the per-channel
/// emote-name list (used only by the `Emote` classifier — exact matches
/// against this list are tagged as emotes regardless of casing).
pub fn tokenize(text: &str, channel_emotes: &[String]) -> Vec<TokenRange> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Skip whitespace.
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() { break; }

        let token_start = i;
        // A token runs until whitespace. We then trim outer punctuation
        // and classify.
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let raw_end = i;
        let raw = &text[token_start..raw_end];

        // Trim leading/trailing punctuation (parens, quotes, commas, etc.)
        // that sit OUTSIDE the spellable word. We don't trim @, :, /, ., -
        // because those carry meaning (mention, emote, URL, hyphenated word).
        let trim_chars: &[char] = &['(', ')', ',', '!', '?', '"', '\'', ';', '`'];
        let trimmed = raw.trim_matches(trim_chars);
        if trimmed.is_empty() { continue; }
        let inner_start_offset = raw.find(trimmed).unwrap_or(0);
        let start = token_start + inner_start_offset;
        let end = start + trimmed.len();

        let class = classify(trimmed, channel_emotes);
        out.push(TokenRange {
            class,
            start,
            end,
            text: trimmed.to_string(),
        });
    }
    out
}

fn classify(token: &str, channel_emotes: &[String]) -> TokenClass {
    if token.starts_with('@') && token.len() > 1 {
        return TokenClass::Mention;
    }
    if is_url(token) {
        return TokenClass::Url;
    }
    if is_colon_emote(token) {
        return TokenClass::Emote;
    }
    if channel_emotes.iter().any(|e| e == token) {
        return TokenClass::Emote;
    }
    if is_all_caps_shorthand(token) {
        return TokenClass::AllCaps;
    }
    TokenClass::Word
}

fn is_url(token: &str) -> bool {
    if token.starts_with("https://") || token.starts_with("http://") {
        return true;
    }
    // Bare domain heuristic: contains a `.`, the part before the last `.`
    // is alphanumeric+hyphens, the part after is 2+ alpha (TLD-like).
    if let Some(dot_idx) = token.rfind('.') {
        let before = &token[..dot_idx];
        let after_dot = &token[dot_idx + 1..];
        let after_tld = after_dot.split('/').next().unwrap_or(after_dot);
        let tld_ok = after_tld.len() >= 2 && after_tld.chars().all(|c| c.is_ascii_alphabetic());
        let before_ok = !before.is_empty()
            && before.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
        if tld_ok && before_ok {
            return true;
        }
    }
    false
}

fn is_colon_emote(token: &str) -> bool {
    token.len() >= 3
        && token.starts_with(':')
        && token.ends_with(':')
        && token[1..token.len() - 1]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_all_caps_shorthand(token: &str) -> bool {
    if token.chars().count() < 3 { return false; }
    token.chars().all(|c| c.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes(text: &str, emotes: &[&str]) -> Vec<(TokenClass, &'static str)> {
        let owned: Vec<String> = emotes.iter().map(|s| s.to_string()).collect();
        tokenize(text, &owned)
            .into_iter()
            .map(|t| (t.class, leak_str(t.text)))
            .collect()
    }
    fn leak_str(s: String) -> &'static str { Box::leak(s.into_boxed_str()) }

    #[test]
    fn plain_words() {
        assert_eq!(
            classes("hello world", &[]),
            vec![(TokenClass::Word, "hello"), (TokenClass::Word, "world")],
        );
    }

    #[test]
    fn mentions_are_skipped() {
        assert_eq!(
            classes("hi @shroud how are you", &[]),
            vec![
                (TokenClass::Word, "hi"),
                (TokenClass::Mention, "@shroud"),
                (TokenClass::Word, "how"),
                (TokenClass::Word, "are"),
                (TokenClass::Word, "you"),
            ],
        );
    }

    #[test]
    fn urls_are_skipped() {
        // Both schemed and bare URLs.
        let r = classes("watch https://twitch.tv/shroud now", &[]);
        assert!(r.iter().any(|(c, t)| *c == TokenClass::Url && *t == "https://twitch.tv/shroud"));
        let r = classes("twitch.tv/shroud is live", &[]);
        assert!(r.iter().any(|(c, t)| *c == TokenClass::Url && *t == "twitch.tv/shroud"));
    }

    #[test]
    fn emote_list_match() {
        let r = classes("Kappa Kappa forsenE", &["Kappa", "forsenE"]);
        assert_eq!(r.len(), 3);
        assert!(r.iter().all(|(c, _)| *c == TokenClass::Emote));
    }

    #[test]
    fn colon_form_emote() {
        let r = classes("hello :Kappa: world", &[]);
        assert!(r.iter().any(|(c, t)| *c == TokenClass::Emote && *t == ":Kappa:"));
    }

    #[test]
    fn all_caps_shorthand() {
        let r = classes("LOL that was great LMAO", &[]);
        assert!(r.iter().any(|(c, t)| *c == TokenClass::AllCaps && *t == "LOL"));
        assert!(r.iter().any(|(c, t)| *c == TokenClass::AllCaps && *t == "LMAO"));
    }

    #[test]
    fn two_letter_caps_are_words_not_caps() {
        // Ambiguous pronouns ("OK", "AM"/"PM") are real words; the caps
        // skip-rule is 3+ chars to avoid false positives.
        let r = classes("OK fine", &[]);
        assert_eq!(r[0].0, TokenClass::Word);
    }

    #[test]
    fn unicode_words() {
        // Common Latin-1 + diacritics must tokenize as single words (no
        // splitting at ä / é).
        let r = classes("schöne grüße", &[]);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].1, "schöne");
        assert_eq!(r[1].1, "grüße");
    }

    #[test]
    fn byte_offsets_are_correct() {
        // Caller relies on these to slice the original string.
        let text = "abc def";
        let toks = tokenize(text, &[]);
        assert_eq!(toks[0].start, 0);
        assert_eq!(toks[0].end, 3);
        assert_eq!(toks[1].start, 4);
        assert_eq!(toks[1].end, 7);
        assert_eq!(&text[toks[0].start..toks[0].end], "abc");
        assert_eq!(&text[toks[1].start..toks[1].end], "def");
    }

    #[test]
    fn mixed_punctuation_around_words() {
        // Trailing/leading punctuation must be stripped; the word inside
        // is what's tokenized.
        let r = classes("(hello), 'world'!", &[]);
        let words: Vec<&str> = r.iter().filter(|(c, _)| *c == TokenClass::Word).map(|(_, t)| *t).collect();
        assert_eq!(words, vec!["hello", "world"]);
    }
}
