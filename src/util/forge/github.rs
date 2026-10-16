//! GitHub implementation of [`Forge`]: `github.com` (API on `api.github.com`)
//! and, when named explicitly, GitHub Enterprise hosts (API on `/api/v3`).

use anyhow::{Result, bail};
use serde_json::Value;

use super::{Forge, ReleaseAsset, ReleaseInfo, RepoRef, api_headers, host_of, path_segments};
use crate::util::http::HttpClient;

pub struct GitHub;

const ACCEPT: &str = "application/vnd.github+json";

fn api_base(host: &str) -> String {
    if host == "github.com" {
        "https://api.github.com".to_string()
    } else {
        format!("https://{host}/api/v3")
    }
}

impl Forge for GitHub {
    fn name(&self) -> &'static str {
        "github"
    }

    fn matches_host(&self, host: &str) -> bool {
        host == "github.com"
    }

    fn parse_url(&self, url: &str) -> Option<RepoRef> {
        let host = host_of(url)?;
        let segments = path_segments(url)?;
        let [namespace, name, kind, rest @ ..] = segments.as_slice() else {
            return None;
        };
        let (tag, from_release_url) = match (*kind, rest) {
            ("releases", ["tag", tag @ ..]) if !tag.is_empty() => (tag.join("/"), true),
            ("tree", tag @ [_, ..]) => (tag.join("/"), false),
            _ => return None,
        };
        Some(RepoRef {
            host: host.to_string(),
            namespace: namespace.to_string(),
            name: name.to_string(),
            tag: Some(tag),
            from_release_url,
        })
    }

    fn token_env(&self) -> &'static str {
        "GITHUB_TOKEN"
    }

    fn fetch_release(&self, repo: &RepoRef, http: &dyn HttpClient) -> Result<Option<ReleaseInfo>> {
        let Some(tag) = &repo.tag else {
            return Ok(None);
        };
        let url = format!(
            "{}/repos/{}/{}/releases/tags/{tag}",
            api_base(&repo.host),
            repo.namespace,
            repo.name
        );
        let Some(json) = http.get_json(&url, &api_headers(self, ACCEPT))? else {
            if repo.from_release_url {
                bail!(
                    "release '{tag}' not found in {}/{} (HTTP 404)",
                    repo.namespace,
                    repo.name
                );
            }
            return Ok(None);
        };

        let text =
            |value: &Value, key: &str| value.get(key).and_then(Value::as_str).map(String::from);
        let assets = json
            .get("assets")
            .and_then(Value::as_array)
            .map(|assets| {
                assets
                    .iter()
                    .filter_map(|asset| {
                        Some(ReleaseAsset {
                            name: text(asset, "name")?,
                            download_url: text(asset, "browser_download_url")?,
                            sha256: text(asset, "digest")
                                .and_then(|d| d.strip_prefix("sha256:").map(String::from)),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(Some(ReleaseInfo {
            tag: text(&json, "tag_name").unwrap_or_else(|| tag.clone()),
            web_url: text(&json, "html_url")
                .unwrap_or_else(|| format!("{}/releases/tag/{tag}", self.repo_url(repo))),
            assets,
        }))
    }

    fn fetch_license(&self, repo: &RepoRef, http: &dyn HttpClient) -> Result<Option<String>> {
        let url = format!(
            "{}/repos/{}/{}",
            api_base(&repo.host),
            repo.namespace,
            repo.name
        );
        let Some(json) = http.get_json(&url, &api_headers(self, ACCEPT))? else {
            bail!(
                "repository {}/{} not found (HTTP 404)",
                repo.namespace,
                repo.name
            );
        };
        Ok(json
            .pointer("/license/spdx_id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && *id != "NOASSERTION")
            .map(String::from))
    }

    fn purl(&self, repo: &RepoRef) -> String {
        let tag = repo.tag.as_deref().unwrap_or_default();
        format!("pkg:github/{}/{}@{tag}", repo.namespace, repo.name)
    }

    fn repo_url(&self, repo: &RepoRef) -> String {
        format!("https://{}/{}/{}", repo.host, repo.namespace, repo.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_release_url() {
        let repo = GitHub
            .parse_url("https://github.com/code-rhapsodie/cyclonelab/releases/tag/v1.1.0")
            .unwrap();
        assert_eq!(repo.namespace, "code-rhapsodie");
        assert_eq!(repo.name, "cyclonelab");
        assert_eq!(repo.tag.as_deref(), Some("v1.1.0"));
        assert!(repo.from_release_url);
    }

    #[test]
    fn parses_a_tree_url() {
        let repo = GitHub
            .parse_url("https://github.com/code-rhapsodie/cyclonelab/tree/v1.1.0")
            .unwrap();
        assert_eq!(repo.tag.as_deref(), Some("v1.1.0"));
        assert!(!repo.from_release_url);
    }

    #[test]
    fn unknown_host_is_not_matched_but_still_parsable() {
        assert!(!GitHub.matches_host("git.example.org"));
        assert!(GitHub.matches_host("github.com"));
        assert!(
            GitHub
                .parse_url("https://git.example.org/a/b/tree/v1")
                .is_some()
        );
    }

    #[test]
    fn incomplete_urls_are_rejected() {
        assert!(GitHub.parse_url("https://github.com/a/b").is_none());
        assert!(
            GitHub
                .parse_url("https://github.com/a/b/releases/tag")
                .is_none()
        );
        assert!(
            GitHub
                .parse_url("https://github.com/a/b/issues/3")
                .is_none()
        );
        assert!(GitHub.parse_url("not a url").is_none());
    }

    #[test]
    fn purl_and_repo_url() {
        let repo = GitHub
            .parse_url("https://github.com/a/b/tree/v1.0.0")
            .unwrap();
        assert_eq!(GitHub.purl(&repo), "pkg:github/a/b@v1.0.0");
        assert_eq!(GitHub.repo_url(&repo), "https://github.com/a/b");
    }
}
