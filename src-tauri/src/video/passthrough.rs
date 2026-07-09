//! Localhost CORS passthrough. The webview cannot fetch streamlink's HTTP
//! server directly (it sends no Access-Control-Allow-Origin header —
//! spike-verified), so an app-owned listener proxies `GET /video/{unique_key}`
//! to the session's streamlink port, injecting the ACAO header and streaming
//! MPEG-TS bytes through unbuffered.
//!
//! **One listener per session** (round 6). WebKitGTK's libsoup caps concurrent
//! connections at ~6 per (scheme, host, port). When every stream shared a
//! single passthrough origin, six live videos pinned the whole pool with
//! long-lived streaming fetches — a watchdog rebuild's new fetch then queued
//! behind a dying connection and starved ("playback pipeline stalled
//! repeatedly"). Giving each session its OWN listener port means no two streams
//! ever share a connection pool, so a rebuild's fetch always has headroom. As a
//! result this server is session-scoped: `serve_session` is spawned per session
//! with the (key, generation, upstream_port) it proxies fixed for its whole
//! life — there is no port map to look up.
//!
//! Deliberately hand-rolled minimal HTTP: one route, GET only,
//! connection-close streaming semantics (matching streamlink's own server).
//! No preflight handling needed — the page issues a simple GET.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::UnboundedSender;

/// Consumer lifecycle notifications, keyed by unique_key and tagged with the
/// incarnation generation the connection was serving. The manager's background
/// task turns these into Serving/Lingering transitions — a new connection also
/// cancels linger for watchdog rebuilds, which reconnect without a fresh
/// video_start. Events whose generation no longer matches the live session are
/// ignored (a prior incarnation's dropped connection must not push its
/// successor into linger).
#[derive(Debug)]
pub(crate) enum ConsumerEvent {
    Connected { key: String, generation: u64 },
    Dropped { key: String, generation: u64 },
}

/// Accept loop for ONE session's dedicated listener. No port-map lookup: the
/// listener serves exactly this `key`/`generation` proxying to `upstream_port`
/// for its entire life. Aborted (via the session's `listener_task` JoinHandle)
/// when the session is torn down. The 100 ms error backoff matches the old
/// shared `serve`.
pub(crate) async fn serve_session(
    listener: TcpListener,
    key: String,
    generation: u64,
    upstream_port: u16,
    events: UnboundedSender<ConsumerEvent>,
) {
    loop {
        let (client, _) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                log::warn!("video passthrough accept error: {e}");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            }
        };
        let key = key.clone();
        let events = events.clone();
        tauri::async_runtime::spawn(async move {
            let _ = handle_conn(client, key, generation, upstream_port, events).await;
        });
    }
}

async fn handle_conn(
    mut client: TcpStream,
    key: String,
    generation: u64,
    upstream_port: u16,
    events: UnboundedSender<ConsumerEvent>,
) -> std::io::Result<()> {
    // ── Request head (bounded: request line + a few headers) ──
    let mut head = Vec::with_capacity(1024);
    let mut buf = [0u8; 4096];
    while find_head_end(&head).is_none() {
        if head.len() > 8192 {
            return respond(&mut client, "400 Bad Request").await;
        }
        let n = client.read(&mut buf).await?;
        if n == 0 {
            return Ok(());
        }
        head.extend_from_slice(&buf[..n]);
    }
    let request_line = String::from_utf8_lossy(&head);
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    if method != "GET" {
        return respond(&mut client, "405 Method Not Allowed").await;
    }
    // The listener is session-scoped, so the path's key suffix is decorative:
    // require the `/video/` prefix but tolerate any suffix (the real key comes
    // from `serve_session`, not the URL).
    if !path.starts_with("/video/") {
        return respond(&mut client, "404 Not Found").await;
    }

    // ── Upstream request ──
    let mut upstream = match TcpStream::connect(("127.0.0.1", upstream_port)).await {
        Ok(s) => s,
        Err(_) => return respond(&mut client, "502 Bad Gateway").await,
    };
    upstream
        .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .await?;

    // ── Upstream head: forward with ACAO injected before the terminator ──
    let mut uhead = Vec::with_capacity(1024);
    let head_end = loop {
        if let Some(pos) = find_head_end(&uhead) {
            break pos;
        }
        if uhead.len() > 16384 {
            return respond(&mut client, "502 Bad Gateway").await;
        }
        let n = upstream.read(&mut buf).await?;
        if n == 0 {
            return respond(&mut client, "502 Bad Gateway").await;
        }
        uhead.extend_from_slice(&buf[..n]);
    };
    // uhead[..head_end] = status line + headers WITHOUT the final CRLFCRLF;
    // uhead[head_end + 4..] = body bytes already read past the head.
    client.write_all(&uhead[..head_end]).await?;
    client
        .write_all(b"\r\nAccess-Control-Allow-Origin: *\r\n\r\n")
        .await?;
    client.write_all(&uhead[head_end + 4..]).await?;

    // ── Streaming phase: consumer is officially attached ──
    let _ = events.send(ConsumerEvent::Connected {
        key: key.clone(),
        generation,
    });
    let result = tokio::io::copy(&mut upstream, &mut client).await;
    let _ = events.send(ConsumerEvent::Dropped { key, generation });
    result.map(|_| ())
}

fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

async fn respond(client: &mut TcpStream, status: &str) -> std::io::Result<()> {
    let msg = format!(
        "HTTP/1.1 {status}\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    client.write_all(msg.as_bytes()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// Full loopback: fake upstream (std thread) -> session-scoped passthrough
    /// -> std client. The server side runs on tauri's global async runtime; the
    /// client and fake upstream are std blocking I/O so no test-runtime
    /// juggling. `serve_session` is spawned with the fixed (key, generation,
    /// upstream_port) — no port registration.
    #[test]
    fn serve_session_injects_acao_and_reports_consumer_lifecycle() {
        // Fake upstream mimicking streamlink's response shape.
        let upstream = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let upstream_port = upstream.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut conn, _) = upstream.accept().unwrap();
            let mut discard = [0u8; 1024];
            let _ = conn.read(&mut discard);
            conn.write_all(
                b"HTTP/1.1 200 OK\r\nServer: Streamlink\r\nContent-Type: video/unknown\r\n\r\nTSBYTES",
            )
            .unwrap();
            // connection closes on drop -> passthrough sees EOF
        });

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let listener = tauri::async_runtime::block_on(async {
            TcpListener::bind("127.0.0.1:0").await.unwrap()
        });
        let pass_port = listener.local_addr().unwrap().port();
        // Nonzero generation; the events must round-trip it verbatim.
        tauri::async_runtime::spawn(serve_session(
            listener,
            "twitch:test".into(),
            7,
            upstream_port,
            tx,
        ));

        let mut client = std::net::TcpStream::connect(("127.0.0.1", pass_port)).unwrap();
        client
            .write_all(b"GET /video/twitch:test HTTP/1.1\r\nHost: x\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();

        assert!(response.starts_with("HTTP/1.1 200 OK"), "got: {response}");
        assert!(
            response.contains("Access-Control-Allow-Origin: *"),
            "got: {response}"
        );
        assert!(response.contains("Content-Type: video/unknown"));
        assert!(response.ends_with("TSBYTES"));

        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(matches!(
            rx.try_recv(),
            Ok(ConsumerEvent::Connected { key, generation }) if key == "twitch:test" && generation == 7
        ));
        assert!(matches!(
            rx.try_recv(),
            Ok(ConsumerEvent::Dropped { key, generation }) if key == "twitch:test" && generation == 7
        ));
    }

    #[test]
    fn non_video_path_404s_and_post_405s() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let listener = tauri::async_runtime::block_on(async {
            TcpListener::bind("127.0.0.1:0").await.unwrap()
        });
        let pass_port = listener.local_addr().unwrap().port();
        // Upstream port is irrelevant — 404/405 short-circuit before connecting.
        tauri::async_runtime::spawn(serve_session(listener, "twitch:x".into(), 1, 1, tx));

        // A path outside /video/ -> 404 (prefix required).
        let mut c1 = std::net::TcpStream::connect(("127.0.0.1", pass_port)).unwrap();
        c1.write_all(b"GET /nope HTTP/1.1\r\n\r\n").unwrap();
        let mut r1 = String::new();
        c1.read_to_string(&mut r1).unwrap();
        assert!(r1.starts_with("HTTP/1.1 404"), "got: {r1}");

        // POST is rejected before the path is even considered.
        let mut c2 = std::net::TcpStream::connect(("127.0.0.1", pass_port)).unwrap();
        c2.write_all(b"POST /video/twitch:x HTTP/1.1\r\n\r\n")
            .unwrap();
        let mut r2 = String::new();
        c2.read_to_string(&mut r2).unwrap();
        assert!(r2.starts_with("HTTP/1.1 405"), "got: {r2}");
    }
}
