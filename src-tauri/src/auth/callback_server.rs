//! Minimal single-shot loopback HTTP server for OAuth callbacks.
//!
//! Twitch's implicit flow returns the access token in the URL **fragment**
//! (`#access_token=...`), which browsers never send to the server. So the
//! flow is:
//!
//!   1. Bind `127.0.0.1:65432`, spawn accept loop
//!   2. Browser hits `GET /callback` with `#access_token=...` in the URL
//!   3. We respond with an HTML page containing JS that reads
//!      `window.location.hash` and POSTs it back to `/token`
//!   4. We receive the `/token` POST, parse the form body, resolve the
//!      oneshot with the token and terminate
//!
//! Kick's PKCE flow (Phase 2b later) returns `?code=...` in the query string
//! directly — the same server handles that via `/callback?code=…`.

use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::time::Duration;
use tokio::sync::oneshot;

pub const CALLBACK_HOST: &str = "127.0.0.1";
pub const CALLBACK_PORT: u16 = 65432;

pub fn redirect_uri() -> String {
    format!("http://{CALLBACK_HOST}:{CALLBACK_PORT}/callback")
}

/// What came back from the browser.
#[derive(Debug, Clone)]
pub enum CallbackResult {
    /// Implicit flow — token in URL fragment, sent to /token via JS POST.
    Token {
        access_token: String,
        scope: Option<String>,
        token_type: Option<String>,
    },
    /// Code flow — `?code=...&state=...` in the query string at /callback.
    Code { code: String, state: Option<String> },
    /// `?error=...&error_description=...`.
    Error { error: String, description: Option<String> },
}

/// Run the server to completion on a background blocking thread. Returns a
/// oneshot receiver that fires with the first callback result. Drops with no
/// result if the client closes the browser without finishing auth.
pub fn spawn_once() -> Result<oneshot::Receiver<CallbackResult>> {
    let addr: SocketAddr = (Ipv4Addr::new(127, 0, 0, 1), CALLBACK_PORT).into();
    let listener = TcpListener::bind(addr)
        .with_context(|| format!("binding {addr} (another OAuth flow in progress?)"))?;
    listener
        .set_nonblocking(false)
        .context("switching to blocking mode")?;

    // 5 min is plenty for a login flow; beyond that something's gone wrong.
    let (tx, rx) = oneshot::channel::<CallbackResult>();

    std::thread::spawn(move || {
        let mut tx = Some(tx);
        // Timeout any single read to avoid a hung connection holding the
        // server open forever.
        let _ = listener.set_nonblocking(false);
        let deadline = std::time::Instant::now() + Duration::from_secs(5 * 60);
        while std::time::Instant::now() < deadline {
            let Ok((stream, _)) = listener.accept() else { break };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
            match handle_conn(stream) {
                Ok(Some(result)) => {
                    if let Some(t) = tx.take() {
                        let _ = t.send(result);
                    }
                    break;
                }
                Ok(None) => continue,
                Err(e) => {
                    log::warn!("callback server conn failed: {e:#}");
                    continue;
                }
            }
        }
    });

    Ok(rx)
}

/// Returns `Some(result)` if this connection produced the final token/code;
/// `None` if it was an intermediate request (the HTML bounce page).
fn handle_conn(mut stream: TcpStream) -> Result<Option<CallbackResult>> {
    let mut buf = vec![0u8; 8192];
    let n = stream
        .read(&mut buf)
        .context("reading request")?;
    let raw = String::from_utf8_lossy(&buf[..n]).to_string();

    let (method, path, body_opt) = parse_request(&raw)?;

    if method == "GET" && path.starts_with("/callback") {
        // Code flow: ?code=...&state=...
        if let Some(qs) = path.splitn(2, '?').nth(1) {
            let q = parse_query(qs);
            if let Some(err) = q.get("error") {
                respond_html(&mut stream, ERROR_PAGE)?;
                return Ok(Some(CallbackResult::Error {
                    error: err.clone(),
                    description: q.get("error_description").cloned(),
                }));
            }
            if let Some(code) = q.get("code") {
                respond_html(&mut stream, SUCCESS_PAGE)?;
                return Ok(Some(CallbackResult::Code {
                    code: code.clone(),
                    state: q.get("state").cloned(),
                }));
            }
        }
        // Implicit flow: token is in fragment → bounce page runs JS that
        // reads `window.location.hash` and POSTs to /token.
        respond_html(&mut stream, BOUNCE_PAGE)?;
        return Ok(None);
    }

    if method == "POST" && path == "/token" {
        let body = body_opt.unwrap_or_default();
        let q = parse_query(body);
        respond_html(&mut stream, SUCCESS_PAGE)?;
        if let Some(err) = q.get("error") {
            return Ok(Some(CallbackResult::Error {
                error: err.clone(),
                description: q.get("error_description").cloned(),
            }));
        }
        let token = q
            .get("access_token")
            .cloned()
            .ok_or_else(|| anyhow!("/token POST missing access_token"))?;
        return Ok(Some(CallbackResult::Token {
            access_token: token,
            scope: q.get("scope").cloned(),
            token_type: q.get("token_type").cloned(),
        }));
    }

    // Anything else — friendly 404.
    let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
    Ok(None)
}

fn parse_request(raw: &str) -> Result<(&str, &str, Option<&str>)> {
    let mut lines = raw.split("\r\n");
    let start = lines.next().ok_or_else(|| anyhow!("empty request"))?;
    let mut parts = start.split_whitespace();
    let method = parts.next().ok_or_else(|| anyhow!("no method"))?;
    let path = parts.next().ok_or_else(|| anyhow!("no path"))?;

    // Skip headers until blank line
    for line in lines.by_ref() {
        if line.is_empty() {
            break;
        }
    }
    let body: Option<&str> = lines.next();
    Ok((method, path, body))
}

fn parse_query(s: &str) -> HashMap<String, String> {
    s.split('&')
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            Some((url_decode(k), url_decode(v)))
        })
        .collect()
}

fn url_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = hex(bytes[i + 1]);
                let lo = hex(bytes[i + 2]);
                match (hi, lo) {
                    (Some(h), Some(l)) => out.push((h << 4 | l) as char),
                    _ => out.push('%'),
                }
                i += 3;
            }
            c => {
                out.push(c as char);
                i += 1;
            }
        }
    }
    out
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn respond_html(stream: &mut TcpStream, body: &str) -> Result<()> {
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(resp.as_bytes()).context("writing response")?;
    Ok(())
}

const BOUNCE_PAGE: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>Finishing login…</title>
<style>
  body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
         background: #09090b; color: #e4e4e7; margin: 0;
         display: flex; align-items: center; justify-content: center; height: 100vh; }
  .card { text-align: center; max-width: 420px; padding: 24px; }
  h1 { font-size: 16px; font-weight: 600; margin: 0 0 12px; }
  p  { font-size: 13px; color: #a1a1aa; margin: 0; }
</style>
</head><body><div class="card">
  <h1>Finishing login…</h1>
  <p>You can close this window.</p>
</div>
<script>
  (function () {
    var h = (window.location.hash || '').replace(/^#/, '');
    if (!h) { document.querySelector('h1').textContent = 'No token in URL'; return; }
    fetch('/token', { method: 'POST', headers: {'Content-Type': 'application/x-www-form-urlencoded'}, body: h })
      .then(function () { document.querySelector('h1').textContent = 'Logged in'; })
      .catch(function (e) { document.querySelector('h1').textContent = 'Login error: ' + e; });
  })();
</script>
</body></html>"#;

const SUCCESS_PAGE: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>Logged in</title>
<style>
  body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
         background: #09090b; color: #e4e4e7; margin: 0;
         display: flex; align-items: center; justify-content: center; height: 100vh; }
  .card { text-align: center; max-width: 420px; padding: 24px; }
  h1 { font-size: 16px; font-weight: 600; margin: 0 0 8px; color: #22c55e; }
  p  { font-size: 13px; color: #a1a1aa; margin: 0; }
</style></head><body><div class="card">
  <h1>Logged in</h1>
  <p>You can close this window and return to livestreamlist.</p>
</div></body></html>"#;

const ERROR_PAGE: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>Login failed</title>
<style>
  body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
         background: #09090b; color: #e4e4e7; margin: 0;
         display: flex; align-items: center; justify-content: center; height: 100vh; }
  .card { text-align: center; max-width: 420px; padding: 24px; }
  h1 { font-size: 16px; font-weight: 600; margin: 0 0 8px; color: #ef4444; }
  p  { font-size: 13px; color: #a1a1aa; margin: 0; }
</style></head><body><div class="card">
  <h1>Login failed</h1>
  <p>See livestreamlist for details.</p>
</div></body></html>"#;
