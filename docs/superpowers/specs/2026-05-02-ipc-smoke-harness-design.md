# IPC Smoke-Test Harness — Design

**Date:** 2026-05-02
**Status:** approved (brainstorming complete)
**Goal:** ship the smaller of the two automated-testing items in `docs/ROADMAP.md` ("Headless IPC smoke-test harness") so that agent-driven PR pipelines can verify any of the 62 `#[tauri::command]` handlers end-to-end through the real Tauri IPC layer without launching the desktop app.

## Background

`docs/ROADMAP.md` line 318 captures the friction:

> Cargo unit tests construct the underlying types directly and bypass `tauri::generate_handler!`, so they don't catch IPC marshalling bugs (wrong arg names in `invoke()` payloads, missing entries in `generate_handler![...]`, serde-skip mismatches between Rust struct and the JS-side shape). Today the workaround is "ask the human to launch the app, paste invocations into devtools, paste results back" — clean for a one-off but a tax on every agent-driven PR.

The roadmap entry's listed driver case ("PR series implementing spellcheck (PRs #92-#96 once landed)") has shipped (#92, #94, #96, #98, #100). The same friction now applies to every IPC-touching PR — the sub-anniversary series (#104–#109) hit it again, and the next IPC change will hit it too.

Tauri 2.10.3 (the version in our `Cargo.toml`) exposes a stable mock-runtime surface in `tauri::test`:

- `tauri::test::mock_builder() -> Builder<MockRuntime>` — an empty builder we register the production handler list against.
- `tauri::test::mock_app() -> App<MockRuntime>` — a complete app on the mock runtime with no real WebKit / GTK behind it.
- `tauri::test::get_ipc_response(&webview, InvokeRequest{cmd, body, …})` — dispatches a command by name through the real serde-based marshalling path.

This is the load-bearing primitive. Without it, the design would need a much larger custom IPC dispatcher; with it, the smoke binary is a thin shell around production code.

### Two automated-testing items in the roadmap

This spec is **only** the IPC smoke harness. The sibling roadmap item — "Headless WebKit test harness for webview-lifecycle code" (`ROADMAP.md:316`) — is strictly disjoint: that one needs Xvfb + WebKitGTK in a container to test the auth flows, embed lifecycle, and JS injection paths, and is real DevOps work. We deliberately ship the IPC harness first because it has a smaller surface area and unblocks more agent-driven work per unit of effort.

## Goals

- A single CLI binary (`cargo run --bin smoke -- …`) that can dispatch any of the 62 registered commands by name with JSON args, and prints the result envelope as JSON on stdout.
- Two invocation modes: single-shot (one positional arg + JSON args, exits with status code) and JSONL streaming (no positional, reads JSONL on stdin, emits JSONL on stdout, one AppState for the session).
- Real `AppState` construction so functional commands return real responses; not just marshalling-layer assertions.
- Hermetic by default — each smoke run uses an isolated temp config dir so an errant `add_channel_from_input` or `update_settings` can't mutate the user's real data.
- Side-effect denylist for commands that talk to the real network (chat WebSockets), spawn real subprocesses (streamlink/mpv), or open browser windows. Marshalling still verified for those commands; the body just doesn't run unless the agent passes `--allow-side-effects`.
- Drop-in for current "ask the human to launch the app, paste devtools output back" workflow. Agent runs one command, gets one JSON response.

## Non-goals (explicit)

- **No headless WebKit / GTK.** Commands that construct `wry::WebView::build_gtk` (embed mounting, login popups, in-app share popout) cannot be exercised — they require a real WebKit2GTK surface. They appear in the denylist with the explicit "use the real app" guidance.
- **No CI integration in v1.** The roadmap entry's stated motivator is agent debugging, not CI gates. Once the binary exists, a future PR can add a CI workflow step that pipes a JSONL fixture through the binary and greps for `"ok":false` lines. Cheap to add later; doesn't gate this work.
- **No HTTP mocking.** Network commands (`refresh_all`, `twitch_anniversary_check`, `import_twitch_follows`) hit the real APIs. Documented in `--help`. The value of seeing real GQL responses outweighs the per-platform mock plumbing cost; revisit if we get burned by GQL flake.
- **No keyring isolation.** The keyring crate touches the real Secret Service / KWallet on the host, not the temp dir. Calls that depend on captured tokens will return "not authenticated" unless the user has a real token. Out-of-scope follow-up; intersects the existing roadmap item "Keyring graceful fallback" (`ROADMAP.md:448`).
- **No production / `livestreamlist` binary changes.** The smoke binary is a peer of the production binary, sharing only the library crate. A "hidden `--smoke` flag on the main binary" was considered (it's option (a) in the roadmap entry's options list) and rejected — the main binary needs WebKit/GTK to start; bolting smoke onto it would either require duplicating setup logic or starting GTK we don't need. A standalone bin keeps both surfaces clean.

## Architecture

A new binary target `smoke` lives at `src-tauri/src/bin/smoke.rs` (alongside the existing `livestreamlist` binary). Cargo automatically discovers `src/bin/*.rs` as binary targets — no `[[bin]]` entry needed in `Cargo.toml` beyond the existing `livestreamlist` one. Both binaries link against the same `livestreamlist_lib` library crate that already exists.

A new module `livestreamlist_lib::smoke` exposes:

```rust
pub fn build_smoke_app(temp_root: &Path) -> Result<App<MockRuntime>, anyhow::Error>;
```

What it does, in order:

1. **Sets `XDG_CONFIG_HOME` and `XDG_DATA_HOME`** to subdirectories of `temp_root` so `config::config_dir()` and `config::data_dir()` (the existing helpers) resolve under the temp root. This must happen before any code reads those paths. Enforced structurally: `build_smoke_app` is the first thing called in `main`, before any state-touching code; the env-var setup is the first thing inside `build_smoke_app`. If `config::config_dir()` ever migrates from "read env on each call" to "OnceLock-cached", this contract breaks silently — call out as an audit item if that refactor lands.
2. **Constructs `AppState`** using the same code path as production setup, lifted out of `lib.rs::run()` setup-closure into a new function `livestreamlist_lib::build_app_state() -> AppState`. Production setup also gets refactored to call `build_app_state()` so there's exactly one construction path.
3. **Builds `mock_builder()`** with the same `invoke_handler!` block. The block is extracted to a public macro `livestreamlist_lib::register_handlers!()` so the smoke binary doesn't drift from production — adding a new command means editing one place.
4. **Manages** the constructed `AppState`, `Arc<PlayerManager>`, `Arc<SpellChecker>`.
5. **Skips** all side-effecting setup: tray (`tray::build`), window state plugin (`window_state::register`), async background tasks (the deferred raise-to-front, the launch-time browser cookie scrape), GTK overlay (`embed::install_overlay`).
6. **Calls** `.build(mock_context(noop_assets()))` to produce the `App<MockRuntime>`.
7. **Creates one mock `WebviewWindow`** named `"main"` (some commands take `tauri::WebviewWindow` as a parameter — they need *something* to receive).

The `App<MockRuntime>` is held by the smoke binary for the process lifetime.

### In-flight cleanup: extracting `build_app_state`

`lib.rs::run()`'s setup closure has grown to ~150 lines mixing pure `AppState` construction with side-effecting work (tray registration, async task spawns, plugin setup). Extracting `build_app_state()` is the kind of focused improvement the design naturally calls for — it's needed to avoid duplicating ~80 lines of construction logic in the smoke module, and it leaves the production setup closure smaller and more focused. Out-of-scope: any further restructuring of `lib.rs` beyond this extraction.

## Isolation + safety

### Config isolation

Set up before `build_smoke_app` runs:

- A temp dir is created via `tempfile::TempDir` (new dev-dependency in `[dev-dependencies]`; the `tempfile` crate is the standard Rust choice, small and dependency-light).
- `XDG_CONFIG_HOME` and `XDG_DATA_HOME` are set to subdirectories of that temp dir (using `std::env::set_var` before `AppState` construction reads them).
- `--use-real-config` flag skips the temp-dir setup; reads/writes hit `~/.config/livestreamlist/` like the production app. Used sparingly, e.g. when an agent wants `list_channels` to return their actual list for inspection.
- The `TempDir` handle lives for the lifetime of the smoke process; on exit it deletes the directory automatically.

### Side-effect denylist

A `&[&str]` constant in the smoke binary listing commands that produce real-world side effects beyond the temp dir:

```
chat_connect, chat_disconnect, chat_send,
launch_stream, stop_stream,
embed_mount, embed_bounds, embed_unmount, embed_set_visible,
twitch_login, twitch_web_login, kick_login, youtube_login,
youtube_login_paste, chaturbate_login,
login_popup_open, login_popup_close, login_popup_resize,
chat_open_in_browser, chat_detach, chat_reattach, chat_focus_detached,
twitch_share_resub_open, twitch_share_window_close,
open_in_browser, open_url,
```

The dispatch order is:

1. **Parse JSON args** → fail with `"kind":"deserialize"` error if shape is wrong (this is what catches marshalling bugs).
2. **Check denylist** → if hit, return `{"ok":false,"error":"command 'X' is blocked in smoke harness; use --allow-side-effects to dispatch","kind":"blocked"}`.
3. **Call `get_ipc_response`** → dispatch through Tauri's real IPC machinery.

So marshalling is verified for **every** command including denied ones — the agent sees the right error class for the right kind of failure. A denylist hit means the args were valid; a deserialize error means the args were wrong; a successful response means the command actually ran.

`--allow-side-effects` opt-out lets the agent fire one anyway. Each side-effecting command logs a clear stderr warning when dispatched in this mode (`WARN: dispatching side-effecting command 'chat_send' under --allow-side-effects`).

### Panic safety

Each invoke is wrapped in `std::panic::catch_unwind`; a panic returns `{"ok":false,"error":"command panicked: <msg>","kind":"panic"}` instead of crashing the smoke process. This is critical for JSONL streaming mode where one bad call would otherwise terminate a session of dozens.

## I/O protocol

### Single-shot mode

```
$ cargo run --bin smoke -- spellcheck_check '{"text":"helo","language":"en_US","channelEmotes":[]}'
```

Stdout (one JSON object on one line, newline-terminated):
```json
{"command":"spellcheck_check","ok":true,"value":[{"start":0,"end":4,"word":"helo"}],"duration_ms":12}
```

Errors:
```json
{"command":"chat_send","ok":false,"error":"command 'chat_send' is blocked in smoke harness; use --allow-side-effects to dispatch","kind":"blocked","duration_ms":0}
```

Marshalling error (the high-value case):
```json
{"command":"add_channel_from_input","ok":false,"error":"missing field `input` at line 1 column 2","kind":"deserialize","duration_ms":1}
```

Exit code: `0` on `ok:true`, `1` otherwise. Lets shell scripts do `if cargo run --bin smoke -- … ; then …`.

### JSONL streaming mode

`cargo run --bin smoke` with no positional args:

```
$ cargo run --bin smoke
{"id":"a","cmd":"add_channel_from_input","args":{"input":"twitch.tv/shroud"}}
{"id":"b","cmd":"list_channels","args":{}}
^D
{"id":"a","command":"add_channel_from_input","ok":true,"value":{"platform":"twitch","channel_id":"shroud",...},"duration_ms":3}
{"id":"b","command":"list_channels","ok":true,"value":[{"platform":"twitch",...}],"duration_ms":1}
```

- Each input line is one JSON object: `{id?, cmd, args}`. `id` is optional; if absent, the response `id` is omitted.
- Each output line is one JSON object: same envelope as single-shot + the `id` field if it was provided.
- Responses are emitted in dispatch order (sequential — no concurrency in the smoke loop). State persists across calls (e.g. add then list reflects the add).
- Malformed input lines: emit `{"ok":false,"error":"malformed input: …","kind":"input"}` and continue. Process exits 0 on EOF.

### Auxiliary subcommands

- `cargo run --bin smoke -- --list` → prints every registered command, one per line, with a `[blocked]` suffix for denylist entries.
- `cargo run --bin smoke -- --help` → usage + flag reference.

### Logging discipline

The production code uses `tracing`. The smoke binary installs a subscriber that writes structured logs to **stderr only**, so stdout stays clean JSONL that downstream tools (jq, agents parsing transcripts) can rely on. `RUST_LOG=debug cargo run --bin smoke …` works as expected; the default level is `warn` to keep stderr quiet during normal use.

## Drift prevention

- The `register_handlers!` macro means there's exactly one place where the handler list lives. Production and smoke read it the same way. If someone adds a `#[tauri::command]` and only edits the macro's body, both surfaces pick it up automatically.
- The `--list` command is implemented by introspecting the registered handler set at runtime — preferred mechanism is reading from `App<MockRuntime>::config().tauri.runtime_authority` or the equivalent registered-commands surface in Tauri 2.10. If implementation discovers that surface isn't public or stable, fall back to a `const SMOKE_COMMAND_NAMES: &[&str]` list generated by a `build.rs` that greps `lib.rs` for `#[tauri::command]\nfn (\w+)` patterns. The runtime-introspection path is preferred because it keeps `--list` automatically in sync with `register_handlers!`; the build.rs fallback is acceptable as a contingency only.
- A test in `tests/smoke_binary.rs` asserts that `--list`'s output line count equals the count of `#[tauri::command]` annotations parsed from `lib.rs` source — catches the "added a command but forgot to add it to the handler block" drift, regardless of which `--list` implementation is used.

## Testing strategy

A `tests/smoke_binary.rs` integration test (Cargo's standard `tests/` dir, separate from the existing 215 unit tests) exercises the binary end-to-end via `assert_cmd`:

- `--list` produces a non-empty output containing every name from `generate_handler!` (the drift-prevention assertion above).
- Single-shot mode: `smoke list_channels '{}'` exits 0 with an empty array on a fresh isolated config.
- Single-shot marshalling failure: `smoke add_channel_from_input '{"wrong_field":"x"}'` exits 1 with `"kind":"deserialize"`.
- Side-effect denylist: `smoke chat_send '{...}'` exits 1 with the blocked-error envelope.
- `--allow-side-effects` opt-out: a denied command no longer returns the blocked envelope (a stderr warning is logged; the actual call may fail for other reasons but the blocked path is bypassed).
- JSONL streaming: a 3-line input produces 3 sequential responses with id round-trip.
- State persistence in JSONL: `add_channel_from_input` followed by `list_channels` shows the added channel in the list response.
- Panic safety: a synthetic command that panics returns the panic envelope and the process keeps running in JSONL mode.

These tests pull in `assert_cmd` + `predicates` as new `[dev-dependencies]`. Total test count: ~8–10. Runs as part of `cargo test`.

## Documentation

- A short `src-tauri/src/bin/README.md` (~30 lines) — what the binary is for, single-shot vs JSONL examples, the safety story (isolation by default, denylist), and a paragraph on what it's *not* (no headless WebKit, no auth bootstrap, no real-time CI gate).
- `CLAUDE.md` gets a one-paragraph entry under "Development Commands" pointing at the binary, so future agents discover it immediately.
- `docs/ROADMAP.md` flips `- [ ] Headless IPC smoke-test harness` to `- [x] … (PR #N)` when this ships.

## Out of scope, parked as follow-ups

1. **Keyring-backed auth state** in the isolated config. Depends on the existing roadmap entry "Keyring graceful fallback" (`ROADMAP.md:448`).
2. **CI integration.** Once the binary exists, a future PR can add a CI workflow step that pipes `tests/fixtures/smoke.jsonl` through the binary and greps for any `"ok":false` line.
3. **HTTP mocking** for deterministic network commands. Same shape — value/cost ratio doesn't justify the per-platform plumbing yet; revisit if GQL flake bites.
4. **Headless WebKit test harness** for webview-lifecycle code. Strictly disjoint from this spec; that's the sibling roadmap item (`ROADMAP.md:316`).

## Risks

| Risk | Mitigation |
|---|---|
| `tauri::test::MockRuntime` semantics drift between Tauri minor versions | The integration test suite (`tests/smoke_binary.rs`) covers the dispatch path end-to-end. A breaking change in MockRuntime fails the test suite, not the production app. Pinned to 2.10.3 in `Cargo.toml`. |
| Commands that read `OnceLock`-cached config paths after the env vars are set might still see the production paths | `build_smoke_app` is the first thing called in `main`; the env-var setup is the first thing inside `build_smoke_app`. The current `config::config_dir()` reads env-vars on each call (no caching); if a future refactor adds `OnceLock` caching there, the smoke binary's contract breaks silently. Documented as an audit item next to `register_handlers!`. |
| The denylist drifts as new IPC-side-effecting commands are added without authors knowing about the smoke binary | The `--list` test catches added commands; reviewers explicitly asked to consider whether new commands belong on the denylist. Nothing fully prevents drift, but the surface is small. |
| Agents accidentally pass `--allow-side-effects` and fire real chat messages or launch real `mpv` processes | The flag name is intentionally awkward; stderr warnings are loud; the denylist exists *because* this is dangerous. We're not trying to make `--allow-side-effects` safe — we're making it visible. |
| `tempfile::TempDir`'s drop guard fails on process abort, leaving temp dirs behind | Acceptable. Temp dirs are under `/tmp` and get cleaned up by the OS / tmpfiles.d on reboot. |

## Acceptance criteria

All commands shown below assume `--manifest-path src-tauri/Cargo.toml` (the project's convention per `CLAUDE.md`'s Development Commands section) or running from inside `src-tauri/`.

- `cargo build --bin smoke` succeeds.
- `cargo run --bin smoke -- --list` prints all 62 commands with the right `[blocked]` flags.
- `cargo run --bin smoke -- spellcheck_check '{"text":"helo","language":"en_US","channelEmotes":[]}'` returns the expected misspelling array.
- `cargo run --bin smoke -- add_channel_from_input '{"input":"twitch.tv/shroud"}'` followed by `cargo run --bin smoke -- list_channels '{}'` returns an empty list (config isolation: each run is fresh).
- The same two commands piped through one JSONL session show the channel persisted across the second call.
- `cargo run --bin smoke -- chat_send '{"uniqueKey":"twitch:shroud","text":"hi"}'` exits 1 with `"kind":"blocked"`.
- `cargo run --bin smoke -- add_channel_from_input '{"wrong_field":"x"}'` exits 1 with `"kind":"deserialize"`.
- `cargo test --test smoke_binary` passes.
- Existing `cargo test` continues to pass (the `build_app_state` extraction doesn't break the production setup path).
- `docs/ROADMAP.md` flipped to `- [x] … (PR #N)`.
