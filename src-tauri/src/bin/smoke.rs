//! IPC smoke-test harness binary. Built only with --features smoke.
//!
//! Single-shot:  smoke <cmd> '<json args>'
//! JSONL stream: smoke   (no positional; reads stdin)
//! Subcommands:  smoke --list | --help
//! Flags:        --use-real-config | --allow-side-effects
//!
//! See README.md in this directory for the full protocol.

use livestreamlist_lib::smoke::DENYLIST;

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

Logs (tracing) go to stderr; stdout is parseable JSONL.
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

    eprintln!("dispatch loop not yet implemented (Task 4)");
    std::process::exit(2);
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
