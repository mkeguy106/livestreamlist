# `smoke` binary

End-to-end IPC smoke-test harness. Lets you (or an agent) call any
`#[tauri::command]` handler with JSON args from the shell, getting a
JSON envelope back, without launching the desktop app.

## Build

```bash
cargo build --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke
```

The binary's `required-features = ["smoke"]` excludes it from default
`cargo build`.

## Use

Single-shot:
```bash
cargo run --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke -- \
    spellcheck_check '{"text":"helo","language":"en_US","channelEmotes":[]}'
# {"command":"spellcheck_check","ok":true,"value":[{"start":0,"end":4,"word":"helo"}],"duration_ms":12}
```

JSONL streaming:
```bash
cargo run --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke <<EOF
{"id":"a","cmd":"add_channel_from_input","args":{"input":"twitch.tv/shroud"}}
{"id":"b","cmd":"list_channels","args":{}}
EOF
```

List all dispatchable commands (with `[blocked]` tag for denylist entries):
```bash
cargo run --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke -- --list
```

## Output envelope

Each command returns a JSON object on stdout, one per line:
- Success: `{"command":"...", "ok":true, "value":..., "duration_ms":N}`
- Error: `{"command":"...", "ok":false, "error":"...", "kind":"...", "duration_ms":N}`

`kind` values:
- `input` — your CLI args weren't valid JSON
- `deserialize` — Tauri's serde layer rejected the args (wrong shape)
- `command` — command body returned `Err`
- `blocked` — command is in DENYLIST; use `--allow-side-effects` to dispatch
- `panic` — command body panicked (caught by catch_unwind)

In JSONL mode, the response also echoes back the optional `id` field.

## Safety

- **Config is isolated by default.** Each run uses a fresh tempdir for
  `XDG_CONFIG_HOME` + `XDG_DATA_HOME`. Your real `~/.config/livestreamlist/`
  is never touched. Pass `--use-real-config` to opt out.
- **Side-effecting commands are blocked by default.** `chat_send`,
  `embed_*`, `*_login`, `launch_stream`, etc. return `kind:"blocked"`
  unless you pass `--allow-side-effects` (which logs a stderr warning
  per dispatch).
- **Network calls are NOT mocked.** `refresh_all`, `twitch_anniversary_check`,
  `import_twitch_follows` hit the real APIs. Don't spam them.

## Out of scope

- No headless WebKit / GTK. Commands that construct `wry::WebView` (embed
  mounts, login popups) appear in the denylist and require the real app.
- No keyring isolation. Auth-token reads hit the real Secret Service /
  KWallet on your machine.
- No CI integration in v1. The binary is the foundation; piping a JSONL
  fixture through it in CI is a follow-up.

## Architecture / contributing

The full design is in `docs/superpowers/specs/2026-05-02-ipc-smoke-harness-design.md`.
The dispatch surface lives in `src-tauri/src/smoke.rs` (build_smoke_app,
DENYLIST) and `src-tauri/src/bin/smoke.rs` (CLI loop, envelope formatting).

Adding a new `#[tauri::command]`:
1. Add the function to `lib.rs` as usual.
2. Add the function name to `register_handlers!` in `lib.rs`.
3. Add the function name to `list_handlers()` in `src/bin/smoke.rs`.
4. If the command has real-world side effects, add it to `DENYLIST` in `smoke.rs`.

The drift test in `tests/smoke_binary.rs` will fail loudly if you forget step 3.
