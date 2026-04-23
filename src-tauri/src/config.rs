use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub fn config_dir() -> Result<PathBuf> {
    let base = dirs::config_dir().context("no XDG config dir")?;
    let dir = base.join("livestreamlist");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating config dir {}", dir.display()))?;
    Ok(dir)
}

pub fn channels_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("channels.json"))
}

pub fn settings_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("settings.json"))
}

pub fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents)
        .with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}
