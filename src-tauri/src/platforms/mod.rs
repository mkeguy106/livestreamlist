pub mod chaturbate;
pub mod kick;
pub mod pronouns;
pub mod twitch;
pub mod twitch_anniversary;
pub mod twitch_users;
pub mod youtube;

use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    #[default]
    Twitch,
    Youtube,
    Kick,
    Chaturbate,
}

impl Platform {
    pub fn as_str(&self) -> &'static str {
        match self {
            Platform::Twitch => "twitch",
            Platform::Youtube => "youtube",
            Platform::Kick => "kick",
            Platform::Chaturbate => "chaturbate",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "twitch" => Some(Platform::Twitch),
            "youtube" => Some(Platform::Youtube),
            "kick" => Some(Platform::Kick),
            "chaturbate" => Some(Platform::Chaturbate),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ParsedChannel {
    pub platform: Platform,
    pub channel_id: String,
    pub display_name: String,
}

/// Detect platform and canonical channel id from a pasted URL or bare handle.
///
/// Accepts:
///   - `https://twitch.tv/shroud` / `https://www.twitch.tv/shroud`
///   - `https://youtube.com/@LudwigAhgren` / `/channel/UC...`
///   - `https://kick.com/trainwreckstv`
///   - `https://chaturbate.com/siswet19/`
///   - Bare handles like `shroud`, `@ludwig`, `t:shroud`, `k:xqc`
pub fn parse_channel_input(input: &str) -> Option<ParsedChannel> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Explicit prefix: "t:name", "y:name", "k:name", "c:name"
    if let Some((prefix, rest)) = trimmed.split_once(':') {
        if let Some(p) = match prefix.to_ascii_lowercase().as_str() {
            "t" | "twitch" => Some(Platform::Twitch),
            "y" | "youtube" | "yt" => Some(Platform::Youtube),
            "k" | "kick" => Some(Platform::Kick),
            "c" | "cb" | "chaturbate" => Some(Platform::Chaturbate),
            _ => None,
        } {
            let id = rest.trim().trim_start_matches('@').to_string();
            if !id.is_empty() {
                return Some(ParsedChannel {
                    platform: p,
                    display_name: id.clone(),
                    channel_id: id,
                });
            }
        }
    }

    // URL path
    if let Ok(url) = Url::parse(trimmed).or_else(|_| Url::parse(&format!("https://{trimmed}"))) {
        let host = url.host_str().unwrap_or("").to_ascii_lowercase();
        let segments: Vec<&str> = url
            .path_segments()
            .map(|s| s.filter(|p| !p.is_empty()).collect())
            .unwrap_or_default();

        if host.contains("twitch.tv") {
            if let Some(first) = segments.first() {
                let id = first.trim_start_matches('@').to_string();
                return Some(ParsedChannel {
                    platform: Platform::Twitch,
                    display_name: id.clone(),
                    channel_id: id,
                });
            }
        }
        if host.contains("youtube.com") || host == "youtu.be" {
            if let Some(first) = segments.first() {
                if *first == "channel" {
                    if let Some(id) = segments.get(1) {
                        return Some(ParsedChannel {
                            platform: Platform::Youtube,
                            display_name: (*id).to_string(),
                            channel_id: (*id).to_string(),
                        });
                    }
                }
                let id = first.trim_start_matches('@').to_string();
                return Some(ParsedChannel {
                    platform: Platform::Youtube,
                    display_name: id.clone(),
                    channel_id: id,
                });
            }
        }
        if host.contains("kick.com") {
            if let Some(first) = segments.first() {
                return Some(ParsedChannel {
                    platform: Platform::Kick,
                    display_name: (*first).to_string(),
                    channel_id: (*first).to_string(),
                });
            }
        }
        if host.contains("chaturbate.com") {
            if let Some(first) = segments.first() {
                return Some(ParsedChannel {
                    platform: Platform::Chaturbate,
                    display_name: (*first).to_string(),
                    channel_id: (*first).to_string(),
                });
            }
        }
    }

    // Bare handle — assume Twitch (most common platform for this user base)
    let id = trimmed.trim_start_matches('@').to_string();
    if id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Some(ParsedChannel {
            platform: Platform::Twitch,
            display_name: id.clone(),
            channel_id: id,
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_twitch_url() {
        let p = parse_channel_input("https://www.twitch.tv/shroud").unwrap();
        assert_eq!(p.platform, Platform::Twitch);
        assert_eq!(p.channel_id, "shroud");
    }

    #[test]
    fn parses_youtube_handle() {
        let p = parse_channel_input("https://youtube.com/@LudwigAhgren").unwrap();
        assert_eq!(p.platform, Platform::Youtube);
        assert_eq!(p.channel_id, "LudwigAhgren");
    }

    #[test]
    fn parses_youtube_channel_id() {
        let p = parse_channel_input("https://youtube.com/channel/UC1234567890").unwrap();
        assert_eq!(p.platform, Platform::Youtube);
        assert_eq!(p.channel_id, "UC1234567890");
    }

    #[test]
    fn parses_kick() {
        let p = parse_channel_input("https://kick.com/trainwreckstv").unwrap();
        assert_eq!(p.platform, Platform::Kick);
        assert_eq!(p.channel_id, "trainwreckstv");
    }

    #[test]
    fn parses_chaturbate() {
        let p = parse_channel_input("https://chaturbate.com/siswet19/").unwrap();
        assert_eq!(p.platform, Platform::Chaturbate);
        assert_eq!(p.channel_id, "siswet19");
    }

    #[test]
    fn parses_bare_handle_as_twitch() {
        let p = parse_channel_input("shroud").unwrap();
        assert_eq!(p.platform, Platform::Twitch);
        assert_eq!(p.channel_id, "shroud");
    }

    #[test]
    fn parses_prefix_notation() {
        let p = parse_channel_input("k:xqc").unwrap();
        assert_eq!(p.platform, Platform::Kick);
        assert_eq!(p.channel_id, "xqc");
    }
}
