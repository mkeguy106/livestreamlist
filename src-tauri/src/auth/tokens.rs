//! Token storage via the system keyring.
//!
//! Linux: Secret Service (GNOME Keyring / KWallet via libsecret). macOS:
//! Keychain. Windows: Credential Manager. If no keyring is available
//! `load`/`save` return an error and the UI surfaces "not logged in" — we
//! don't silently fall back to disk.

use anyhow::{Context, Result};
use keyring::Entry;

const SERVICE: &str = "livestreamlist";

pub const TWITCH_TURBO_COOKIE: &str = "twitch_turbo_auth_cookie";

fn entry(account: &str) -> Result<Entry> {
    Entry::new(SERVICE, account).with_context(|| format!("opening keyring entry {account}"))
}

pub fn save(account: &str, value: &str) -> Result<()> {
    entry(account)?
        .set_password(value)
        .with_context(|| format!("saving keyring entry {account}"))
}

pub fn load(account: &str) -> Result<Option<String>> {
    match entry(account)?.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading keyring entry {account}")),
    }
}

pub fn clear(account: &str) -> Result<()> {
    match entry(account)?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e).with_context(|| format!("clearing keyring entry {account}")),
    }
}
