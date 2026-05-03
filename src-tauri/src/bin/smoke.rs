//! IPC smoke-test harness binary. Built only with --features smoke.
//!
//! Single-shot:  smoke <cmd> '<json args>'
//! JSONL stream: smoke   (no positional; reads stdin)
//! Subcommands:  smoke --list | --help
//! Flags:        --use-real-config | --allow-side-effects
//!
//! See README.md in this directory for the full protocol.

use std::io::{BufRead, Write};
use std::time::Instant;

use livestreamlist_lib::smoke::{build_smoke_app, DENYLIST};
use serde_json::{json, Value};
use tauri::ipc::{CallbackFn, InvokeBody, InvokeResponseBody};
use tauri::test::{get_ipc_response, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::WebviewWindowBuilder;

const HELP: &str = "\
Usage:
    smoke [FLAGS] <command> '<json-args>'   # single-shot
    smoke [FLAGS]                            # JSONL streaming on stdin
    smoke --list                             # list all dispatchable commands
    smoke --help                             # this help

Flags:
    --use-real-config       Use ~/.config/livestreamlist/ instead of an isolated
                            temp dir. Off by default to protect real state.
    --allow-side-effects    Dispatch side-effecting commands (chat_send, embed_*,
                            *_login, etc.) instead of returning the blocked envelope.

Output: one JSON object per command, line-delimited.
    Success: {\"command\":\"...\",\"ok\":true,\"value\":...,\"duration_ms\":N}
    Error:   {\"command\":\"...\",\"ok\":false,\"error\":\"...\",\"kind\":\"...\",\"duration_ms\":N}

JSONL input shape: {\"id\":\"...\",\"cmd\":\"...\",\"args\":{...}}
JSONL output shape: same as Output above + \"id\" field if input had one.

Stdout is parseable JSONL. Tracing/log output is unrouted in this build;
Task 10 will install a stderr subscriber. RUST_LOG has no effect yet.
Exit code is 0 in JSONL mode (on EOF) and matches the 'ok' field in single-shot mode.
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{HELP}");
        return;
    }

    if args.iter().any(|a| a == "--list") {
        let mut names = list_handlers();
        names.sort();
        for name in &names {
            if DENYLIST.contains(&name.as_str()) {
                println!("{name} [blocked]");
            } else {
                println!("{name}");
            }
        }
        return;
    }

    let allow_side_effects = args.iter().any(|a| a == "--allow-side-effects");

    // Strip flag-style args; keep positional (cmd + json).
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(|s| s.as_str())
        .collect();

    if positional.len() != 0 && positional.len() != 2 {
        eprintln!("expected 0 (JSONL stream) or 2 (single-shot) positional args; got {}", positional.len());
        eprintln!("run with --help for usage.");
        std::process::exit(2);
    }

    let temp = match tempfile::tempdir() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("failed to create temp dir: {e}");
            std::process::exit(2);
        }
    };
    let app = match build_smoke_app(temp.path()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("build_smoke_app failed: {e:#}");
            std::process::exit(2);
        }
    };
    let webview = match WebviewWindowBuilder::new(&app, "main", Default::default()).build() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("webview build failed: {e}");
            std::process::exit(2);
        }
    };

    if positional.is_empty() {
        run_jsonl_loop(&webview, allow_side_effects);
        drop(temp);
        return;
    }

    let envelope = dispatch_one(&webview, positional[0], positional[1], allow_side_effects);
    println!("{}", serde_json::to_string(&envelope).unwrap());
    // Keep temp alive until after the print to ensure XDG paths are valid for the dispatch.
    drop(temp);
    std::process::exit(if envelope["ok"].as_bool().unwrap_or(false) { 0 } else { 1 });
}

/// Run one command and return its envelope as a serde_json::Value.
fn dispatch_one(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    cmd: &str,
    raw_args: &str,
    allow_side_effects: bool,
) -> Value {
    let started = Instant::now();

    // Step 1: parse the args. A JSON parse failure here is a CLI input
    // error, not a Tauri marshalling error — distinguish them via 'kind'.
    let args: Value = match serde_json::from_str(raw_args) {
        Ok(v) => v,
        Err(e) => {
            return json!({
                "command": cmd,
                "ok": false,
                "error": format!("input json parse error: {e}"),
                "kind": "input",
                "duration_ms": started.elapsed().as_millis() as u64,
            });
        }
    };

    // Step 1.5: denylist check.
    if !allow_side_effects && DENYLIST.contains(&cmd) {
        return json!({
            "command": cmd,
            "ok": false,
            "error": format!(
                "command '{cmd}' is blocked in smoke harness; use --allow-side-effects to dispatch"
            ),
            "kind": "blocked",
            "duration_ms": started.elapsed().as_millis() as u64,
        });
    }
    if allow_side_effects && DENYLIST.contains(&cmd) {
        eprintln!("WARN: dispatching side-effecting command '{cmd}' under --allow-side-effects");
    }

    // Step 2: dispatch via Tauri's mock IPC.
    let request = InvokeRequest {
        cmd: cmd.into(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: "http://tauri.localhost".parse().unwrap(),
        body: InvokeBody::Json(args),
        headers: Default::default(),
        invoke_key: INVOKE_KEY.to_string(),
    };
    // Wrap in catch_unwind so a synchronous panic in the command body
    // does not kill the JSONL streaming session — the panic is caught and
    // converted to a kind="panic" envelope instead.
    //
    // NOTE: async panics inside Tauri-spawned futures may slip through
    // because Tokio absorbs them into JoinError before they can propagate
    // back to this call site. That is a known limitation; only synchronous
    // panics during get_ipc_response itself are caught here.
    let result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        get_ipc_response(webview, request)
    })) {
        Ok(r) => r,
        Err(panic) => {
            let msg = panic_message(&panic);
            return json!({
                "command": cmd,
                "ok": false,
                "error": format!("command panicked: {msg}"),
                "kind": "panic",
                "duration_ms": started.elapsed().as_millis() as u64,
            });
        }
    };
    let duration_ms = started.elapsed().as_millis() as u64;

    match result {
        Ok(InvokeResponseBody::Json(s)) => {
            let value: Value = serde_json::from_str(&s).unwrap_or(Value::Null);
            json!({
                "command": cmd,
                "ok": true,
                "value": value,
                "duration_ms": duration_ms,
            })
        }
        Ok(InvokeResponseBody::Raw(_)) => json!({
            "command": cmd,
            "ok": false,
            "error": "command returned binary response, not supported in smoke harness",
            "kind": "binary",
            "duration_ms": duration_ms,
        }),
        Err(err_value) => {
            let msg = err_value.as_str()
                .map(String::from)
                .unwrap_or_else(|| err_value.to_string());
            // Tauri's argument-deserialization errors are stably formatted as
            // "invalid args `<arg>` for command `<cmd>`: <serde_err>". See
            // tauri::Error::InvalidArgs in Tauri 2.10.3 src/error.rs.
            let kind = if msg.starts_with("invalid args ") {
                "deserialize"
            } else {
                "command"
            };
            json!({
                "command": cmd,
                "ok": false,
                "error": msg,
                "kind": kind,
                "duration_ms": duration_ms,
            })
        }
    }
}

/// Read JSONL from stdin; for each line, parse {id?, cmd, args} and emit
/// a JSONL response. Continue on malformed lines (emit kind='input'). Exit
/// 0 on EOF.
fn run_jsonl_loop(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    allow_side_effects: bool,
) {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) if l.trim().is_empty() => continue,
            Ok(l) => l,
            Err(e) => {
                let env = json!({
                    "command": Value::Null,
                    "ok": false,
                    "error": format!("stdin read error: {e}"),
                    "kind": "input",
                });
                writeln!(out, "{}", serde_json::to_string(&env).unwrap()).ok();
                continue;
            }
        };
        let parsed: Result<JsonlInput, _> = serde_json::from_str(&line);
        let envelope = match parsed {
            Err(e) => {
                json!({
                    "command": Value::Null,
                    "ok": false,
                    "error": format!("malformed input line ({e}): {line}"),
                    "kind": "input",
                })
            }
            Ok(JsonlInput { id, cmd, args }) => {
                let raw_args = match &args {
                    Value::Null => "{}".to_string(),
                    other => serde_json::to_string(other).unwrap_or_else(|_| "{}".to_string()),
                };
                let mut env = dispatch_one(webview, &cmd, &raw_args, allow_side_effects);
                if let Some(id) = id {
                    env["id"] = Value::String(id);
                }
                env
            }
        };
        writeln!(out, "{}", serde_json::to_string(&envelope).unwrap()).ok();
    }
}

#[derive(serde::Deserialize)]
struct JsonlInput {
    id: Option<String>,
    cmd: String,
    #[serde(default)]
    args: Value,
}

/// Extract a human-readable message from a panic payload.
/// Handles the two common payload types (`String` and `&str`); falls back
/// to a generic label for anything else (e.g. a boxed custom panic type).
fn panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = panic.downcast_ref::<&str>() {
        (*s).to_string()
    } else {
        "<non-string panic payload>".to_string()
    }
}

/// Source the handler-name list from the `register_handlers!` macro.
/// Today this is a hard-coded mirror of the macro's body — verified by the
/// drift-prevention test in tests/smoke_binary.rs (Task 9). If Tauri ever
/// exposes a public API to enumerate registered command names, switch this
/// to runtime introspection.
fn list_handlers() -> Vec<String> {
    [
        "list_livestreams", "list_channels", "add_channel_from_input",
        "clipboard_channel_url", "remove_channel", "set_favorite",
        "refresh_all", "launch_stream", "stop_stream", "list_playing",
        "open_in_browser", "open_url", "list_socials",
        "chat_connect", "chat_disconnect", "chat_send", "chat_open_in_browser",
        "chat_detach", "chat_reattach", "chat_focus_detached",
        "embed_mount", "embed_bounds", "embed_unmount", "embed_set_visible",
        "login_popup_open", "login_popup_close", "login_popup_resize",
        "list_emotes", "replay_chat_history",
        "get_settings", "update_settings",
        "get_user_metadata", "set_user_metadata", "get_user_profile",
        "get_user_messages", "list_blocked_users",
        "auth_status",
        "twitch_login", "twitch_logout", "twitch_web_login", "twitch_web_clear",
        "twitch_anniversary_check", "twitch_anniversary_dismiss",
        "twitch_share_resub_open", "twitch_share_window_close",
        "kick_login", "kick_logout",
        "youtube_login", "youtube_login_paste", "youtube_logout", "youtube_detect_browsers",
        "chaturbate_login", "chaturbate_logout",
        "import_twitch_follows",
        "spellcheck_check", "spellcheck_suggest", "spellcheck_add_word", "spellcheck_list_dicts",
    ].iter().map(|s| s.to_string()).collect()
}
