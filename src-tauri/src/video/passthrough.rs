//! Localhost CORS passthrough. The webview cannot fetch streamlink's HTTP
//! server directly (it sends no Access-Control-Allow-Origin header —
//! spike-verified), so one app-owned listener proxies
//! `GET /video/{unique_key}` to the session's streamlink port, injecting the
//! ACAO header and streaming MPEG-TS bytes through unbuffered.
//!
//! Deliberately hand-rolled minimal HTTP: one route, GET only,
//! connection-close streaming semantics (matching streamlink's own server).
//! No preflight handling needed — the page issues a simple GET.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::UnboundedSender;

/// unique_key -> (streamlink session port, incarnation generation). Written by
/// VideoManager. The generation is captured at connection time and echoed on
/// the consumer events so the manager can drop events belonging to an
/// incarnation that was replaced (quality switch) under the same key.
pub(crate) type PortMap = Arc<parking_lot::Mutex<HashMap<String, (u16, u64)>>>;

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

pub(crate) async fn serve(
    listener: TcpListener,
    ports: PortMap,
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
        let ports = Arc::clone(&ports);
        let events = events.clone();
        tauri::async_runtime::spawn(async move {
            let _ = handle_conn(client, ports, events).await;
        });
    }
}

async fn handle_conn(
    mut client: TcpStream,
    ports: PortMap,
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
    // unique_keys contain ':' which is a legal path character — no decoding.
    let Some(key) = path.strip_prefix("/video/") else {
        return respond(&mut client, "404 Not Found").await;
    };
    let entry = ports.lock().get(key).copied();
    let Some((port, generation)) = entry else {
        return respond(&mut client, "404 Not Found").await;
    };

    // ── Upstream request ──
    let mut upstream = match TcpStream::connect(("127.0.0.1", port)).await {
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
    let key = key.to_string();
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

    /// Full loopback: fake upstream (std thread) -> passthrough -> std client.
    /// The server side runs on tauri's global async runtime; the client and
    /// fake upstream are std blocking I/O so no test-runtime juggling.
    #[test]
    fn injects_acao_and_reports_consumer_lifecycle() {
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

        let ports: PortMap = Arc::new(parking_lot::Mutex::new(HashMap::new()));
        // Register with a nonzero generation; the events must round-trip it.
        ports
            .lock()
            .insert("twitch:test".into(), (upstream_port, 7));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let listener = tauri::async_runtime::block_on(async {
            TcpListener::bind("127.0.0.1:0").await.unwrap()
        });
        let pass_port = listener.local_addr().unwrap().port();
        tauri::async_runtime::spawn(serve(listener, Arc::clone(&ports), tx));

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
    fn unknown_key_404s_and_post_405s() {
        let ports: PortMap = Arc::new(parking_lot::Mutex::new(HashMap::new()));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let listener = tauri::async_runtime::block_on(async {
            TcpListener::bind("127.0.0.1:0").await.unwrap()
        });
        let pass_port = listener.local_addr().unwrap().port();
        tauri::async_runtime::spawn(serve(listener, ports, tx));

        let mut c1 = std::net::TcpStream::connect(("127.0.0.1", pass_port)).unwrap();
        c1.write_all(b"GET /video/twitch:nope HTTP/1.1\r\n\r\n")
            .unwrap();
        let mut r1 = String::new();
        c1.read_to_string(&mut r1).unwrap();
        assert!(r1.starts_with("HTTP/1.1 404"));

        let mut c2 = std::net::TcpStream::connect(("127.0.0.1", pass_port)).unwrap();
        c2.write_all(b"POST /video/twitch:x HTTP/1.1\r\n\r\n")
            .unwrap();
        let mut r2 = String::new();
        c2.read_to_string(&mut r2).unwrap();
        assert!(r2.starts_with("HTTP/1.1 405"));
    }
}
