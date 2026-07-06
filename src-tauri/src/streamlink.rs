use anyhow::{Context, Result};
use std::process::{Command, Stdio};

use crate::platforms::Platform;

pub fn stream_url(platform: Platform, channel_id: &str) -> String {
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
