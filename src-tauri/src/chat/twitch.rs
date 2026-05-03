use anyhow::{Context, Result};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use super::badges::classify_mod_twitch;
use super::emotes::{self, EmoteCache};
use super::irc::{self, IrcMessage};
use super::log_store::ChatLogWriter;
use super::models::{
    ChatBadge, ChatMessage, ChatModerationEvent, ChatRoomState, ChatRoomStateEvent, ChatStatus,
    ChatStatusEvent, ChatUser, EmoteRange, ReplyInfo, SystemEvent,
};
use super::OutboundMsg;
use crate::platforms::Platform;

const IRC_URL: &str = "wss://irc-ws.chat.twitch.tv";

static SELF_ECHO_SEQ: AtomicU64 = AtomicU64::new(0);

/// Process-startup nonce mixed into self-echo IDs so they can't collide
/// with `self-N` IDs persisted to the chat log by previous sessions.
/// Without this, after app restart the counter resets to 0; the first
/// echo gets `self-0` and is silently deduped on the React side against
/// a `self-0` loaded from the chat log file (replay_chat_history). Bug
/// surfaces as "first message after restart doesn't show in our chat
/// even though it sent successfully to Twitch."
static SELF_ECHO_PREFIX: OnceLock<String> = OnceLock::new();

fn self_echo_prefix() -> &'static str {
    SELF_ECHO_PREFIX.get_or_init(|| {
        // Unix-millis at first call. Stable for the process lifetime,
        // unique across processes (and almost certainly across restarts
        // even on hot-reload — devloop relaunch takes >1ms).
        format!("{:x}", chrono::Utc::now().timestamp_millis())
    })
}

/// Auth context passed into a chat connection. When `None` the task falls
/// back to an anonymous `justinfan*` read-only session (no sending).
#[derive(Debug, Clone)]
pub struct TwitchAuth {
    pub login: String,
    pub token: String,
}

pub struct TwitchChatConfig {
    pub app: AppHandle,
    pub http: reqwest::Client,
    pub channel_key: String,
    pub channel_login: String,
    pub emotes: Arc<EmoteCache>,
    pub badges: Arc<crate::chat::badges::BadgeCache>,
    pub users: Arc<crate::users::UserStore>,
    pub auth: Option<TwitchAuth>,
    pub outbound: mpsc::UnboundedReceiver<OutboundMsg>,
    /// Updated when ROOMSTATE arrives so build_privmsg / build_usernotice
    /// can scope their badge lookups. Interior-mutable since cfg is shared
    /// as `&TwitchChatConfig` through handle_line.
    pub room_id: parking_lot::Mutex<Option<String>>,
    /// Per-channel own-user badges captured from USERSTATE; used for
    /// local echo of own outgoing messages.
    pub own_badges: parking_lot::Mutex<Vec<crate::chat::models::ChatBadge>>,
    /// Properly-cased display name from GLOBALUSERSTATE/USERSTATE. The
    /// IRC NICK we send is forced lowercase, so without this the self-echo
    /// would render as `angeloftheodd` instead of `AngelOfTheOdd`.
    pub own_display_name: parking_lot::Mutex<Option<String>>,
    /// Latest parsed ROOMSTATE from IRC; used for emit-on-change logic.
    pub last_room_state: parking_lot::Mutex<Option<ChatRoomState>>,
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

    // Prefetch global Twitch badges in the background. Idempotent — the
    // cache skips the HTTP call if globals were already loaded by an earlier
    // connection in this process.
    {
        let cache = Arc::clone(&cfg.badges);
        let http = cfg.http.clone();
        tauri::async_runtime::spawn(async move {
            cache.ensure_twitch_global(&http).await;
        });
    }

    let mut log = ChatLogWriter::open(Platform::Twitch, &cfg.channel_login).ok();

    // Backfill ~50 recent messages from recent-messages.robotty.de so a
    // freshly opened channel isn't an empty box for the first 30
    // seconds. Mirrors the Qt app: serial before read_loop, 5 s
    // timeout, silent on failure. Each historical message flows
    // through the same persist_and_emit path with is_backfill=true so
    // the frontend can dim them.
    fetch_and_replay_recent_messages(cfg, log.as_mut()).await;

    read_loop(cfg, &mut ws, log.as_mut()).await
}

const RECENT_MESSAGES_URL: &str = "https://recent-messages.robotty.de/api/v2/recent-messages";
const RECENT_MESSAGES_LIMIT: u32 = 50;
const RECENT_MESSAGES_TIMEOUT_SECS: u64 = 5;

#[derive(serde::Deserialize)]
struct RecentMessagesBody {
    #[serde(default)]
    messages: Vec<String>,
}

async fn fetch_and_replay_recent_messages(
    cfg: &TwitchChatConfig,
    mut log: Option<&mut ChatLogWriter>,
) {
    let url = format!(
        "{RECENT_MESSAGES_URL}/{}?limit={RECENT_MESSAGES_LIMIT}&hideModeratedMessages=true",
        cfg.channel_login.to_ascii_lowercase()
    );
    let resp = match cfg
        .http
        .get(&url)
        .timeout(std::time::Duration::from_secs(RECENT_MESSAGES_TIMEOUT_SECS))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            log::debug!("recent-messages HTTP {} for {}", r.status(), cfg.channel_login);
            return;
        }
        Err(e) => {
            log::debug!("recent-messages fetch failed for {}: {e:#}", cfg.channel_login);
            return;
        }
    };
    let body: RecentMessagesBody = match resp.json().await {
        Ok(b) => b,
        Err(e) => {
            log::debug!("recent-messages decode failed for {}: {e:#}", cfg.channel_login);
            return;
        }
    };
    if body.messages.is_empty() {
        return;
    }
    for raw in body.messages {
        let Some(parsed) = irc::parse(&raw) else {
            continue;
        };
        if parsed.command != "PRIVMSG" {
            continue;
        }
        if let Some(mut msg) = build_privmsg(cfg, &parsed) {
            msg.is_backfill = true;
            persist_and_emit(cfg, log.as_deref_mut(), msg);
        }
    }
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
                    Ok(()) => {
                        // Local echo: IRC doesn't echo own PRIVMSG. Synthesize so
                        // the user sees their own message and badges.
                        if let Some(echo) = build_self_echo(cfg, &text) {
                            log::info!(
                                "self-echo emit channel={} id={}",
                                cfg.channel_key, echo.id
                            );
                            persist_and_emit(cfg, log.as_deref_mut(), echo);
                        } else {
                            log::warn!(
                                "self-echo skipped channel={} (auth missing? cfg.auth.is_some()={})",
                                cfg.channel_key,
                                cfg.auth.is_some()
                            );
                        }
                        Ok(())
                    }
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
        "ROOMSTATE" => {
            if let Some(rid) = extract_room_id(&msg) {
                let prev = cfg.room_id.lock().clone();
                if prev.as_deref() != Some(rid.as_str()) {
                    *cfg.room_id.lock() = Some(rid.clone());
                    let cache = Arc::clone(&cfg.badges);
                    let http = cfg.http.clone();
                    tauri::async_runtime::spawn(async move {
                        cache.ensure_twitch_channel(&http, &rid).await;
                    });
                }
            }
            let prior = cfg.last_room_state.lock().clone().unwrap_or_default();
            let next = apply_roomstate_tags(&msg.tags, prior);
            if cfg.last_room_state.lock().as_ref() != Some(&next) {
                *cfg.last_room_state.lock() = Some(next.clone());
                let _ = cfg.app.emit(
                    &format!("chat:roomstate:{}", cfg.channel_key),
                    ChatRoomStateEvent {
                        channel_key: cfg.channel_key.clone(),
                        state: next,
                    },
                );
            }
        }
        "USERSTATE" | "GLOBALUSERSTATE" => {
            let badges = extract_own_badges(&msg);
            *cfg.own_badges.lock() = badges;
            if let Some(name) = extract_own_display_name(&msg) {
                *cfg.own_display_name.lock() = Some(name);
            }
        }
        "NOTICE" => {
            // Surface lands in Phase 4b with preferences.
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
        .and_then(chrono::DateTime::from_timestamp_millis)
        .unwrap_or_else(Utc::now);

    let color = msg.tags.get("color").filter(|s| !s.is_empty()).cloned();

    let mut badges = parse_badges(
        msg.tags.get("badges").map(String::as_str).unwrap_or(""),
        msg.tags.get("badge-info").map(String::as_str).unwrap_or(""),
    );
    let room_snapshot = cfg.room_id.lock().clone();
    cfg.badges
        .resolve(Platform::Twitch, room_snapshot.as_deref(), &mut badges);

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
        link_ranges: Vec::new(),
        badges,
        is_action,
        is_first_message: msg.tags.get("first-msg").map(|v| v == "1").unwrap_or(false),
        reply_to,
        system: None,
        is_backfill: false,
        is_log_replay: false,
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

fn build_self_echo(cfg: &TwitchChatConfig, text: &str) -> Option<ChatMessage> {
    // Anonymous (justinfan…) connections shouldn't echo — they can't even
    // send. Require auth to be present.
    let auth = cfg.auth.as_ref()?;
    let login = auth.login.to_ascii_lowercase();
    if login.is_empty() {
        return None;
    }

    let mut badges = cfg.own_badges.lock().clone();
    let room_snapshot = cfg.room_id.lock().clone();
    cfg.badges
        .resolve(Platform::Twitch, room_snapshot.as_deref(), &mut badges);

    // Strip /me ACTION wrapping mirroring inbound behavior.
    let (clean_text, is_action) = strip_action(text);

    let display_name = cfg
        .own_display_name
        .lock()
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| login.clone());

    // Scan for emotes by name against the channel's cache (3rd-party
    // globals/channel + the user's Twitch sub/follower emotes loaded
    // via emote_loader). Without this, own messages render emote
    // codes as plain text — inbound messages get the same treatment
    // via build_privmsg's scan_message call.
    let initial: Vec<EmoteRange> = Vec::new();
    let mut emote_ranges = cfg
        .emotes
        .scan_message(&cfg.channel_key, &clean_text, &initial);
    emote_ranges.sort_by_key(|r| r.start);

    Some(ChatMessage {
        id: format!(
            "self-{}-{}",
            self_echo_prefix(),
            SELF_ECHO_SEQ.fetch_add(1, Ordering::Relaxed)
        ),
        channel_key: cfg.channel_key.clone(),
        platform: Platform::Twitch,
        timestamp: chrono::Utc::now(),
        user: ChatUser {
            id: None,
            login,
            display_name,
            color: None,
            is_mod: badges.iter().any(|b| b.id.starts_with("moderator/")),
            is_subscriber: badges.iter().any(|b| b.id.starts_with("subscriber/")),
            is_broadcaster: badges.iter().any(|b| b.id.starts_with("broadcaster/")),
            is_turbo: badges.iter().any(|b| b.id.starts_with("turbo/")),
        },
        text: clean_text,
        emote_ranges,
        link_ranges: Vec::new(),
        badges,
        is_action,
        is_first_message: false,
        reply_to: None,
        system: None,
        is_backfill: false,
        is_log_replay: false,
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
        .and_then(chrono::DateTime::from_timestamp_millis)
        .unwrap_or_else(Utc::now);

    let mut badges = parse_badges(
        msg.tags.get("badges").map(String::as_str).unwrap_or(""),
        msg.tags.get("badge-info").map(String::as_str).unwrap_or(""),
    );
    let room_snapshot = cfg.room_id.lock().clone();
    cfg.badges
        .resolve(Platform::Twitch, room_snapshot.as_deref(), &mut badges);

    // Own-resub detection: when the logged-in user shares their resub
    // anniversary, Twitch broadcasts a USERNOTICE with msg-id=resub
    // (or msg-id=sub for first-time subs). Emit a separate event so
    // the React useSubAnniversary hook can auto-dismiss the banner
    // without filtering every chat message itself.
    if matches!(kind.as_str(), "resub" | "sub") {
        if let Some(ref own) = cfg.auth {
            if own.login.eq_ignore_ascii_case(&login) {
                use tauri::Emitter;
                let months = msg
                    .tags
                    .get("msg-param-cumulative-months")
                    .and_then(|s| s.parse::<u32>().ok())
                    .or_else(|| {
                        msg.tags
                            .get("msg-param-months")
                            .and_then(|s| s.parse::<u32>().ok())
                    })
                    .unwrap_or(0);
                let payload = serde_json::json!({
                    "months": months,
                    "login": login,
                });
                let _ = cfg.app.emit(
                    &format!("chat:resub_self:{}", cfg.channel_key),
                    payload,
                );
            }
        }
    }

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
        link_ranges: Vec::new(),
        badges,
        is_action: false,
        is_first_message: false,
        reply_to: None,
        system: Some(SystemEvent {
            kind,
            text: system_text,
        }),
        is_backfill: false,
        is_log_replay: false,
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
    let done_e = false;
    for (char_idx, (byte_idx, _ch)) in text.char_indices().enumerate() {
        if !done_s && char_idx == char_start {
            bs = byte_idx;
            done_s = true;
        }
        if !done_e && char_idx == char_end {
            be = byte_idx;
            break;
        }
    }
    (bs, be)
}

fn extract_room_id(msg: &crate::chat::irc::IrcMessage<'_>) -> Option<String> {
    msg.tags.get("room-id").filter(|s| !s.is_empty()).cloned()
}

/// Apply Twitch ROOMSTATE tags onto a prior state. Tags absent from the map
/// preserve their prior value (Twitch sends partial ROOMSTATEs on flips).
pub fn apply_roomstate_tags(
    tags: &std::collections::HashMap<String, String>,
    mut prior: ChatRoomState,
) -> ChatRoomState {
    if let Some(v) = tags.get("slow").and_then(|s| s.parse::<u32>().ok()) {
        prior.slow_seconds = v;
    }
    if let Some(v) = tags.get("followers-only").and_then(|s| s.parse::<i32>().ok()) {
        prior.followers_only_minutes = v;
    }
    if let Some(v) = tags.get("subs-only") {
        prior.subs_only = v == "1";
    }
    if let Some(v) = tags.get("emote-only") {
        prior.emote_only = v == "1";
    }
    if let Some(v) = tags.get("r9k") {
        prior.r9k = v == "1";
    }
    prior
}

fn extract_own_badges(msg: &crate::chat::irc::IrcMessage<'_>) -> Vec<ChatBadge> {
    parse_badges(
        msg.tags.get("badges").map(String::as_str).unwrap_or(""),
        msg.tags.get("badge-info").map(String::as_str).unwrap_or(""),
    )
}

fn extract_own_display_name(msg: &crate::chat::irc::IrcMessage<'_>) -> Option<String> {
    msg.tags
        .get("display-name")
        .filter(|s| !s.is_empty())
        .cloned()
}

fn parse_badges(badges_tag: &str, badge_info_tag: &str) -> Vec<ChatBadge> {
    if badges_tag.is_empty() {
        return Vec::new();
    }
    let info = parse_badge_info(badge_info_tag);
    badges_tag
        .split(',')
        .filter_map(|pair| {
            let (set_name, version) = pair.split_once('/')?;
            let title = match set_name {
                "subscriber" | "founder" => info
                    .get("subscriber")
                    .or_else(|| info.get("founder"))
                    .copied()
                    .map(format_subscriber_title)
                    .unwrap_or_else(|| "Subscriber".to_string()),
                _ => set_name.to_string(),
            };
            Some(ChatBadge {
                id: format!("{set_name}/{version}"),
                url: String::new(),
                title,
                is_mod: classify_mod_twitch(set_name),
            })
        })
        .collect()
}

fn parse_badge_info(tag: &str) -> std::collections::HashMap<String, u32> {
    if tag.is_empty() {
        return std::collections::HashMap::new();
    }
    tag.split(',')
        .filter_map(|pair| {
            let (set_name, value) = pair.split_once('/')?;
            let months = value.parse::<u32>().ok()?;
            Some((set_name.to_string(), months))
        })
        .collect()
}

/// Format a subscriber tooltip in Twitch's tier-title style:
/// "{N}-Month Subscriber" for under-a-year and non-whole-year tenures,
/// "{N}-Year Subscriber" for whole years (12, 24, 36…).
fn format_subscriber_title(months: u32) -> String {
    if months == 0 {
        "Subscriber".to_string()
    } else if months >= 12 && months % 12 == 0 {
        format!("{}-Year Subscriber", months / 12)
    } else {
        format!("{months}-Month Subscriber")
    }
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
        let badges = parse_badges("broadcaster/1,subscriber/6,vip/1,turbo/1", "");
        let map: std::collections::HashMap<&str, bool> =
            badges.iter().map(|b| (b.id.as_str(), b.is_mod)).collect();
        assert_eq!(map.get("broadcaster/1").copied(), Some(true));
        assert_eq!(map.get("vip/1").copied(), Some(true));
        assert_eq!(map.get("subscriber/6").copied(), Some(false));
        assert_eq!(map.get("turbo/1").copied(), Some(false));
    }

    #[test]
    fn parse_badges_empty_returns_empty() {
        assert!(parse_badges("", "").is_empty());
    }

    #[test]
    fn parse_badges_subscriber_tenure_uses_badge_info_months() {
        // User holds the 12-month tier badge but has actually been subscribed
        // 15 months — Twitch sends both. We want the actual tenure on hover.
        let badges = parse_badges("subscriber/12", "subscriber/15");
        let sub = badges.iter().find(|b| b.id == "subscriber/12").unwrap();
        assert_eq!(sub.title, "15-Month Subscriber");
    }

    #[test]
    fn parse_badges_subscriber_tenure_clean_year_uses_year_label() {
        let badges = parse_badges("subscriber/24", "subscriber/24");
        let sub = badges.iter().find(|b| b.id == "subscriber/24").unwrap();
        assert_eq!(sub.title, "2-Year Subscriber");
    }

    #[test]
    fn parse_badges_subscriber_without_badge_info_falls_back() {
        // No badge-info means we don't know actual months; fall back to a
        // generic "Subscriber" label rather than the lowercase set name or
        // an empty string that would resolve to the badge id.
        let badges = parse_badges("subscriber/3", "");
        let sub = badges.iter().find(|b| b.id == "subscriber/3").unwrap();
        assert_eq!(sub.title, "Subscriber");
    }

    #[test]
    fn parse_badges_founder_uses_founder_badge_info_months() {
        // Founders have badges=founder/0 with badge-info=founder/N (or
        // sometimes subscriber/N). Both cases should produce a tenure label.
        let badges = parse_badges("founder/0", "founder/8");
        let f = badges.iter().find(|b| b.id == "founder/0").unwrap();
        assert_eq!(f.title, "8-Month Subscriber");
    }

    #[test]
    fn format_subscriber_title_handles_edge_cases() {
        assert_eq!(format_subscriber_title(0), "Subscriber");
        assert_eq!(format_subscriber_title(1), "1-Month Subscriber");
        assert_eq!(format_subscriber_title(11), "11-Month Subscriber");
        assert_eq!(format_subscriber_title(12), "1-Year Subscriber");
        assert_eq!(format_subscriber_title(15), "15-Month Subscriber");
        assert_eq!(format_subscriber_title(24), "2-Year Subscriber");
        assert_eq!(format_subscriber_title(36), "3-Year Subscriber");
        assert_eq!(format_subscriber_title(37), "37-Month Subscriber");
    }

    #[test]
    fn extract_room_id_from_roomstate() {
        let line = "@emote-only=0;followers-only=-1;r9k=0;room-id=12345;slow=0;subs-only=0 :tmi.twitch.tv ROOMSTATE #shroud";
        let m = crate::chat::irc::parse(line).unwrap();
        assert_eq!(extract_room_id(&m), Some("12345".to_string()));
    }

    #[test]
    fn extract_own_badges_from_userstate() {
        let line = "@badge-info=subscriber/12;badges=broadcaster/1,subscriber/12;color=#FF0000;display-name=Me;mod=0;subscriber=1;user-type= :tmi.twitch.tv USERSTATE #shroud";
        let m = crate::chat::irc::parse(line).unwrap();
        let badges = extract_own_badges(&m);
        let ids: Vec<&str> = badges.iter().map(|b| b.id.as_str()).collect();
        assert!(ids.contains(&"broadcaster/1"));
        assert!(ids.contains(&"subscriber/12"));
        assert!(
            badges
                .iter()
                .find(|b| b.id == "broadcaster/1")
                .unwrap()
                .is_mod
        );
        assert!(
            !badges
                .iter()
                .find(|b| b.id == "subscriber/12")
                .unwrap()
                .is_mod
        );
    }

    #[test]
    fn extract_own_display_name_picks_cased_name() {
        let line = "@badge-info=;badges=;color=#FF0000;display-name=AngelOfTheOdd;emote-sets=0;mod=0;subscriber=0;user-type= :tmi.twitch.tv GLOBALUSERSTATE";
        let m = crate::chat::irc::parse(line).unwrap();
        assert_eq!(
            extract_own_display_name(&m),
            Some("AngelOfTheOdd".to_string())
        );
    }

    #[test]
    fn extract_own_display_name_absent_returns_none() {
        let line = "@badges= :tmi.twitch.tv USERSTATE #shroud";
        let m = crate::chat::irc::parse(line).unwrap();
        assert_eq!(extract_own_display_name(&m), None);
    }

    #[test]
    fn extract_own_display_name_empty_returns_none() {
        let line = "@display-name=;badges= :tmi.twitch.tv USERSTATE #shroud";
        let m = crate::chat::irc::parse(line).unwrap();
        assert_eq!(extract_own_display_name(&m), None);
    }

    #[test]
    fn extract_room_id_absent_returns_none() {
        // Twitch sends ROOMSTATE without room-id on mode-change updates
        // (slow-mode toggle, etc.). The handler must short-circuit cleanly.
        let line = "@slow=30 :tmi.twitch.tv ROOMSTATE #shroud";
        let m = crate::chat::irc::parse(line).unwrap();
        assert_eq!(extract_room_id(&m), None);
    }

    #[test]
    fn parses_full_join_roomstate() {
        let mut tags = std::collections::HashMap::new();
        tags.insert("emote-only".to_string(), "0".to_string());
        tags.insert("followers-only".to_string(), "30".to_string());
        tags.insert("r9k".to_string(), "0".to_string());
        tags.insert("slow".to_string(), "10".to_string());
        tags.insert("subs-only".to_string(), "1".to_string());

        let s = apply_roomstate_tags(&tags, ChatRoomState::default());

        assert_eq!(s.slow_seconds, 10);
        assert_eq!(s.followers_only_minutes, 30);
        assert!(s.subs_only);
        assert!(!s.emote_only);
        assert!(!s.r9k);
    }

    #[test]
    fn partial_roomstate_merges_with_prior() {
        let prior = ChatRoomState {
            slow_seconds: 5,
            subs_only: true,
            followers_only_minutes: 60,
            ..ChatRoomState::default()
        };
        let mut tags = std::collections::HashMap::new();
        tags.insert("slow".to_string(), "30".to_string());

        let s = apply_roomstate_tags(&tags, prior);

        assert_eq!(s.slow_seconds, 30);
        assert!(s.subs_only); // preserved
        assert_eq!(s.followers_only_minutes, 60); // preserved
    }

    #[test]
    fn followers_only_negative_one_means_off() {
        let mut tags = std::collections::HashMap::new();
        tags.insert("followers-only".to_string(), "-1".to_string());
        let s = apply_roomstate_tags(&tags, ChatRoomState::default());
        assert_eq!(s.followers_only_minutes, -1);
    }
}
