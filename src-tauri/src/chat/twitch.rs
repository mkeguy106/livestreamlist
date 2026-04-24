use anyhow::{Context, Result};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use super::badges::classify_mod_twitch;
use super::emotes::{self, EmoteCache};
use super::irc::{self, IrcMessage};
use super::log_store::ChatLogWriter;
use super::models::{
    ChatBadge, ChatMessage, ChatModerationEvent, ChatStatus, ChatStatusEvent, ChatUser, EmoteRange,
    ReplyInfo, SystemEvent,
};
use super::OutboundMsg;
use crate::platforms::Platform;

const IRC_URL: &str = "wss://irc-ws.chat.twitch.tv";

/// Auth context passed into a chat connection. When `None` the task falls
/// back to an anonymous `justinfan*` read-only session (no sending).
#[derive(Debug, Clone)]
pub struct TwitchAuth {
    pub login: String,
    pub token: String,
}

pub struct TwitchChatConfig {
    pub app: AppHandle,
    #[allow(dead_code)]
    pub http: reqwest::Client,
    pub channel_key: String,
    pub channel_login: String,
    pub emotes: Arc<EmoteCache>,
    #[allow(dead_code)]
    pub badges: Arc<crate::chat::badges::BadgeCache>,
    pub users: Arc<crate::users::UserStore>,
    pub auth: Option<TwitchAuth>,
    pub outbound: mpsc::UnboundedReceiver<OutboundMsg>,
    /// Updated when ROOMSTATE arrives so build_privmsg / build_usernotice
    /// can scope their badge lookups. Interior-mutable since cfg is shared
    /// as `&TwitchChatConfig` through handle_line.
    #[allow(dead_code)]
    pub room_id: parking_lot::Mutex<Option<String>>,
    /// Per-channel own-user badges captured from USERSTATE; used for
    /// local echo of own outgoing messages.
    #[allow(dead_code)]
    pub own_badges: parking_lot::Mutex<Vec<crate::chat::models::ChatBadge>>,
}

/// Run the Twitch IRC connection until dropped/aborted. Emits
/// `chat:message` and `chat:status` events for the given channel_key.
/// Uses the auth token when present, otherwise a read-only `justinfan*` login.
pub async fn run(mut cfg: TwitchChatConfig) {
    emit_status(&cfg.app, &cfg.channel_key, ChatStatus::Connecting, None);

    match connect_and_read(&mut cfg).await {
        Ok(()) => emit_status(&cfg.app, &cfg.channel_key, ChatStatus::Closed, None),
        Err(e) => {
            log::warn!("Twitch IRC for {} errored: {:#}", cfg.channel_login, e);
            emit_status(
                &cfg.app,
                &cfg.channel_key,
                ChatStatus::Error,
                Some(format!("{e:#}")),
            );
        }
    }
}

async fn connect_and_read(cfg: &mut TwitchChatConfig) -> Result<()> {
    let (mut ws, _) = connect_async(IRC_URL)
        .await
        .context("connect wss://irc-ws.chat.twitch.tv")?;

    // Request IRCv3 capabilities for tags + membership + commands.
    ws.send(WsMessage::Text(
        "CAP REQ :twitch.tv/tags twitch.tv/commands twitch.tv/membership".into(),
    ))
    .await?;

    let (pass, nick) = match &cfg.auth {
        Some(auth) => (
            format!("oauth:{}", auth.token),
            auth.login.to_ascii_lowercase(),
        ),
        None => (
            "SCHMOOPIIE".to_string(),
            format!("justinfan{}", rand_suffix()),
        ),
    };
    ws.send(WsMessage::Text(format!("PASS {pass}"))).await?;
    ws.send(WsMessage::Text(format!("NICK {nick}"))).await?;
    ws.send(WsMessage::Text(format!("USER {nick} 8 * :{nick}")))
        .await?;
    ws.send(WsMessage::Text(format!(
        "JOIN #{}",
        cfg.channel_login.to_ascii_lowercase()
    )))
    .await?;

    emit_status(&cfg.app, &cfg.channel_key, ChatStatus::Connected, None);

    let mut log = ChatLogWriter::open(Platform::Twitch, &cfg.channel_login).ok();
    read_loop(cfg, &mut ws, log.as_mut()).await
}

fn rand_suffix() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    10_000 + (seed % 90_000)
}

async fn read_loop(
    cfg: &mut TwitchChatConfig,
    ws: &mut WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    mut log: Option<&mut ChatLogWriter>,
) -> Result<()> {
    loop {
        tokio::select! {
            frame = ws.next() => {
                let Some(frame) = frame else { break };
                match frame? {
                    WsMessage::Text(text) => {
                        for line in text.split("\r\n").filter(|l| !l.is_empty()) {
                            handle_line(cfg, ws, log.as_deref_mut(), line).await?;
                        }
                    }
                    WsMessage::Binary(_) => {}
                    WsMessage::Ping(p) => ws.send(WsMessage::Pong(p)).await?,
                    WsMessage::Pong(_) => {}
                    WsMessage::Close(_) => break,
                    WsMessage::Frame(_) => {}
                }
            }
            Some((text, reply)) = cfg.outbound.recv() => {
                // Outbound is user text — format as PRIVMSG on the way out.
                // IRC has no per-message ack, so ws-write success is as
                // close to a delivery confirmation as we get; Twitch may
                // still silently drop for ratelimit/slow-mode/ban.
                let line = format!("PRIVMSG #{} :{}", cfg.channel_login.to_ascii_lowercase(), text);
                let result = match ws.send(WsMessage::Text(line)).await {
                    Ok(()) => Ok(()),
                    Err(e) => {
                        log::warn!("twitch outbound send failed: {e:#}");
                        Err(format!("{e:#}"))
                    }
                };
                let _ = reply.send(result);
            }
            else => break,
        }
    }
    if let Some(l) = log {
        let _ = l.flush();
    }
    Ok(())
}

async fn handle_line(
    cfg: &TwitchChatConfig,
    ws: &mut WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    log: Option<&mut ChatLogWriter>,
    line: &str,
) -> Result<()> {
    let Some(msg) = irc::parse(line) else {
        return Ok(());
    };

    match msg.command {
        "PING" => {
            let body = msg.trailing.unwrap_or("tmi.twitch.tv");
            ws.send(WsMessage::Text(format!("PONG :{body}"))).await?;
        }
        "PRIVMSG" => {
            if let Some(chat_msg) = build_privmsg(cfg, &msg) {
                persist_and_emit(cfg, log, chat_msg);
            }
        }
        "USERNOTICE" => {
            if let Some(chat_msg) = build_usernotice(cfg, &msg) {
                persist_and_emit(cfg, log, chat_msg);
            }
        }
        "CLEARCHAT" => {
            let ev = build_clearchat(cfg, &msg);
            let _ = cfg
                .app
                .emit(&format!("chat:moderation:{}", cfg.channel_key), ev);
        }
        "CLEARMSG" => {
            let ev = build_clearmsg(cfg, &msg);
            let _ = cfg
                .app
                .emit(&format!("chat:moderation:{}", cfg.channel_key), ev);
        }
        "NOTICE" | "ROOMSTATE" | "USERSTATE" | "GLOBALUSERSTATE" => {
            // Not surfaced yet — state events for room mode / user state live
            // behind their own UI that lands in Phase 4b with preferences.
        }
        _ => {}
    }
    Ok(())
}

fn persist_and_emit(cfg: &TwitchChatConfig, log: Option<&mut ChatLogWriter>, msg: ChatMessage) {
    if let Some(uid) = msg.user.id.as_deref() {
        let key = format!("twitch:{uid}");
        if cfg.users.is_blocked(&key) {
            return; // skip emit AND skip log write
        }
    }
    if let Some(l) = log {
        if let Err(e) = l.append(&msg) {
            log::warn!("chat log append failed for {}: {e:#}", cfg.channel_key);
        }
    }
    let _ = cfg
        .app
        .emit(&format!("chat:message:{}", cfg.channel_key), msg);
}

fn build_privmsg(cfg: &TwitchChatConfig, msg: &IrcMessage<'_>) -> Option<ChatMessage> {
    let trailing = msg.trailing?.to_string();
    let (mut text, is_action) = strip_action(&trailing);

    // Reply-threading: Twitch prepends "@parent-login " to the message body.
    // Strip that prefix so the reply context row carries the attribution and
    // the message body stays clean.
    let reply_to = extract_reply_info(&msg.tags);
    if let Some(r) = &reply_to {
        let prefix_token = format!("@{} ", r.parent_login);
        if text.starts_with(&prefix_token) {
            text = text[prefix_token.len()..].to_string();
        }
    }

    let prefix = msg.prefix.unwrap_or("");
    let login = irc::prefix_nick(prefix).to_string();
    let display_name = msg
        .tags
        .get("display-name")
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| login.clone());

    // Twitch emote ranges from tag use char (UCS) indices; convert to byte offsets.
    let mut emote_ranges: Vec<EmoteRange> = Vec::new();
    if let Some(tag) = msg.tags.get("emotes") {
        for (id, s_char, e_char_incl) in irc::parse_twitch_emotes(tag) {
            let (start, end) = char_range_to_bytes(&text, s_char, e_char_incl + 1);
            let name = text.get(start..end).unwrap_or("").to_string();
            emote_ranges.push(EmoteRange {
                start,
                end,
                name,
                url_1x: emotes::twitch_cdn_url(&id, 1),
                url_2x: Some(emotes::twitch_cdn_url(&id, 2)),
                url_4x: Some(emotes::twitch_cdn_url(&id, 4)),
                animated: false,
            });
        }
    }

    // Overlay 3rd-party emotes for any word not already covered.
    let mut third_party = cfg
        .emotes
        .scan_message(&cfg.channel_key, &text, &emote_ranges);
    emote_ranges.append(&mut third_party);
    emote_ranges.sort_by_key(|r| r.start);

    let id = msg.tags.get("id").cloned().unwrap_or_default();
    let timestamp = msg
        .tags
        .get("tmi-sent-ts")
        .and_then(|s| s.parse::<i64>().ok())
        .and_then(|ms| chrono::DateTime::from_timestamp_millis(ms))
        .unwrap_or_else(Utc::now);

    let color = msg.tags.get("color").filter(|s| !s.is_empty()).cloned();

    Some(ChatMessage {
        id,
        channel_key: cfg.channel_key.clone(),
        platform: Platform::Twitch,
        timestamp,
        user: ChatUser {
            id: msg.tags.get("user-id").cloned(),
            login,
            display_name,
            color,
            is_mod: msg.tags.get("mod").map(|v| v == "1").unwrap_or(false),
            is_subscriber: msg
                .tags
                .get("subscriber")
                .map(|v| v == "1")
                .unwrap_or(false),
            is_broadcaster: msg
                .tags
                .get("badges")
                .map(|b| b.contains("broadcaster/"))
                .unwrap_or(false),
            is_turbo: msg
                .tags
                .get("badges")
                .map(|b| b.contains("turbo/"))
                .unwrap_or(false),
        },
        text,
        emote_ranges,
        badges: parse_badges(msg.tags.get("badges").map(String::as_str).unwrap_or("")),
        is_action,
        is_first_message: msg.tags.get("first-msg").map(|v| v == "1").unwrap_or(false),
        reply_to,
        system: None,
    })
}

fn extract_reply_info(tags: &std::collections::HashMap<String, String>) -> Option<ReplyInfo> {
    let parent_id = tags
        .get("reply-parent-msg-id")
        .filter(|s| !s.is_empty())
        .cloned()?;
    let parent_login = tags
        .get("reply-parent-user-login")
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_default();
    let parent_display_name = tags
        .get("reply-parent-display-name")
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| parent_login.clone());
    let parent_text = tags
        .get("reply-parent-msg-body")
        .cloned()
        .unwrap_or_default();
    Some(ReplyInfo {
        parent_id,
        parent_login,
        parent_display_name,
        parent_text,
    })
}

fn build_usernotice(cfg: &TwitchChatConfig, msg: &IrcMessage<'_>) -> Option<ChatMessage> {
    let kind = msg.tags.get("msg-id").cloned().unwrap_or_default();
    if kind.is_empty() {
        return None;
    }
    let system_text = msg.tags.get("system-msg").cloned().unwrap_or_default();

    // Optional user-attached message (e.g. re-subscriber's custom message).
    let body = msg.trailing.unwrap_or("").to_string();
    let (text, _) = strip_action(&body);

    let login = msg
        .tags
        .get("login")
        .cloned()
        .or_else(|| msg.prefix.map(|p| irc::prefix_nick(p).to_string()))
        .unwrap_or_else(|| "system".to_string());
    let display_name = msg
        .tags
        .get("display-name")
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| login.clone());
    let color = msg.tags.get("color").filter(|s| !s.is_empty()).cloned();

    // Include 3rd-party emotes if the user attached a message.
    let mut emote_ranges: Vec<EmoteRange> = Vec::new();
    if let Some(tag) = msg.tags.get("emotes") {
        for (id, s_char, e_char_incl) in irc::parse_twitch_emotes(tag) {
            let (start, end) = char_range_to_bytes(&text, s_char, e_char_incl + 1);
            let name = text.get(start..end).unwrap_or("").to_string();
            emote_ranges.push(EmoteRange {
                start,
                end,
                name,
                url_1x: super::emotes::twitch_cdn_url(&id, 1),
                url_2x: Some(super::emotes::twitch_cdn_url(&id, 2)),
                url_4x: Some(super::emotes::twitch_cdn_url(&id, 4)),
                animated: false,
            });
        }
    }
    let mut third = cfg
        .emotes
        .scan_message(&cfg.channel_key, &text, &emote_ranges);
    emote_ranges.append(&mut third);
    emote_ranges.sort_by_key(|r| r.start);

    let id = msg.tags.get("id").cloned().unwrap_or_default();
    let timestamp = msg
        .tags
        .get("tmi-sent-ts")
        .and_then(|s| s.parse::<i64>().ok())
        .and_then(|ms| chrono::DateTime::from_timestamp_millis(ms))
        .unwrap_or_else(Utc::now);

    Some(ChatMessage {
        id,
        channel_key: cfg.channel_key.clone(),
        platform: Platform::Twitch,
        timestamp,
        user: ChatUser {
            id: msg.tags.get("user-id").cloned(),
            login,
            display_name,
            color,
            is_mod: false,
            is_subscriber: msg
                .tags
                .get("subscriber")
                .map(|v| v == "1")
                .unwrap_or(false),
            is_broadcaster: false,
            is_turbo: false,
        },
        text,
        emote_ranges,
        badges: parse_badges(msg.tags.get("badges").map(String::as_str).unwrap_or("")),
        is_action: false,
        is_first_message: false,
        reply_to: None,
        system: Some(SystemEvent {
            kind,
            text: system_text,
        }),
    })
}

fn strip_action(s: &str) -> (String, bool) {
    // "\u{0001}ACTION ...\u{0001}"
    if let Some(inner) = s
        .strip_prefix('\u{0001}')
        .and_then(|s| s.strip_suffix('\u{0001}'))
    {
        if let Some(rest) = inner.strip_prefix("ACTION ") {
            return (rest.to_string(), true);
        }
    }
    (s.to_string(), false)
}

fn char_range_to_bytes(text: &str, char_start: usize, char_end: usize) -> (usize, usize) {
    // Twitch emote indices count Unicode scalar values; our text is UTF-8.
    // Walk once, recording byte offsets for the relevant char positions.
    let mut bs = text.len();
    let mut be = text.len();
    let mut done_s = false;
    let mut done_e = false;
    for (char_idx, (byte_idx, _ch)) in text.char_indices().enumerate() {
        if !done_s && char_idx == char_start {
            bs = byte_idx;
            done_s = true;
        }
        if !done_e && char_idx == char_end {
            be = byte_idx;
            done_e = true;
            break;
        }
    }
    (bs, be)
}

fn parse_badges(tag: &str) -> Vec<ChatBadge> {
    if tag.is_empty() {
        return Vec::new();
    }
    tag.split(',')
        .filter_map(|pair| {
            let (set_name, version) = pair.split_once('/')?;
            Some(ChatBadge {
                id: format!("{set_name}/{version}"),
                url: String::new(),
                title: set_name.to_string(),
                is_mod: classify_mod_twitch(set_name),
            })
        })
        .collect()
}

fn build_clearchat(cfg: &TwitchChatConfig, msg: &IrcMessage<'_>) -> ChatModerationEvent {
    // CLEARCHAT has the target login in the trailing (if per-user) or is
    // empty (if full chat wipe). Ban-duration tag distinguishes timeout vs ban.
    let target_login = msg
        .trailing
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
    let ban_duration = msg
        .tags
        .get("ban-duration")
        .and_then(|s| s.parse::<i64>().ok());
    let kind = match (&target_login, ban_duration) {
        (Some(_), Some(_)) => "timeout",
        (Some(_), None) => "ban",
        (None, _) => "clear_chat",
    };
    ChatModerationEvent {
        channel_key: cfg.channel_key.clone(),
        kind: kind.to_string(),
        target_login,
        target_msg_id: None,
        duration_seconds: ban_duration,
    }
}

fn build_clearmsg(cfg: &TwitchChatConfig, msg: &IrcMessage<'_>) -> ChatModerationEvent {
    ChatModerationEvent {
        channel_key: cfg.channel_key.clone(),
        kind: "msg_delete".to_string(),
        target_login: msg.tags.get("login").cloned(),
        target_msg_id: msg.tags.get("target-msg-id").cloned(),
        duration_seconds: None,
    }
}

fn emit_status(app: &AppHandle, channel_key: &str, status: ChatStatus, message: Option<String>) {
    let _ = app.emit(
        &format!("chat:status:{channel_key}"),
        ChatStatusEvent {
            channel_key: channel_key.to_string(),
            status,
            message,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_badges_classifies_mod_vs_cosmetic() {
        let badges = parse_badges("broadcaster/1,subscriber/6,vip/1,turbo/1");
        let map: std::collections::HashMap<&str, bool> =
            badges.iter().map(|b| (b.id.as_str(), b.is_mod)).collect();
        assert_eq!(map.get("broadcaster/1").copied(), Some(true));
        assert_eq!(map.get("vip/1").copied(), Some(true));
        assert_eq!(map.get("subscriber/6").copied(), Some(false));
        assert_eq!(map.get("turbo/1").copied(), Some(false));
    }

    #[test]
    fn parse_badges_empty_returns_empty() {
        assert!(parse_badges("").is_empty());
    }
}
