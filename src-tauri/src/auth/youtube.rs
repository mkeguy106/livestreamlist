//! YouTube cookie auth — manual paste + in-app WebviewWindow sign-in.
//!
//! YouTube has no real OAuth surface. Authenticated endpoints (subscriptions,
//! age-restricted videos, member chat, etc.) want the standard Google session
//! cookies. We capture the canonical five — SID, HSID, SSID, APISID, SAPISID
//! — and stash them in two places:
//!
//!   - JSON in the keyring (single source of truth, survives restart)
//!   - Netscape `youtube-cookies.txt` under XDG data dir, so `yt-dlp
//!     --cookies <path>` can authenticate as us without us regenerating the
//!     file on every spawn.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use super::tokens;
use crate::config;

const KEYRING_ENTRY: &str = "youtube_cookies";
const COOKIES_FILENAME: &str = "youtube-cookies.txt";
const REQUIRED: &[&str] = &["SID", "HSID", "SSID", "APISID", "SAPISID"];
const LOGIN_WINDOW_LABEL: &str = "youtube-login";
const LOGIN_URL: &str = "https://accounts.google.com/signin/v2/identifier?service=youtube";
const POLL_INTERVAL: Duration = Duration::from_millis(750);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300); // 5 min — generous for 2FA

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YouTubeCookies {
    #[serde(rename = "SID")]
    pub sid: String,
    #[serde(rename = "HSID")]
    pub hsid: String,
    #[serde(rename = "SSID")]
    pub ssid: String,
    #[serde(rename = "APISID")]
    pub apisid: String,
    #[serde(rename = "SAPISID")]
    pub sapisid: String,
}

impl YouTubeCookies {
    fn from_map(map: &HashMap<String, String>) -> Result<Self> {
        let pull = |k: &str| {
            map.get(k)
                .filter(|v| !v.is_empty())
                .cloned()
                .ok_or_else(|| anyhow!("missing cookie: {k}"))
        };
        Ok(Self {
            sid: pull("SID")?,
            hsid: pull("HSID")?,
            ssid: pull("SSID")?,
            apisid: pull("APISID")?,
            sapisid: pull("SAPISID")?,
        })
    }

    fn entries(&self) -> [(&'static str, &str); 5] {
        [
            ("SID", &self.sid),
            ("HSID", &self.hsid),
            ("SSID", &self.ssid),
            ("APISID", &self.apisid),
            ("SAPISID", &self.sapisid),
        ]
    }
}

pub fn cookies_path() -> Result<PathBuf> {
    Ok(config::data_dir()?.join(COOKIES_FILENAME))
}

pub fn save(cookies: &YouTubeCookies) -> Result<()> {
    let json = serde_json::to_string(cookies).context("serialising YouTube cookies")?;
    tokens::save(KEYRING_ENTRY, &json).context("saving YouTube cookies to keyring")?;
    write_netscape_file(cookies).context("writing yt-dlp cookies file")?;
    Ok(())
}

pub fn load() -> Result<Option<YouTubeCookies>> {
    let Some(json) = tokens::load(KEYRING_ENTRY)? else {
        return Ok(None);
    };
    let cookies = serde_json::from_str(&json).context("parsing stored YouTube cookies")?;
    Ok(Some(cookies))
}

pub fn clear() -> Result<()> {
    tokens::clear(KEYRING_ENTRY).ok();
    if let Ok(path) = cookies_path() {
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
    }
    Ok(())
}

/// True iff a saved cookie set is on disk (yt-dlp consumes via `--cookies`).
pub fn cookies_file_present() -> bool {
    cookies_path().map(|p| p.exists()).unwrap_or(false)
}

/// Browser the user told us to pull cookies from, plus a path that exists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedBrowser {
    /// yt-dlp `--cookies-from-browser` name. Lowercased.
    pub id: String,
    /// User-facing label (e.g. "Google Chrome", "Firefox").
    pub label: String,
}

/// Browsers yt-dlp's `--cookies-from-browser` supports, paired with the
/// directories we check to decide they're installed. Order = display order.
fn browser_table() -> Vec<(&'static str, &'static str, Vec<PathBuf>)> {
    let home = dirs::home_dir().unwrap_or_default();
    let config = dirs::config_dir().unwrap_or_default();
    let local = dirs::data_local_dir().unwrap_or_default();

    let chromium_paths = |sub: &str| -> Vec<PathBuf> {
        let mut out = Vec::new();
        // Linux/macOS layouts both go through dirs::config_dir() correctly.
        out.push(config.join(sub));
        // Windows: Local AppData
        out.push(local.join(sub));
        out
    };

    vec![
        ("chrome",   "Google Chrome",
            chromium_paths("google-chrome").into_iter()
                .chain([config.join("Google/Chrome"), local.join("Google/Chrome")])
                .collect()),
        ("brave",    "Brave",
            chromium_paths("BraveSoftware/Brave-Browser")),
        ("chromium", "Chromium",
            chromium_paths("chromium")),
        ("edge",     "Microsoft Edge",
            chromium_paths("microsoft-edge").into_iter()
                .chain([config.join("Microsoft/Edge"), local.join("Microsoft/Edge")])
                .collect()),
        ("opera",    "Opera",
            chromium_paths("opera").into_iter()
                .chain([config.join("Opera Software/Opera Stable")])
                .collect()),
        ("vivaldi",  "Vivaldi",
            chromium_paths("vivaldi").into_iter()
                .chain([config.join("Vivaldi")])
                .collect()),
        ("firefox",  "Firefox",
            vec![
                home.join(".mozilla/firefox"),
                home.join("Library/Application Support/Firefox"),
                home.join("AppData/Roaming/Mozilla/Firefox"),
                local.join("Mozilla/Firefox"),
            ]),
        ("librewolf", "LibreWolf",
            vec![
                home.join(".librewolf"),
                home.join("Library/Application Support/LibreWolf"),
            ]),
    ]
}

/// Scan known config / data dirs for browser cookie stores. Returns only the
/// browsers we found something for.
pub fn detect_browsers() -> Vec<DetectedBrowser> {
    browser_table()
        .into_iter()
        .filter(|(_, _, paths)| paths.iter().any(|p| p.exists()))
        .map(|(id, label, _)| DetectedBrowser {
            id: id.to_string(),
            label: label.to_string(),
        })
        .collect()
}

/// yt-dlp args for the configured cookie source. Returned as a `Vec<OsString>`
/// so the caller can splice into `Command::args` without string conversions.
/// Priority: configured browser → pasted cookies file → none.
pub fn yt_dlp_cookie_args(browser: Option<&str>) -> Vec<OsString> {
    if let Some(name) = browser.filter(|s| !s.is_empty()) {
        return vec![
            OsString::from("--cookies-from-browser"),
            OsString::from(name),
        ];
    }
    if let Ok(path) = cookies_path() {
        if path.exists() {
            return vec![OsString::from("--cookies"), path.into_os_string()];
        }
    }
    Vec::new()
}

/// Parse a user-pasted block. Accepts:
///   - Cookie-header form: `SID=…; HSID=…; SSID=…; APISID=…; SAPISID=…`
///   - One-per-line `name=value` pairs (with or without trailing `;`)
///   - Netscape cookies.txt format (`# Netscape …` header optional; tab-
///     separated rows). For Netscape rows we use the 6th column as name and
///     7th as value, ignoring domain so users can paste a wider export.
pub fn parse_pasted(text: &str) -> Result<YouTubeCookies> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        anyhow::bail!("nothing to parse — paste your cookies into the box");
    }
    let mut map: HashMap<String, String> = HashMap::new();
    let looks_netscape = trimmed
        .lines()
        .any(|l| l.starts_with("# Netscape") || l.starts_with("# HTTP Cookie"))
        || trimmed.contains("\tTRUE\t")
        || trimmed.contains("\tFALSE\t");

    if looks_netscape {
        for line in trimmed.lines() {
            let line = line.trim_start_matches("#HttpOnly_");
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 7 {
                map.insert(parts[5].to_string(), parts[6].to_string());
            }
        }
    } else {
        for chunk in trimmed.split(|c| c == ';' || c == '\n') {
            let pair = chunk.trim().trim_end_matches(';');
            if pair.is_empty() {
                continue;
            }
            if let Some((k, v)) = pair.split_once('=') {
                map.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
    }
    YouTubeCookies::from_map(&map)
}

fn write_netscape_file(cookies: &YouTubeCookies) -> Result<()> {
    use std::fmt::Write;
    let path = cookies_path()?;
    // 180 days from now — far enough that yt-dlp won't drop them, short
    // enough that we'll re-prompt eventually if Google rotates them.
    let expires = (chrono::Utc::now() + chrono::Duration::days(180)).timestamp();
    let mut out = String::new();
    out.push_str("# Netscape HTTP Cookie File\n");
    out.push_str("# Auto-generated by livestreamlist; do not edit by hand.\n\n");
    for (name, value) in cookies.entries() {
        // Netscape format: domain  include_subdomains  path  secure  expires  name  value
        writeln!(
            out,
            ".google.com\tTRUE\t/\tTRUE\t{expires}\t{name}\t{value}"
        )
        .ok();
    }
    config::atomic_write(&path, out.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Per-platform persistent webview profile dir. The auth-capture window AND
/// the inline chat embed share this dir so cookies set during sign-in are
/// reused by the embed without us having to re-inject them.
pub fn webview_profile_dir() -> Result<PathBuf> {
    let dir = config::data_dir()?.join("webviews").join("youtube");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating webview profile dir {}", dir.display()))?;
    Ok(dir)
}

/// Seed the main webview's cookie jar with the 5 stored Google session
/// cookies so any iframe loaded into the React tree (including the embedded
/// /live_chat) sees the user as signed in. Idempotent — silently no-ops when
/// no cookies are stored.
pub fn inject_into_main_webview<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<()> {
    let Some(cookies) = load()? else {
        return Ok(());
    };
    let Some(main) = tauri::Manager::get_webview_window(app, "main") else {
        return Ok(());
    };
    use tauri::webview::cookie;
    for (name, value) in cookies.entries() {
        // Google sets these on `.google.com` with Secure; mirror that here so
        // the cookie scope matches the browser's. Path "/", HttpOnly, Secure.
        let cookie = cookie::Cookie::build((name.to_string(), value.to_string()))
            .domain(".google.com")
            .path("/")
            .secure(true)
            .http_only(true)
            .build();
        if let Err(e) = main.set_cookie(cookie) {
            log::warn!("set_cookie {name} failed: {e:#}");
        }
    }
    Ok(())
}

/// Open a child WebviewWindow at Google sign-in, poll its cookie jar until
/// all 5 target cookies appear, then save + close. Bubbles a clear error
/// if the user closes the window or the timeout (5 min) expires.
pub async fn login_via_webview(app: AppHandle) -> Result<YouTubeCookies> {
    if let Some(existing) = app.get_webview_window(LOGIN_WINDOW_LABEL) {
        let _ = existing.close();
    }
    let profile_dir = webview_profile_dir()?;
    let window = WebviewWindowBuilder::new(
        &app,
        LOGIN_WINDOW_LABEL,
        WebviewUrl::External(LOGIN_URL.parse()?),
    )
    .title("Sign in to YouTube")
    .inner_size(520.0, 720.0)
    .min_inner_size(400.0, 600.0)
    .data_directory(profile_dir)
    .build()
    .context("opening YouTube login window")?;

    let google: url::Url = "https://www.google.com/".parse()?;
    let started = std::time::Instant::now();

    loop {
        if started.elapsed() > LOGIN_TIMEOUT {
            let _ = window.close();
            anyhow::bail!("YouTube login timed out after 5 minutes");
        }
        if app.get_webview_window(LOGIN_WINDOW_LABEL).is_none() {
            anyhow::bail!("login window closed before sign-in completed");
        }

        match window.cookies_for_url(google.clone()) {
            Ok(jar) => {
                let map: HashMap<String, String> = jar
                    .into_iter()
                    .map(|c| (c.name().to_string(), c.value().to_string()))
                    .collect();
                if REQUIRED
                    .iter()
                    .all(|k| map.get(*k).map(|v| !v.is_empty()).unwrap_or(false))
                {
                    let cookies = YouTubeCookies::from_map(&map)?;
                    save(&cookies)?;
                    let _ = window.close();
                    return Ok(cookies);
                }
            }
            Err(e) => log::debug!("cookies_for_url(google.com): {e}"),
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cookie_header() {
        let text = "SID=AAA; HSID=BBB; SSID=CCC; APISID=DDD; SAPISID=EEE";
        let c = parse_pasted(text).unwrap();
        assert_eq!(c.sid, "AAA");
        assert_eq!(c.hsid, "BBB");
        assert_eq!(c.sapisid, "EEE");
    }

    #[test]
    fn parses_lines() {
        let text = "SID=AAA\nHSID=BBB\nSSID=CCC\nAPISID=DDD\nSAPISID=EEE\n";
        let c = parse_pasted(text).unwrap();
        assert_eq!(c.apisid, "DDD");
    }

    #[test]
    fn parses_netscape_format() {
        let text = "# Netscape HTTP Cookie File\n\
            .google.com\tTRUE\t/\tTRUE\t9999999999\tSID\tAAA\n\
            .google.com\tTRUE\t/\tTRUE\t9999999999\tHSID\tBBB\n\
            .google.com\tTRUE\t/\tTRUE\t9999999999\tSSID\tCCC\n\
            .google.com\tTRUE\t/\tTRUE\t9999999999\tAPISID\tDDD\n\
            .google.com\tTRUE\t/\tTRUE\t9999999999\tSAPISID\tEEE\n";
        let c = parse_pasted(text).unwrap();
        assert_eq!(c.sid, "AAA");
        assert_eq!(c.sapisid, "EEE");
    }

    #[test]
    fn rejects_missing_cookie() {
        let text = "SID=AAA; HSID=BBB; SSID=CCC; APISID=DDD"; // no SAPISID
        let err = parse_pasted(text).unwrap_err();
        assert!(err.to_string().contains("SAPISID"), "got: {err}");
    }

    #[test]
    fn rejects_empty() {
        assert!(parse_pasted("").is_err());
        assert!(parse_pasted("   \n\n  ").is_err());
    }
}
