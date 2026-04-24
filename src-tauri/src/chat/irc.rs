use std::collections::HashMap;

/// A single parsed IRCv3 line — the subset we actually use.
///
/// Format (simplified): `[@tags] [:prefix] COMMAND [params...] [:trailing]`
///
/// Tags value-unescaping follows IRCv3.2 §tags.
#[derive(Debug, Clone)]
pub struct IrcMessage<'a> {
    pub tags: HashMap<String, String>,
    pub prefix: Option<&'a str>,
    pub command: &'a str,
    pub params: Vec<&'a str>,
    pub trailing: Option<&'a str>,
}

pub fn parse(line: &str) -> Option<IrcMessage<'_>> {
    let line = line.trim_end_matches(['\r', '\n']);
    if line.is_empty() {
        return None;
    }

    let mut rest = line;
    let mut tags = HashMap::new();

    if let Some(stripped) = rest.strip_prefix('@') {
        let (tag_blob, after) = stripped.split_once(' ')?;
        for raw in tag_blob.split(';') {
            if raw.is_empty() {
                continue;
            }
            let (k, v) = raw.split_once('=').unwrap_or((raw, ""));
            tags.insert(k.to_string(), unescape_tag(v));
        }
        rest = after;
    }

    let mut prefix = None;
    if let Some(stripped) = rest.strip_prefix(':') {
        let (pfx, after) = stripped.split_once(' ')?;
        prefix = Some(pfx);
        rest = after;
    }

    let (command_and_params, trailing) = match rest.find(" :") {
        Some(idx) => (&rest[..idx], Some(&rest[idx + 2..])),
        None => (rest, None),
    };

    let mut parts = command_and_params.split_ascii_whitespace();
    let command = parts.next()?;
    let params: Vec<&str> = parts.collect();

    Some(IrcMessage {
        tags,
        prefix,
        command,
        params,
        trailing,
    })
}

fn unescape_tag(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some(':') => out.push(';'),
                Some('s') => out.push(' '),
                Some('\\') => out.push('\\'),
                Some('r') => out.push('\r'),
                Some('n') => out.push('\n'),
                Some(other) => out.push(other),
                None => break,
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Parse a Twitch `emotes` tag value such as `25:0-4,12-16/1902:6-10` into a
/// Vec of `(emote_id, start_char, end_char)` ranges. Indices are **Unicode
/// scalar value indices** as Twitch spec them — they must be converted to
/// byte offsets using the message text before being used as byte slices.
pub fn parse_twitch_emotes(tag: &str) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    if tag.is_empty() {
        return out;
    }
    for entry in tag.split('/') {
        let Some((id, ranges)) = entry.split_once(':') else {
            continue;
        };
        for range in ranges.split(',') {
            let Some((start, end)) = range.split_once('-') else {
                continue;
            };
            if let (Ok(s), Ok(e)) = (start.parse::<usize>(), end.parse::<usize>()) {
                out.push((id.to_string(), s, e));
            }
        }
    }
    out
}

/// Extract the username from a Twitch IRC prefix like `nick!user@host.tmi.twitch.tv`.
pub fn prefix_nick(prefix: &str) -> &str {
    prefix.split_once('!').map(|(n, _)| n).unwrap_or(prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_privmsg_with_tags() {
        let line = "@badge-info=;badges=;color=#FF0000;display-name=Foo;emotes=25:0-4;id=abc;room-id=1;tmi-sent-ts=1700000000000;user-id=2 :foo!foo@foo.tmi.twitch.tv PRIVMSG #shroud :Kappa hello world";
        let m = parse(line).unwrap();
        assert_eq!(m.command, "PRIVMSG");
        assert_eq!(m.params, vec!["#shroud"]);
        assert_eq!(m.trailing, Some("Kappa hello world"));
        assert_eq!(m.tags.get("display-name").unwrap(), "Foo");
        assert_eq!(m.tags.get("color").unwrap(), "#FF0000");
        assert_eq!(prefix_nick(m.prefix.unwrap()), "foo");
    }

    #[test]
    fn parses_emote_tag() {
        let ranges = parse_twitch_emotes("25:0-4,12-16/1902:6-10");
        assert_eq!(ranges.len(), 3);
        assert_eq!(ranges[0], ("25".to_string(), 0, 4));
        assert_eq!(ranges[1], ("25".to_string(), 12, 16));
        assert_eq!(ranges[2], ("1902".to_string(), 6, 10));
    }

    #[test]
    fn handles_escapes() {
        let line = "@msg-param-sub-plan-name=Nightbot\\scommunity :n PRIVMSG #c :hi";
        let m = parse(line).unwrap();
        assert_eq!(
            m.tags.get("msg-param-sub-plan-name").unwrap(),
            "Nightbot community"
        );
    }
}
