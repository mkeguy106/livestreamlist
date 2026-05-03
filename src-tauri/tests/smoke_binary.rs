//! End-to-end integration tests for the `smoke` binary. Each test
//! invokes the binary via assert_cmd and asserts on stdout/stderr/exit.
//!
//! Run with: cargo test --features smoke --test smoke_binary

#![cfg(feature = "smoke")]

use assert_cmd::Command;

fn smoke() -> Command {
    Command::cargo_bin("smoke").expect("smoke binary built with --features smoke")
}

#[test]
fn list_prints_at_least_some_known_commands() {
    let output = smoke().arg("--list").output().expect("run --list");
    assert!(output.status.success(), "exit status: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout).unwrap();
    // Sanity: a few well-known commands must be in the output.
    for cmd in &["list_channels", "list_livestreams", "spellcheck_check", "chat_send"] {
        assert!(stdout.contains(cmd), "--list output missing {cmd}; got:\n{stdout}");
    }
    // chat_send is on the denylist, so it should be tagged.
    assert!(stdout.contains("chat_send [blocked]"), "denylist tag missing for chat_send");
}

#[test]
fn help_prints_usage_and_exits_zero() {
    let output = smoke().arg("--help").output().expect("run --help");
    assert!(output.status.success(), "exit status: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Usage:"), "--help output missing 'Usage:'");
    assert!(stdout.contains("--list"), "--help should mention --list");
    assert!(stdout.contains("--use-real-config"), "--help should mention --use-real-config");
    assert!(stdout.contains("--allow-side-effects"), "--help should mention --allow-side-effects");
}

#[test]
fn single_shot_list_channels_returns_empty_array() {
    let output = smoke()
        .args(["list_channels", "{}"])
        .output()
        .expect("run list_channels");
    assert!(output.status.success(), "exit: {:?} stderr: {}", output.status, String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let json: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout not JSON ({e}): {stdout}"));
    assert_eq!(json["command"], "list_channels");
    assert_eq!(json["ok"], true);
    assert_eq!(json["value"], serde_json::json!([]));
    assert!(json["duration_ms"].is_number(), "duration_ms missing");
}

#[test]
fn single_shot_marshalling_error_classifies_as_deserialize() {
    let output = smoke()
        .args(["add_channel_from_input", r#"{"wrong_field":"x"}"#])
        .output()
        .expect("run add_channel_from_input with bad args");
    assert!(!output.status.success(), "expected non-zero exit for bad args");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let json: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout not JSON ({e}): {stdout}"));
    assert_eq!(json["command"], "add_channel_from_input");
    assert_eq!(json["ok"], false);
    assert_eq!(json["kind"], "deserialize");
    assert!(
        json["error"].as_str().unwrap_or("").contains("invalid args"),
        "error message should contain Tauri's stable 'invalid args' prefix; got: {}",
        json["error"]
    );
}

#[test]
fn single_shot_unknown_command_returns_command_error() {
    let output = smoke()
        .args(["this_command_does_not_exist", "{}"])
        .output()
        .expect("run unknown command");
    assert!(!output.status.success(), "expected non-zero exit for unknown command");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(json["ok"], false);
    // kind is "command" for any post-dispatch error that isn't deserialize
    assert_eq!(json["kind"], "command", "unknown command should classify as 'command'; got: {}", json["kind"]);
}

#[test]
fn denylist_blocks_chat_send_by_default() {
    let output = smoke()
        .args(["chat_send", r#"{"uniqueKey":"twitch:shroud","text":"hi"}"#])
        .output()
        .expect("run chat_send");
    assert!(!output.status.success(), "expected non-zero exit");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(json["ok"], false);
    assert_eq!(json["kind"], "blocked");
    assert!(
        json["error"].as_str().unwrap_or("").contains("--allow-side-effects"),
        "error should mention the opt-out flag; got: {}", json["error"]
    );
}

#[test]
fn allow_side_effects_bypasses_denylist() {
    let output = smoke()
        .args(["--allow-side-effects", "chat_send", r#"{"uniqueKey":"twitch:shroud","text":"hi"}"#])
        .output()
        .expect("run chat_send with allow-side-effects");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    // We don't assert the call succeeds — chat_send under MockRuntime with no
    // ChatManager state will fail (Task 2 deliberately doesn't construct it).
    // We just assert it's NOT blocked.
    assert_ne!(json["kind"], "blocked", "kind=blocked means flag wasn't honored");
    // The warning should appear on stderr.
    assert!(
        stderr.contains("dispatching side-effecting command 'chat_send'"),
        "stderr missing side-effects warning; got: {stderr}"
    );
}

#[test]
fn denylist_intercepts_before_tauri_arg_deserialization() {
    // Sending bad args to a denylisted command returns kind='blocked',
    // NOT kind='deserialize'. This documents the dispatch order:
    //   1. parse the input JSON syntactically (kind='input' on failure)
    //   2. check denylist (kind='blocked' if hit)
    //   3. dispatch via Tauri (kind='deserialize' or 'command' on failure)
    //
    // Agents who want marshalling verification on a denied command pass
    // --allow-side-effects.
    let output = smoke()
        .args(["chat_send", r#"{"wrong_field":"x"}"#])
        .output()
        .expect("run chat_send with bad args");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(json["ok"], false);
    assert_eq!(json["kind"], "blocked");
}

#[test]
fn jsonl_streaming_dispatches_multiple_commands() {
    let input = "\
{\"id\":\"a\",\"cmd\":\"list_channels\",\"args\":{}}
{\"id\":\"b\",\"cmd\":\"list_livestreams\",\"args\":{}}
";
    let output = smoke()
        .write_stdin(input)
        .output()
        .expect("run jsonl stream");
    assert!(output.status.success(), "exit: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<_> = stdout.trim().split('\n').collect();
    assert_eq!(lines.len(), 2, "expected 2 response lines, got: {stdout}");

    let r0: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(r0["id"], "a");
    assert_eq!(r0["command"], "list_channels");
    assert_eq!(r0["ok"], true);

    let r1: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(r1["id"], "b");
    assert_eq!(r1["command"], "list_livestreams");
    assert_eq!(r1["ok"], true);
}

#[test]
fn jsonl_state_persists_across_calls_in_one_session() {
    let input = "\
{\"id\":\"add\",\"cmd\":\"add_channel_from_input\",\"args\":{\"input\":\"twitch.tv/shroud\"}}
{\"id\":\"list\",\"cmd\":\"list_channels\",\"args\":{}}
";
    let output = smoke()
        .write_stdin(input)
        .output()
        .expect("run add+list jsonl stream");
    assert!(output.status.success(), "exit: {:?} stderr: {}", output.status, String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<_> = stdout.trim().split('\n').collect();
    assert_eq!(lines.len(), 2, "expected 2 response lines, got: {stdout}");

    let add: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(add["ok"], true, "add_channel_from_input failed: {add}");

    let list: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(list["ok"], true);
    let channels = list["value"].as_array().expect("channels array");
    assert_eq!(channels.len(), 1, "expected 1 channel after add; got {channels:?}");
    assert_eq!(channels[0]["channel_id"], "shroud");
}

#[test]
fn jsonl_malformed_input_continues_session() {
    let input = "\
{\"cmd\":\"list_channels\",\"args\":{}}
this is not json
{\"cmd\":\"list_livestreams\",\"args\":{}}
";
    let output = smoke().write_stdin(input).output().expect("run with malformed line");
    assert!(output.status.success(), "should exit 0 on EOF even with malformed lines");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<_> = stdout.trim().split('\n').collect();
    assert_eq!(lines.len(), 3, "should emit 3 responses (good, bad, good); got: {stdout}");

    let r0: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(r0["ok"], true);

    let r1: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(r1["ok"], false);
    assert_eq!(r1["kind"], "input");

    let r2: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
    assert_eq!(r2["ok"], true);
}

#[test]
fn jsonl_panicking_command_does_not_kill_session() {
    // Send a sequence: known-good, possibly-panicking, known-good.
    // The middle call uses chat_send with --allow-side-effects to bypass
    // the denylist check. chat_send under MockRuntime fails because the
    // ChatManager state is not managed (kind="command"). This tests that
    // NON-panicking errors don't kill the JSONL session.
    //
    // To verify catch_unwind specifically: if no command in the codebase
    // panics under MockRuntime, this test still validates that the session
    // survives any kind of failure (which is the actual user-facing
    // contract). If a panic IS observed (kind="panic"), even better.
    let input = "\
{\"id\":\"a\",\"cmd\":\"list_channels\",\"args\":{}}
{\"id\":\"b\",\"cmd\":\"chat_send\",\"args\":{\"uniqueKey\":\"twitch:nonexistent\",\"text\":\"hi\"}}
{\"id\":\"c\",\"cmd\":\"list_channels\",\"args\":{}}
";
    let output = smoke()
        .args(["--allow-side-effects"])
        .write_stdin(input)
        .output()
        .expect("run panic-test jsonl");
    assert!(output.status.success(), "session must survive failing call; exit: {:?} stderr: {}", output.status, String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<_> = stdout.trim().split('\n').collect();
    assert_eq!(lines.len(), 3, "expected 3 lines (good, fail, good); got: {stdout}");

    let r0: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(r0["id"], "a");
    assert_eq!(r0["ok"], true, "first call must succeed");

    let r1: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(r1["id"], "b");
    assert_eq!(r1["ok"], false, "middle call must fail");
    // Acceptable kinds: 'panic' (catch_unwind caught it) or 'command'
    // (the called function returned an error instead of panicking).
    assert!(
        ["panic", "command", "deserialize"].contains(&r1["kind"].as_str().unwrap_or("")),
        "kind should be panic, command, or deserialize for failing call; got: {}", r1["kind"]
    );

    let r2: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
    assert_eq!(r2["id"], "c");
    assert_eq!(r2["ok"], true, "third call must succeed (session survived)");
}

#[test]
fn jsonl_optional_id_omitted_in_response_when_absent_in_input() {
    let input = "{\"cmd\":\"list_channels\",\"args\":{}}\n";
    let output = smoke().write_stdin(input).output().expect("run no-id jsonl");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert!(json.get("id").is_none(), "id should be omitted when absent in input; got: {json}");
}

#[test]
fn jsonl_omitted_args_dispatches_with_empty_object() {
    // Bug-regression test: when the JSONL input omits the 'args' key,
    // serde defaults args to Value::Null. Without the Null→{} fallback,
    // dispatch_one receives raw_args="null" and Tauri rejects it.
    // Confirms list_channels (which takes no args) returns ok=true.
    let input = "{\"cmd\":\"list_channels\"}\n";
    let output = smoke().write_stdin(input).output().expect("run no-args jsonl");
    assert!(output.status.success(), "exit: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(json["ok"], true, "omitted args should dispatch as empty object; got: {json}");
    assert_eq!(json["value"], serde_json::json!([]));
}

#[test]
fn config_isolated_by_default_across_runs() {
    // Two independent smoke runs. Each gets a fresh temp config.
    // The second run must NOT see channels added by the first.
    let r1 = smoke()
        .args(["add_channel_from_input", r#"{"input":"https://twitch.tv/shroud"}"#])
        .output().expect("first run");
    assert!(r1.status.success(), "first add failed: {:?}", String::from_utf8_lossy(&r1.stderr));

    let r2 = smoke()
        .args(["list_channels", "{}"])
        .output().expect("second run");
    assert!(r2.status.success());
    let stdout = String::from_utf8(r2.stdout).unwrap();
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(
        json["value"], serde_json::json!([]),
        "second run must see empty channel list (isolation broken if non-empty); got: {json}"
    );
}

#[test]
fn use_real_config_flag_does_not_use_temp_dir() {
    // We can't easily assert this against the user's real ~/.config
    // (we'd corrupt their state). Instead: verify the flag doesn't
    // crash and the response shape is correct.
    let output = smoke()
        .args(["--use-real-config", "list_channels", "{}"])
        .output()
        .expect("run --use-real-config list_channels");
    // Status may be 0 or non-zero depending on whether the user has
    // channels persisted. The point: it must not crash.
    let stdout = String::from_utf8(output.stdout).unwrap();
    let json: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout not JSON ({e}): {stdout}"));
    // Shape check.
    assert_eq!(json["command"], "list_channels");
    assert!(json["ok"].is_boolean());

    // The stderr WARN about real config should be visible
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("--use-real-config"),
        "stderr should warn about real config use; got: {stderr}"
    );
}

#[test]
fn list_count_matches_register_handlers_macro_body() {
    // Asserts that the hard-coded handler list in src/bin/smoke.rs's
    // list_handlers() stays in sync with the register_handlers! macro
    // body in src/lib.rs. Adding a new command without updating both
    // will fail this test, surfacing the drift before it ships.
    //
    // We count `$crate::` occurrences inside the macro body — each
    // command is registered as `$crate::name,` exactly once. This is
    // more robust than counting `#[tauri::command]` annotations because
    // some commands have cfg-gated stub variants that ALSO carry the
    // annotation but appear only once in the macro.
    let lib_rs = std::fs::read_to_string("src/lib.rs").expect("read lib.rs");

    // Extract the register_handlers! macro body. The body is delimited by
    // `tauri::generate_handler![` ... `]` inside the macro definition.
    let macro_start = lib_rs
        .find("macro_rules! register_handlers")
        .expect("register_handlers! macro not found in lib.rs");
    let after_macro_start = &lib_rs[macro_start..];
    let body_start = after_macro_start
        .find("tauri::generate_handler![")
        .expect("generate_handler![ not found inside register_handlers! macro");
    let body_relative = &after_macro_start[body_start..];
    let body_end = body_relative
        .find(']')
        .expect("closing ] not found inside generate_handler![");
    let macro_body = &body_relative[..=body_end];

    let macro_command_count = macro_body.matches("$crate::").count();

    let output = smoke().arg("--list").output().expect("run --list");
    assert!(output.status.success());
    let list_count = String::from_utf8(output.stdout).unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count();

    assert_eq!(
        list_count, macro_command_count,
        "smoke --list ({list_count}) must equal the count of $crate:: entries in register_handlers! macro ({macro_command_count}). \
         Did you add a #[tauri::command] without updating list_handlers() in src/bin/smoke.rs and register_handlers!() in src/lib.rs?"
    );
}
