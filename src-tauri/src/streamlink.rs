use anyhow::{Context, Result};
use std::process::{Command, Stdio};

use crate::platforms::Platform;

fn stream_url(platform: Platform, channel_id: &str) -> String {
    match platform {
        Platform::Twitch => format!("https://www.twitch.tv/{channel_id}"),
        Platform::Youtube => {
            if channel_id.starts_with("UC") {
                format!("https://www.youtube.com/channel/{channel_id}/live")
            } else {
                format!("https://www.youtube.com/@{channel_id}/live")
            }
        }
        Platform::Kick => format!("https://kick.com/{channel_id}"),
        Platform::Chaturbate => format!("https://chaturbate.com/{channel_id}/"),
    }
}

/// Launch `streamlink <url> <quality>` with mpv as the player. The child is
/// detached so closing the app doesn't kill the stream.
///
/// If a `turbo_cookie` is present AND the platform is Twitch, passes it as
/// `--twitch-api-header=Authorization=OAuth {cookie}` plus
/// `--twitch-disable-ads`. The cookie must be the browser's `auth-token`
/// cookie value — OAuth access tokens don't work here (Twitch binds
/// ad-gating to the client-id-less cookie path).
pub fn launch(
    platform: Platform,
    channel_id: &str,
    quality: &str,
    turbo_cookie: Option<&str>,
) -> Result<u32> {
    let url = stream_url(platform, channel_id);
    let quality = if quality.is_empty() { "best" } else { quality };

    let mut cmd = Command::new("streamlink");
    cmd.arg(&url)
        .arg(quality)
        .arg("--player=mpv")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    if platform == Platform::Twitch {
        if let Some(cookie) = turbo_cookie.filter(|c| !c.is_empty()) {
            cmd.arg(format!("--twitch-api-header=Authorization=OAuth {cookie}"));
            cmd.arg("--twitch-disable-ads");
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0); // detach from our process group
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }

    let child = cmd.spawn().with_context(|| {
        format!("spawning streamlink for {url} (is `streamlink` on PATH?)")
    })?;
    Ok(child.id())
}

/// Open a URL in the user's default browser.
pub fn open_browser(url: &str) -> Result<()> {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "start"
    } else {
        "xdg-open"
    };
    let mut cmd = Command::new(opener);
    if opener == "start" {
        cmd.arg(""); // start requires a title arg first
    }
    cmd.arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn().with_context(|| format!("opening {url}"))?;
    Ok(())
}
