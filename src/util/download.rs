use std::fs::{self, File};
use std::io::copy;
use std::path::Path;

use anyhow::{Context, Result};

use crate::util::hashing::sha256_file;

/// Downloads `url` to `dest`.
pub fn download_file(url: &str, dest: &Path) -> Result<()> {
    let response = ureq::get(url)
        .call()
        .with_context(|| format!("Failed to download '{url}'"))?;

    let mut reader = response.into_reader();
    let mut file =
        File::create(dest).with_context(|| format!("Unable to create '{}'", dest.display()))?;
    copy(&mut reader, &mut file)
        .with_context(|| format!("Failed to write '{}'", dest.display()))?;

    Ok(())
}

/// Downloads `url` to `dest`, hashes the downloaded file with SHA-256, then
/// deletes `dest` — used by `add`'s `hash` generator (`valueFrom.url`).
pub fn download_and_hash_sha256(url: &str, dest: &Path) -> Result<String> {
    download_file(url, dest)?;
    let hash = sha256_file(dest);
    let _ = fs::remove_file(dest);
    hash
}
