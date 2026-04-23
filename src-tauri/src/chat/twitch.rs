use anyhow::{Context, Result};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use super::emotes::{self, EmoteCache};
use super::irc::{self, IrcMessage};
use super::models::{ChatBadge, ChatMessage, ChatStatus, ChatStatusEvent, ChatUser, EmoteRange};
use crate::platforms::Platform;

const IRC_URL: &str = "wss://irc-ws.chat.twitch.tv";

pub struct TwitchChatConfig {
    pub app: AppHandle,
    pub channel_key: String,
    pub channel_login: String,
    pub emotes: Arc<EmoteCache>,
}

/// Run the anonymous Twitch IRC connection until dropped/aborted. Emits
/// `chat:message` and `chat:status` events for the given channel_key.
pub async fn run(cfg: TwitchChatConfig) {
    emit_status(&cfg.app, &cfg.channel_key, ChatStatus::Connecting, None);

    match connect_and_read(&cfg).await {
        Ok(()) => emit_status(&cfg.app, &cfg.channel_key, ChatStatus::Closed, None),
        Err(e) => {
            log::warn!("Twitch IRC for {} errored: {:#}", cfg.channel_login, e);
            emit_status(&cfg.app, &cfg.channel_key, ChatStatus::Error, Some(format!("{e:#}")));
        }
    }
}

async fn connect_and_read(cfg: &TwitchChatConfig) -> Result<()> {
    let (mut ws, _) = connect_async(IRC_URL).await.context("connect wss://irc-ws.chat.twitch.tv")?;

    // Request IRCv3 capabilities for tags + membership + commands.
    ws.send(WsMessage::Text(
        "CAP REQ :twitch.tv/tags twitch.tv/commands twitch.tv/membership".into(),
    ))
    .await?;

    // Anonymous connection — Twitch lets any justinfan* nick join read-only.
    let nick = format!("justinfan{}", rand_suffix());
    ws.send(WsMessage::Text(format!("PASS SCHMOOPIIE"))).await?;
    ws.send(WsMessage::Text(format!("NICK {nick}"))).await?;
    ws.send(WsMessage::Text(format!("USER {nick} 8 * :{nick}"))).await?;
    ws.send(WsMessage::Text(format!("JOIN #{}", cfg.channel_login.to_ascii_lowercase()))).await?;

    emit_status(&cfg.app, &cfg.channel_key, ChatStatus::Connected, None);

    read_loop(cfg, &mut ws).await
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
    cfg: &TwitchChatConfig,
    ws: &mut WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
) -> Result<()> {
    while let Some(frame) = ws.next().await {
        match frame? {
            WsMessage::Text(text) => {
                for line in text.split("\r\n").filter(|l| !l.is_empty()) {
                    handle_line(cfg, ws, line).await?;
                }
            }
            WsMessage::Binary(_) => {}
            WsMessage::Ping(p) => ws.send(WsMessage::Pong(p)).await?,
            WsMessage::Pong(_) => {}
            WsMessage::Close(_) => break,
            WsMessage::Frame(_) => {}
        }
    }
    Ok(())
}

async fn handle_line(
    cfg: &TwitchChatConfig,
    ws: &mut WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    line: &str,
) -> Result<()> {
    let Some(msg) = irc::parse(line) else { return Ok(()) };

    match msg.command {
        "PING" => {
            let body = msg.trailing.unwrap_or("tmi.twitch.tv");
            ws.send(WsMessage::Text(format!("PONG :{body}"))).await?;
        }
        "PRIVMSG" => {
            if let Some(chat_msg) = build_privmsg(cfg, &msg) {
                let _ = cfg
                    .app
                    .emit(&format!("chat:message:{}", cfg.channel_key), chat_msg);
            }
        }
        "USERNOTICE" | "NOTICE" | "ROOMSTATE" | "USERSTATE" | "GLOBALUSERSTATE" | "CLEARCHAT"
        | "CLEARMSG" => {
            // Handled in Phase 3 (banners, moderation).
        }
        _ => {}
    }
    Ok(())
}

fn build_privmsg(cfg: &TwitchChatConfig, msg: &IrcMessage<'_>) -> Option<ChatMessage> {
    let trailing = msg.trailing?.to_string();
    let (text, is_action) = strip_action(&trailing);

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
    let mut third_party = cfg.emotes.scan_message(&cfg.channel_key, &text, &emote_ranges);
    emote_ranges.append(&mut third_party);
    emote_ranges.sort_by_key(|r| r.start);

    let id = msg.tags.get("id").cloned().unwrap_or_default();
    let timestamp = msg
        .tags
        .get("tmi-sent-ts")
        .and_then(|s| s.parse::<i64>().ok())
        .and_then(|ms| chrono::DateTime::from_timestamp_millis(ms))
        .unwrap_or_else(Utc::now);

    let color = msg
        .tags
        .get("color")
        .filter(|s| !s.is_empty())
        .cloned();

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
            is_subscriber: msg.tags.get("subscriber").map(|v| v == "1").unwrap_or(false),
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
        is_first_message: msg
            .tags
            .get("first-msg")
            .map(|v| v == "1")
            .unwrap_or(false),
        reply_to: None,
    })
}

fn strip_action(s: &str) -> (String, bool) {
    // "\u{0001}ACTION ...\u{0001}"
    if let Some(inner) = s.strip_prefix('\u{0001}').and_then(|s| s.strip_suffix('\u{0001}')) {
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
    // Without channel badge URLs we just note the ids; frontend can style by id.
    if tag.is_empty() {
        return Vec::new();
    }
    tag.split(',')
        .filter_map(|pair| {
            let (id, version) = pair.split_once('/')?;
            Some(ChatBadge {
                id: format!("{id}/{version}"),
                url: String::new(),
                title: id.to_string(),
            })
        })
        .collect()
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
