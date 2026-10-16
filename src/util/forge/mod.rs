//! Forge abstraction (GitHub today; GitLab, Gitea, ... later) used by the
//! `collect-vcs` action (see `doc/transform/action-collect-vcs.md`). The
//! action only manipulates the neutral types below; everything that depends on
//! a forge's URL scheme or API lives behind [`Forge`].
//!
//! Adding a forge = a new `<forge>.rs` implementing [`Forge`] + one line in
//! [`registry`].

pub mod github;

use anyhow::{Result, bail};

use crate::util::http::HttpClient;

/// Identifies a repository, independently of the forge hosting it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRef {
    pub host: String,
    pub namespace: String,
    pub name: String,
    pub tag: Option<String>,
    /// `true` when the URL pointed at a release page (as opposed to a bare
    /// tag): a missing release is then an error rather than "no release".
    pub from_release_url: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub tag: String,
    pub web_url: String,
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    pub download_url: String,
    pub sha256: Option<String>,
}

pub trait Forge {
    /// Human-readable forge name, used in error messages ("github", ...).
    fn name(&self) -> &'static str;
    /// Whether `host` is one of this forge's public hosts (e.g. `github.com`);
    /// used only when the step has no `forge`.
    fn matches_host(&self, host: &str) -> bool;
    /// Parses the path of `url` into a repository, whatever its host (so
    /// self-hosted instances work when the step names the forge); `None` if
    /// the path does not have this forge's shape.
    fn parse_url(&self, url: &str) -> Option<RepoRef>;
    /// Environment variable holding the optional API token.
    fn token_env(&self) -> &'static str;
    /// Release of `repo.tag`, or `None` if the tag has no release.
    fn fetch_release(&self, repo: &RepoRef, http: &dyn HttpClient) -> Result<Option<ReleaseInfo>>;
    /// SPDX id of the repository license, or `None` if undeclared or not
    /// identifiable.
    fn fetch_license(&self, repo: &RepoRef, http: &dyn HttpClient) -> Result<Option<String>>;
    /// Package URL of `repo`'s tag; forges without an official purl type
    /// return `pkg:generic/...`.
    fn purl(&self, repo: &RepoRef) -> String;
    /// Public repository URL (for the `vcs` external reference).
    fn repo_url(&self, repo: &RepoRef) -> String;
}

/// Every registered forge.
pub fn registry() -> Vec<Box<dyn Forge>> {
    vec![Box::new(github::GitHub)]
}

/// Names accepted by a step's `forge` field.
pub fn names() -> Vec<&'static str> {
    registry().iter().map(|forge| forge.name()).collect()
}

/// Picks the forge for `url` (the one named by `forge`, else the first whose
/// `matches_host` accepts the URL's host) and parses `url` with it.
pub fn resolve(url: &str, forge: Option<&str>) -> Result<(Box<dyn Forge>, RepoRef)> {
    resolve_in(registry(), url, forge)
}

/// [`resolve`] against an explicit registry (lets tests register a fake forge).
pub fn resolve_in(
    forges: Vec<Box<dyn Forge>>,
    url: &str,
    forge: Option<&str>,
) -> Result<(Box<dyn Forge>, RepoRef)> {
    let chosen = match forge {
        Some(name) => {
            let known: Vec<&str> = forges.iter().map(|f| f.name()).collect();
            match forges.into_iter().find(|f| f.name() == name) {
                Some(found) => found,
                None => bail!(
                    "unsupported forge '{name}' (accepted values: {})",
                    known.join(", ")
                ),
            }
        }
        None => {
            let Some(host) = host_of(url) else {
                bail!("malformed URL '{url}'");
            };
            match forges.into_iter().find(|f| f.matches_host(host)) {
                Some(found) => found,
                None => bail!(
                    "unsupported forge: no registered forge recognizes host '{host}' (use the 'forge' field for a self-hosted instance)"
                ),
            }
        }
    };
    match chosen.parse_url(url) {
        Some(repo) => Ok((chosen, repo)),
        None => bail!(
            "URL '{url}' does not have the shape expected by forge '{}'",
            chosen.name()
        ),
    }
}

/// Host part of an `http(s)` URL.
pub fn host_of(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = rest.split(['/', '?', '#']).next()?;
    (!host.is_empty()).then_some(host)
}

/// Path segments of an `http(s)` URL, query and fragment dropped.
pub fn path_segments(url: &str) -> Option<Vec<&str>> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let (_, path) = rest.split_once('/')?;
    let path = path.split(['?', '#']).next()?;
    Some(path.split('/').filter(|s| !s.is_empty()).collect())
}

/// Headers sent with every API call: JSON, a user agent, and the token read
/// from `forge.token_env()` when defined (never from the YAML file).
pub fn api_headers(forge: &dyn Forge, accept: &str) -> Vec<(String, String)> {
    let mut headers = vec![
        ("Accept".to_string(), accept.to_string()),
        (
            "User-Agent".to_string(),
            format!("cyclonelab/{}", crate::version::VERSION),
        ),
    ];
    if let Ok(token) = std::env::var(forge.token_env())
        && !token.is_empty()
    {
        headers.push(("Authorization".to_string(), format!("Bearer {token}")));
    }
    headers
}

#[cfg(test)]
pub(crate) mod testing {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::*;

    /// Fake [`HttpClient`] serving recorded responses by URL and recording
    /// every URL requested.
    #[derive(Default)]
    pub struct FakeHttp {
        pub responses: HashMap<String, serde_json::Value>,
        pub requested: RefCell<Vec<String>>,
    }

    impl HttpClient for FakeHttp {
        fn get_json(
            &self,
            url: &str,
            _headers: &[(String, String)],
        ) -> Result<Option<serde_json::Value>> {
            self.requested.borrow_mut().push(url.to_string());
            Ok(self.responses.get(url).cloned())
        }
    }

    pub struct FakeForge;

    impl Forge for FakeForge {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn matches_host(&self, host: &str) -> bool {
            host == "fake.example"
        }
        fn parse_url(&self, url: &str) -> Option<RepoRef> {
            let seg = path_segments(url)?;
            Some(RepoRef {
                host: host_of(url)?.to_string(),
                namespace: seg.first()?.to_string(),
                name: seg.get(1)?.to_string(),
                tag: seg.get(2).map(|s| s.to_string()),
                from_release_url: false,
            })
        }
        fn token_env(&self) -> &'static str {
            "FAKE_TOKEN"
        }
        fn fetch_release(&self, _: &RepoRef, _: &dyn HttpClient) -> Result<Option<ReleaseInfo>> {
            Ok(None)
        }
        fn fetch_license(&self, _: &RepoRef, _: &dyn HttpClient) -> Result<Option<String>> {
            Ok(None)
        }
        fn purl(&self, repo: &RepoRef) -> String {
            format!("pkg:generic/{}/{}", repo.namespace, repo.name)
        }
        fn repo_url(&self, repo: &RepoRef) -> String {
            format!("https://{}/{}/{}", repo.host, repo.namespace, repo.name)
        }
    }

    #[test]
    fn a_registered_fake_forge_is_chosen_by_its_host() {
        let forges: Vec<Box<dyn Forge>> = vec![Box::new(github::GitHub), Box::new(FakeForge)];
        let (forge, repo) = resolve_in(forges, "https://fake.example/a/b/v1", None).unwrap();
        assert_eq!(forge.name(), "fake");
        assert_eq!(repo.name, "b");
    }

    #[test]
    fn unknown_host_without_forge_is_unsupported() {
        let err = resolve("https://git.example.org/a/b/releases/tag/v1", None)
            .err()
            .unwrap();
        assert!(err.to_string().contains("unsupported forge"));
    }

    #[test]
    fn named_forge_bypasses_host_detection() {
        let (forge, repo) = resolve(
            "https://git.example.org/team/app/releases/tag/v1.0.0",
            Some("github"),
        )
        .unwrap();
        assert_eq!(forge.name(), "github");
        assert_eq!(repo.host, "git.example.org");
        assert_eq!(repo.tag.as_deref(), Some("v1.0.0"));
    }

    #[test]
    fn unknown_forge_name_lists_accepted_values() {
        let err = resolve("https://github.com/a/b/tree/v1", Some("gitlab"))
            .err()
            .unwrap();
        let message = err.to_string();
        assert!(message.contains("unsupported forge") && message.contains("github"));
    }

    #[test]
    fn named_forge_failing_to_parse_is_an_error() {
        assert!(resolve("https://github.com/a", Some("github")).is_err());
    }
}
