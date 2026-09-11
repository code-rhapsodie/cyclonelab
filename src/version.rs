//! Version information injected at compile time by `build.rs`, shared by the
//! CLI's `--version` output and any component describing this tool.

/// Release version, as `git describe --always --tags` reports it: the
/// nearest tag, plus the number of commits since it and an abbreviated hash
/// when not built exactly on a tag (e.g. `v1.2.0-5-gabc1234`), or the bare
/// abbreviated hash when no tag is reachable. Falls back to the
/// `Cargo.toml` version when git is unavailable.
pub const VERSION: &str = env!("CYCLONELAB_VERSION");

/// Full git commit hash, or `"dev"` when unavailable.
#[allow(dead_code)]
pub const GIT_HASH: &str = env!("CYCLONELAB_GIT_HASH");

/// Build date and time, in ISO 8601 / RFC 3339 (e.g. `2026-09-11T14:32:07+00:00`).
#[allow(dead_code)]
pub const BUILD_DATE: &str = env!("CYCLONELAB_BUILD_DATE");

/// `VERSION` with `GIT_HASH` and `BUILD_DATE`, as shown by `cyclonelab -V`.
pub const FULL: &str = concat!(
    env!("CYCLONELAB_VERSION"),
    " (",
    env!("CYCLONELAB_GIT_HASH"),
    ", ",
    env!("CYCLONELAB_BUILD_DATE"),
    ")"
);
