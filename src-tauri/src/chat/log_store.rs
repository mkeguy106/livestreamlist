//! Per-channel chat log persistence.
//!
//! Each connected channel owns a `ChatLogWriter`. On every message the writer
//! appends one JSON line to `~/.local/share/livestreamlist/logs/{platform}/{login}/YYYY-MM-DD.jsonl`.
//! Writes are buffered — we don't flush per-message, which would thrash the
//! disk in busy channels; instead the OS buffers and we flush explicitly when
//! the writer is dropped.
//!
//! History replay: `read_recent(channel_key, limit)` returns the last `limit`
//! messages from today's (and, if short, yesterday's) JSONL so a freshly
//! opened channel isn't blank while waiting for live chat.
//!
//! Disk budget enforcement is left to Phase 4 (Preferences) when the user
//! will be able to configure the cap.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use super::models::ChatMessage;
use crate::config;
use crate::platforms::Platform;

pub struct ChatLogWriter {
    path: PathBuf,
    file: BufWriter<File>,
    date: String,
}

impl ChatLogWriter {
    pub fn open(platform: Platform, channel_id: &str) -> Result<Self> {
        let now = Utc::now();
        let date = now.format("%Y-%m-%d").to_string();
        let path = log_path_for(platform, channel_id, &date)?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        Ok(Self {
            path,
            file: BufWriter::new(file),
            date,
        })
    }

    /// Append one message as a single JSON line. Rolls to a new file if the
    /// UTC date has advanced past the open file's date.
    pub fn append(&mut self, msg: &ChatMessage) -> Result<()> {
        let msg_date = msg.timestamp.format("%Y-%m-%d").to_string();
        if msg_date != self.date {
            self.roll_to(&msg_date, msg.platform, extract_login(&msg.channel_key))?;
        }
        let line = serde_json::to_string(msg).context("serializing ChatMessage")?;
        self.file.write_all(line.as_bytes())?;
        self.file.write_all(b"\n")?;
        Ok(())
    }

    fn roll_to(&mut self, date: &str, platform: Platform, channel_id: &str) -> Result<()> {
        self.file.flush().ok();
        let new_path = log_path_for(platform, channel_id, date)?;
        let new_file = OpenOptions::new().create(true).append(true).open(&new_path)?;
        self.path = new_path;
        self.file = BufWriter::new(new_file);
        self.date = date.to_string();
        Ok(())
    }

    pub fn flush(&mut self) -> Result<()> {
        self.file.flush()?;
        Ok(())
    }
}

impl Drop for ChatLogWriter {
    fn drop(&mut self) {
        let _ = self.file.flush();
    }
}

fn log_path_for(platform: Platform, channel_id: &str, date: &str) -> Result<PathBuf> {
    let dir = config::logs_dir()?
        .join(platform.as_str())
        .join(channel_id.to_ascii_lowercase());
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating log dir {}", dir.display()))?;
    Ok(dir.join(format!("{date}.jsonl")))
}

fn extract_login(channel_key: &str) -> &str {
    channel_key.split_once(':').map(|(_, v)| v).unwrap_or(channel_key)
}

/// Read the most recent `limit` messages for a channel, stitching today's
/// file with yesterday's if today is short. Corrupt lines are skipped.
pub fn read_recent(
    platform: Platform,
    channel_id: &str,
    limit: usize,
) -> Result<Vec<ChatMessage>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let today = Utc::now();
    let yesterday = today - chrono::Duration::days(1);
    let paths = [
        log_path_for(platform, channel_id, &today.format("%Y-%m-%d").to_string())?,
        log_path_for(platform, channel_id, &yesterday.format("%Y-%m-%d").to_string())?,
    ];

    let mut collected: Vec<ChatMessage> = Vec::new();
    for p in paths.iter() {
        if !p.exists() {
            continue;
        }
        let mut from_file = read_tail(p, limit.saturating_sub(collected.len()))?;
        from_file.append(&mut collected);
        collected = from_file;
        if collected.len() >= limit {
            break;
        }
    }
    // Keep the last `limit` entries in chronological order.
    if collected.len() > limit {
        let excess = collected.len() - limit;
        collected.drain(0..excess);
    }
    Ok(collected)
}

/// Read up to `limit` messages from the end of a JSONL file.
///
/// For small counts we just read forward — JSONL files stay modest (busy
/// Twitch channels produce ~5-10 MB of chat in a day) so a linear scan is
/// fine. If this ever becomes a bottleneck, walk backwards in blocks.
fn read_tail(path: &Path, limit: usize) -> Result<Vec<ChatMessage>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut file = File::open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    file.seek(SeekFrom::Start(0)).ok();
    let reader = BufReader::new(file);
    let mut ring: Vec<ChatMessage> = Vec::with_capacity(limit);
    for line in reader.lines() {
        let Ok(line) = line else { continue };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<ChatMessage>(&line) {
            Ok(m) => {
                if ring.len() >= limit {
                    ring.remove(0);
                }
                ring.push(m);
            }
            Err(_) => continue, // skip corrupt lines
        }
    }
    Ok(ring)
}

#[allow(dead_code)]
pub fn parse_timestamp(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))
}
