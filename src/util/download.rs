use std::fs::File;
use std::io::copy;
use std::path::Path;

use anyhow::{Context, Result};

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
