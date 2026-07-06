//! Notification sound playback via rodio. Fire-and-forget: failures are
//! logged and never block the visual notification.

use crate::settings::NotificationSettings;

static DEFAULT_SOUND: &[u8] = include_bytes!("../../sounds/notify.ogg");

/// Honors `sound_enabled`; resolves custom file vs bundled default.
pub fn play(settings: &NotificationSettings) {
    if !settings.sound_enabled {
        return;
    }
    play_path_or_default(&settings.custom_sound_path);
}

/// Play `custom_path` if non-empty and readable, else the bundled default.
/// Detached thread: rodio's OutputStream must outlive playback and must not
/// block the caller (refresh loop / IPC).
pub fn play_path_or_default(custom_path: &str) {
    let custom = custom_path.trim().to_string();
    let spawned = std::thread::Builder::new()
        .name("notify-sound".into())
        .spawn(move || {
            let bytes: std::borrow::Cow<'static, [u8]> = if custom.is_empty() {
                std::borrow::Cow::Borrowed(DEFAULT_SOUND)
            } else {
                match std::fs::read(&custom) {
                    Ok(b) => std::borrow::Cow::Owned(b),
                    Err(e) => {
                        log::warn!(
                            "custom notify sound {custom:?} unreadable ({e}); using default"
                        );
                        std::borrow::Cow::Borrowed(DEFAULT_SOUND)
                    }
                }
            };
            if let Err(e) = play_bytes(&bytes) {
                log::warn!("notification sound playback failed: {e:#}");
            }
        });
    if let Err(e) = spawned {
        log::warn!("notify sound thread spawn failed: {e}");
    }
}

fn play_bytes(bytes: &[u8]) -> anyhow::Result<()> {
    use rodio::{Decoder, OutputStream, Sink};
    let (_stream, handle) = OutputStream::try_default()?;
    let sink = Sink::try_new(&handle)?;
    let cursor = std::io::Cursor::new(bytes.to_vec());
    sink.append(Decoder::new(cursor)?);
    sink.sleep_until_end();
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn bundled_default_sound_decodes() {
        let cursor = std::io::Cursor::new(super::DEFAULT_SOUND.to_vec());
        assert!(
            rodio::Decoder::new(cursor).is_ok(),
            "bundled notify.ogg must decode"
        );
    }
}
