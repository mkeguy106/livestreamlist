//! Kick chat via Pusher WebSocket (read) + REST (send).
//!
//! Kick uses the Pusher protocol. On connect we receive
//! `pusher:connection_established`, then subscribe to `chatrooms.{id}.v2`.
//! Chat arrives as `App\Events\ChatMessageEvent` with JSON-string data.
//!
//! Sending goes through `POST https://api.kick.com/public/v1/chat` with a
//! bearer token. When `type=user`, Kick requires `broadcaster_user_id` in
//! the body — the token identifies the *sender*, not the *room*. On 401 we
//! refresh the token once and retry.
//!
//! Kick echoes your own messages back over websocket (unlike Twitch), so
//! we never synthesize local echoes — they'd double.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use super::emotes::EmoteCache;
use super::log_store::ChatLogWriter;
use super::models::{ChatBadge, ChatMessage, ChatStatus, ChatStatusEvent, ChatUser, EmoteRange};
use super::OutboundMsg;
use crate::auth;
use crate::platforms::Platform;

const PUSHER_WS_URL: &str = "wss://ws-us2.pusher.com/app/32cbd69e4b950bf97679";
const PUSHER_PARAMS: &str = "?protocol=7&client=js&version=8.3.0&flash=false";
const CHANNEL_INFO_URL: &str = "https://kick.com/api/v2/channels";
const SEND_URL: &str = "https://api.kick.com/public/v1/chat";

pub struct KickChatConfig {
    pub app: AppHandle,
    pub http: reqwest::Client,
    pub channel_key: String,
    pub channel_slug: String,
    #[allow(dead_code)]
    pub emotes: Arc<EmoteCache>,
    pub outbound: mpsc::UnboundedReceiver<OutboundMsg>,
}

struct KickChannelIds {
    chatroom_id: u64,
    broadcaster_user_id: u64,
}

pub async fn run(mut cfg: KickChatConfig) {
    emit_status(&cfg.app, &cfg.channel_key, ChatStatus::Connecting, None);

    match connect_and_read(&mut cfg).await {
        Ok(()) => emit_status(&cfg.app, &cfg.channel_key, ChatStatus::Closed, None),
        Err(e) => {
            log::warn!("Kick Pusher for {} errored: {:#}", cfg.channel_slug, e);
            emit_status(
                &cfg.app,
                &cfg.channel_key,
                ChatStatus::Error,
                Some(format!("{e:#}")),
            );
        }
    }
}

async fn connect_and_read(cfg: &mut KickChatConfig) -> Result<()> {
    let ids = resolve_channel_ids(&cfg.http, &cfg.channel_slug).await?;

    let url = format!("{PUSHER_WS_URL}{PUSHER_PARAMS}");
    let (mut ws, _) = connect_async(&url).await.context("connect Pusher ws-us2")?;

    // Wait for connection_established before subscribing so the Pusher
    // server has a socket_id for us.
    wait_for_connection_established(&mut ws).await?;

    let subscribe = json!({
        "event": "pusher:subscribe",
        "data": { "auth": "", "channel": format!("chatrooms.{}.v2", ids.chatroom_id) }
    });
    ws.send(WsMessage::Text(subscribe.to_string())).await?;

    emit_status(&cfg.app, &cfg.channel_key, ChatStatus::Connected, None);

    let mut log = ChatLogWriter::open(Platform::Kick, &cfg.channel_slug).ok();
    read_loop(cfg, &mut ws, log.as_mut(), &ids).await
}

async fn wait_for_connection_established(
    ws: &mut WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
) -> Result<()> {
    while let Some(frame) = ws.next().await {
        match frame? {
            WsMessage::Text(text) => {
                let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                if parsed.get("event").and_then(|v| v.as_str())
                    == Some("pusher:connection_established")
                {
                    return Ok(());
                }
            }
            WsMessage::Ping(p) => ws.send(WsMessage::Pong(p)).await?,
            WsMessage::Close(_) => anyhow::bail!("closed before connection_established"),
            _ => {}
        }
    }
    anyhow::bail!("stream ended before connection_established")
}

async fn read_loop(
    cfg: &mut KickChatConfig,
    ws: &mut WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    mut log: Option<&mut ChatLogWriter>,
    ids: &KickChannelIds,
) -> Result<()> {
    loop {
        tokio::select! {
            frame = ws.next() => {
                let Some(frame) = frame else { break };
                match frame? {
                    WsMessage::Text(text) => {
                        handle_pusher_line(cfg, ws, log.as_deref_mut(), &text).await?;
                    }
                    WsMessage::Binary(_) => {}
                    WsMessage::Ping(p) => ws.send(WsMessage::Pong(p)).await?,
                    WsMessage::Pong(_) => {}
                    WsMessage::Close(_) => break,
                    WsMessage::Frame(_) => {}
                }
            }
            Some((text, reply)) = cfg.outbound.recv() => {
                let result = send_via_rest(&cfg.http, ids.broadcaster_user_id, &text).await;
                if let Err(e) = &result {
                    log::warn!("Kick send failed: {e:#}");
                    emit_status(
                        &cfg.app,
                        &cfg.channel_key,
                        ChatStatus::Error,
                        Some(format!("send failed: {e:#}")),
                    );
                }
                let _ = reply.send(result.map_err(|e| format!("{e:#}")));
            }
            else => break,
        }
    }
    if let Some(l) = log {
        let _ = l.flush();
    }
    Ok(())
}

async fn handle_pusher_line(
    cfg: &KickChatConfig,
    ws: &mut WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    log: Option<&mut ChatLogWriter>,
    line: &str,
) -> Result<()> {
    let parsed: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    let event = parsed.get("event").and_then(|v| v.as_str()).unwrap_or("");

    match event {
        "pusher:ping" => {
            ws.send(WsMessage::Text(
                json!({"event":"pusher:pong","data":""}).to_string(),
            ))
            .await?;
        }
        "pusher:pong"
        | "pusher:connection_established"
        | "pusher_internal:subscription_succeeded" => {}
        "App\\Events\\ChatMessageEvent" => {
            if let Some(chat_msg) = build_chat_message(cfg, &parsed) {
                if let Some(l) = log {
                    let _ = l.append(&chat_msg);
                }
                let _ = cfg
                    .app
                    .emit(&format!("chat:message:{}", cfg.channel_key), chat_msg);
            }
        }
        _ => {
            // Moderation + banner events land in Phase 3 follow-ups.
        }
    }
    Ok(())
}

fn build_chat_message(cfg: &KickChatConfig, parsed: &Value) -> Option<ChatMessage> {
    // Pusher wraps event payloads as JSON *strings* in `.data`.
    let data_str = parsed.get("data").and_then(|v| v.as_str())?;
    let data: Value = serde_json::from_str(data_str).ok()?;

    let content = data
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let id = data
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let sender = data.get("sender")?;
    let sender_id = sender
        .get("id")
        .map(|v| v.to_string().trim_matches('"').to_string())
        .unwrap_or_default();
    let login = sender
        .get("username")
        .and_then(|v| v.as_str())
        .or_else(|| sender.get("slug").and_then(|v| v.as_str()))
        .unwrap_or("unknown")
        .to_string();
    let display_name = sender
        .get("username")
        .and_then(|v| v.as_str())
        .unwrap_or(&login)
        .to_string();
    let color = sender
        .pointer("/identity/color")
        .and_then(|v| v.as_str())
        .map(String::from);

    let (stripped, emote_ranges) = extract_kick_emotes(&content);

    let timestamp = data
        .get("created_at")
        .and_then(|v| v.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    let badges: Vec<ChatBadge> = sender
        .pointer("/identity/badges")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|b| {
                    let t = b.get("type").and_then(|v| v.as_str())?.to_string();
                    let text = b
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&t)
                        .to_string();
                    Some(ChatBadge {
                        id: t.clone(),
                        url: String::new(),
                        title: text,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Some(ChatMessage {
        id,
        channel_key: cfg.channel_key.clone(),
        platform: Platform::Kick,
        timestamp,
        user: ChatUser {
            id: Some(sender_id),
            login,
            display_name,
            color,
            is_mod: false,
            is_subscriber: false,
            is_broadcaster: false,
            is_turbo: false,
        },
        text: stripped,
        emote_ranges,
        badges,
        is_action: false,
        is_first_message: false,
        reply_to: None,
        system: None,
    })
}

/// Convert Kick's inline `[emote:ID:name]` markers into the rendered name
/// plus an `EmoteRange` array that points back into the resulting text.
fn extract_kick_emotes(content: &str) -> (String, Vec<EmoteRange>) {
    let mut out = String::with_capacity(content.len());
    let mut ranges: Vec<EmoteRange> = Vec::new();
    let bytes = content.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' && bytes[i..].starts_with(b"[emote:") {
            // Parse `[emote:ID:NAME]`
            let rest = &content[i + 7..];
            if let Some(end_rel) = rest.find(']') {
                let body = &rest[..end_rel];
                if let Some((id, name)) = body.split_once(':') {
                    let start_byte = out.len();
                    out.push_str(name);
                    let end_byte = out.len();
                    ranges.push(EmoteRange {
                        start: start_byte,
                        end: end_byte,
                        name: name.to_string(),
                        url_1x: format!("https://files.kick.com/emotes/{id}/fullsize"),
                        url_2x: None,
                        url_4x: None,
                        animated: false,
                    });
                    i += 7 + end_rel + 1;
                    continue;
                }
            }
        }
        // Regular byte → copy as-is
        out.push(bytes[i] as char);
        i += 1;
    }
    (out, ranges)
}

async fn resolve_channel_ids(http: &reqwest::Client, slug: &str) -> Result<KickChannelIds> {
    let url = format!("{CHANNEL_INFO_URL}/{slug}");
    let resp = http
        .get(&url)
        .header("Accept", "application/json")
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    if !resp.status().is_success() {
        anyhow::bail!(
            "Kick channel lookup {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
    }
    let data: Value = resp.json().await?;
    parse_channel_ids(&data)
}

fn parse_channel_ids(data: &Value) -> Result<KickChannelIds> {
    let chatroom_id = data
        .pointer("/chatroom/id")
        .and_then(|v| v.as_u64())
        .context("chatroom.id missing in Kick channel response")?;
    // Prefer top-level `user_id`; fall back to `user.id`. Some responses
    // return it as a string, so coerce both cases.
    let broadcaster_user_id = data
        .get("user_id")
        .and_then(value_to_u64)
        .or_else(|| data.pointer("/user/id").and_then(value_to_u64))
        .context("user_id missing in Kick channel response")?;
    Ok(KickChannelIds {
        chatroom_id,
        broadcaster_user_id,
    })
}

fn value_to_u64(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse::<u64>().ok(),
        _ => None,
    }
}

async fn send_via_rest(http: &reqwest::Client, broadcaster_user_id: u64, text: &str) -> Result<()> {
    // Kick's /public/v1/chat requires `broadcaster_user_id` (integer) when
    // `type=user` — the bearer identifies the sender, not the target room.
    let Some(token) = auth::kick::stored_access_token()? else {
        anyhow::bail!("not logged in to Kick");
    };
    let body = json!({
        "broadcaster_user_id": broadcaster_user_id,
        "type": "user",
        "content": text,
    });
    let resp = http
        .post(SEND_URL)
        .bearer_auth(&token)
        .json(&body)
        .send()
        .await
        .context("POST /public/v1/chat")?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        // Refresh and retry once.
        if let Ok(Some(new_token)) = auth::kick::refresh(http).await {
            let resp2 = http
                .post(SEND_URL)
                .bearer_auth(&new_token)
                .json(&body)
                .send()
                .await?;
            if !resp2.status().is_success() {
                anyhow::bail!(
                    "Kick send {}: {}",
                    resp2.status(),
                    resp2.text().await.unwrap_or_default()
                );
            }
            return Ok(());
        }
    }
    if !resp.status().is_success() {
        anyhow::bail!(
            "Kick send {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
    }
    Ok(())
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
    fn parses_top_level_user_id() {
        let data = serde_json::json!({
            "user_id": 1234,
            "chatroom": { "id": 98765 },
            "user": { "id": 9999 }
        });
        let ids = parse_channel_ids(&data).expect("parse");
        assert_eq!(ids.chatroom_id, 98765);
        assert_eq!(ids.broadcaster_user_id, 1234);
    }

    #[test]
    fn falls_back_to_user_id_under_user() {
        let data = serde_json::json!({
            "chatroom": { "id": 1 },
            "user": { "id": 42 }
        });
        let ids = parse_channel_ids(&data).expect("parse");
        assert_eq!(ids.broadcaster_user_id, 42);
    }

    #[test]
    fn parses_string_user_id() {
        let data = serde_json::json!({
            "user_id": "777",
            "chatroom": { "id": 2 }
        });
        let ids = parse_channel_ids(&data).expect("parse");
        assert_eq!(ids.broadcaster_user_id, 777);
    }

    #[test]
    fn errors_when_chatroom_missing() {
        let data = serde_json::json!({ "user_id": 1 });
        assert!(parse_channel_ids(&data).is_err());
    }

    #[test]
    fn errors_when_user_id_missing() {
        let data = serde_json::json!({ "chatroom": { "id": 2 } });
        assert!(parse_channel_ids(&data).is_err());
    }
}
