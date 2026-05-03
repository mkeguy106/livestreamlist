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
