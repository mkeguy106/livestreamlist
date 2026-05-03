//! IPC smoke-test harness binary. Built only with --features smoke.
//!
//! Single-shot:  smoke <cmd> '<json args>'
//! JSONL stream: smoke   (no positional; reads stdin)
//! Subcommands:  smoke --list | --help
//! Flags:        --use-real-config | --allow-side-effects
//!
//! See README.md in this directory for the full protocol.

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

    // Strip flag-style args; keep positional (cmd + json).
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(|s| s.as_str())
        .collect();

    if positional.len() != 2 {
        eprintln!("expected exactly 2 positional args (cmd + json args); got {}", positional.len());
        eprintln!("for JSONL streaming mode (Task 6), pass no positionals.");
        eprintln!("run with --help for usage.");
        std::process::exit(2);
    }
    let cmd = positional[0];
    let raw_args = positional[1];

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

    let envelope = dispatch_one(&webview, cmd, raw_args);
    println!("{}", serde_json::to_string(&envelope).unwrap());
    // Keep temp alive until after the print to ensure XDG paths are valid for the dispatch.
    drop(temp);
    std::process::exit(if envelope["ok"].as_bool().unwrap_or(false) { 0 } else { 1 });
}

/// Run one command and return its envelope as a serde_json::Value.
fn dispatch_one(webview: &tauri::WebviewWindow<tauri::test::MockRuntime>, cmd: &str, raw_args: &str) -> Value {
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
    let result = get_ipc_response(webview, request);
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
