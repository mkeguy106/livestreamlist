//! streamlink invocation helpers: pure argument building + port allocation.

use anyhow::Context;

/// Build the argv (after the `streamlink` binary itself) for one session.
/// Pure so the exact flag set — the load-bearing part of the whole feature —
/// is unit-tested.
pub(crate) fn build_streamlink_args(
    login: &str,
    port: u16,
    quality: &str,
    web_token: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "--player-external-http".to_string(),
        "--player-external-http-port".to_string(),
        port.to_string(),
        "--player-external-http-interface".to_string(),
        "127.0.0.1".to_string(),
        "--twitch-low-latency".to_string(),
        "--quiet".to_string(),
    ];
    if let Some(tok) = web_token {
        args.push(format!("--twitch-api-header=Authorization=OAuth {tok}"));
    }
    args.push(format!("twitch.tv/{login}"));
    args.push(format!("{quality},best"));
    args
}

/// OS-assigned free port: bind a probe listener to 127.0.0.1:0, read the
/// port, drop the listener. Small race between drop and streamlink's own
/// bind — the caller retries with a fresh port on spawn failure.
pub(crate) fn alloc_port() -> anyhow::Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).context("binding port probe")?;
    Ok(listener.local_addr()?.port())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_without_token() {
        let args = build_streamlink_args("gems", 8901, "720p60", None);
        assert!(args.contains(&"--player-external-http".to_string()));
        assert!(args.contains(&"--twitch-low-latency".to_string()));
        assert!(args.contains(&"127.0.0.1".to_string()));
        let port_flag = args
            .iter()
            .position(|a| a == "--player-external-http-port")
            .unwrap();
        assert_eq!(args[port_flag + 1], "8901");
        assert_eq!(args[args.len() - 2], "twitch.tv/gems");
        assert_eq!(args[args.len() - 1], "720p60,best");
        assert!(!args.iter().any(|a| a.starts_with("--twitch-api-header")));
    }

    #[test]
    fn args_with_token() {
        let args = build_streamlink_args("gems", 8901, "480p", Some("abc123"));
        assert!(args.contains(&"--twitch-api-header=Authorization=OAuth abc123".to_string()));
        assert_eq!(args[args.len() - 1], "480p,best");
    }

    #[test]
    fn alloc_port_returns_high_port() {
        let p = alloc_port().unwrap();
        assert!(p > 1024);
    }
}
