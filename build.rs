use std::process::Command;

use chrono::SecondsFormat;

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");

    let version = Command::new("git")
        .args(["describe", "--always", "--tags"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|desc| desc.trim().to_owned())
        .filter(|desc| !desc.is_empty())
        .map(|desc| match desc.strip_prefix('v') {
            Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => rest.to_owned(),
            _ => desc,
        })
        .unwrap_or_else(|| std::env::var("CARGO_PKG_VERSION").unwrap());
    println!("cargo:rustc-env=CYCLONELAB_VERSION={version}");

    let git_hash = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|hash| hash.trim().to_owned())
        .filter(|hash| !hash.is_empty())
        .unwrap_or_else(|| "dev".to_owned());
    println!("cargo:rustc-env=CYCLONELAB_GIT_HASH={git_hash}");

    let build_date = chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    println!("cargo:rustc-env=CYCLONELAB_BUILD_DATE={build_date}");
}
