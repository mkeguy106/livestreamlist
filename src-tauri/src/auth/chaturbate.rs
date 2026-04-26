//! Chaturbate auth — in-app webview sign-in.
//!
//! Chaturbate has no real OAuth surface. Logged-in state is decided by
//! the `sessionid` cookie on chaturbate.com (see livestream.list.qt's
//! chat/chaturbate_web_chat.py for prior art). We open a popup
//! WebviewWindow at the login page with a persistent profile dir, poll
//! its cookie jar until `sessionid` appears, and write a small stamp
//! file marking the user as signed in. The cookies themselves live in
//! the webview profile dir (shared with the chat embed); the stamp is
//! a presence flag plus timestamps the UI uses to render hints.

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::config;

const STAMP_FILENAME: &str = "chaturbate-auth.json";
const LOGIN_WINDOW_LABEL: &str = "chaturbate-login";
const LOGIN_URL: &str = "https://chaturbate.com/auth/login/";
const SITE_URL: &str = "https://chaturbate.com/";
const POLL_INTERVAL: Duration = Duration::from_millis(750);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChaturbateAuth {
    pub logged_in_at: DateTime<Utc>,
    pub last_verified_at: DateTime<Utc>,
}

fn stamp_path() -> Result<PathBuf> {
    Ok(config::data_dir()?.join(STAMP_FILENAME))
}

pub fn webview_profile_dir() -> Result<PathBuf> {
    let dir = config::data_dir()?.join("webviews").join("chaturbate");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating webview profile dir {}", dir.display()))?;
    Ok(dir)
}

pub fn load() -> Result<Option<ChaturbateAuth>> {
    let path = stamp_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path)
        .with_context(|| format!("reading {}", path.display()))?;
    let stamp: ChaturbateAuth = serde_json::from_slice(&bytes)
        .context("parsing Chaturbate stamp file")?;
    Ok(Some(stamp))
}

pub fn save(stamp: &ChaturbateAuth) -> Result<()> {
    let path = stamp_path()?;
    let bytes = serde_json::to_vec(stamp).context("serialising Chaturbate stamp")?;
    config::atomic_write(&path, &bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Bumps `last_verified_at` if the stamp exists. No-op when not signed in.
pub fn touch_verified() -> Result<()> {
    let Some(mut stamp) = load()? else {
        return Ok(());
    };
    stamp.last_verified_at = Utc::now();
    save(&stamp)
}

pub fn clear() -> Result<()> {
    if let Ok(path) = stamp_path() {
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
    }
    // Inline the profile-dir path — calling webview_profile_dir() here would
    // create the directory (it has create_dir_all baked in) only to
    // immediately delete it.
    if let Ok(base) = config::data_dir() {
        let dir = base.join("webviews").join("chaturbate");
        if dir.exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
    Ok(())
}

/// Open a child WebviewWindow at the Chaturbate login page, poll its
/// cookie jar until `sessionid` appears, then save + close. Bubbles a
/// clear error if the user closes the window or the timeout (5 min)
/// expires.
pub async fn login_via_webview(app: AppHandle) -> Result<ChaturbateAuth> {
    if let Some(existing) = app.get_webview_window(LOGIN_WINDOW_LABEL) {
        let _ = existing.close();
    }
    let profile_dir = webview_profile_dir()?;
    let window = WebviewWindowBuilder::new(
        &app,
        LOGIN_WINDOW_LABEL,
        WebviewUrl::External(LOGIN_URL.parse()?),
    )
    .title("Sign in to Chaturbate")
    .inner_size(480.0, 720.0)
    .min_inner_size(400.0, 600.0)
    .data_directory(profile_dir)
    .build()
    .context("opening Chaturbate login window")?;

    let site: url::Url = SITE_URL.parse()?;
    let started = std::time::Instant::now();

    loop {
        if started.elapsed() > LOGIN_TIMEOUT {
            let _ = window.close();
            anyhow::bail!("Chaturbate login timed out after 5 minutes");
        }
        if app.get_webview_window(LOGIN_WINDOW_LABEL).is_none() {
            anyhow::bail!("login window closed before sign-in completed");
        }

        match window.cookies_for_url(site.clone()) {
            Ok(jar) => {
                let signed_in = jar
                    .iter()
                    .any(|c| c.name() == "sessionid" && !c.value().is_empty());
                if signed_in {
                    let now = Utc::now();
                    let stamp = ChaturbateAuth {
                        logged_in_at: now,
                        last_verified_at: now,
                    };
                    save(&stamp)?;
                    let _ = window.close();
                    return Ok(stamp);
                }
            }
            Err(e) => log::debug!("cookies_for_url(chaturbate.com): {e}"),
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn round_trip_serialises_rfc3339() {
        let stamp = ChaturbateAuth {
            logged_in_at: chrono::Utc.with_ymd_and_hms(2026, 4, 25, 10, 0, 0).unwrap(),
            last_verified_at: chrono::Utc.with_ymd_and_hms(2026, 4, 25, 11, 30, 0).unwrap(),
        };
        let json = serde_json::to_string(&stamp).unwrap();
        assert!(json.contains("2026-04-25T10:00:00Z"));
        assert!(json.contains("2026-04-25T11:30:00Z"));
        let back: ChaturbateAuth = serde_json::from_str(&json).unwrap();
        assert_eq!(back.logged_in_at, stamp.logged_in_at);
        assert_eq!(back.last_verified_at, stamp.last_verified_at);
    }
}
