//! `collect-commits` action: lists the commits of a local Git repository
//! that touched a path, and writes them as CycloneDX `commit` objects at
//! `target` (see `doc/transform/action-collect-commits.md`).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::util::jsonpath;

use super::{Action, StepContext};

const FIELD_SEP: char = '\x1f';
const RECORD_SEP: char = '\x1e';
/// `git log` format: commit hash, then author and committer
/// (name, email, strict ISO 8601 date), then the raw message body.
const LOG_FORMAT: &str = "%H%x1f%an%x1f%ae%x1f%aI%x1f%cn%x1f%ce%x1f%cI%x1f%B%x1e";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectCommitsStep {
    /// File or directory of the Git repository whose history is wanted,
    /// resolved from the transformation file's directory.
    pub path: PathBuf,
    /// Commits made after this one (excluded) up to `HEAD` are listed; the
    /// whole history of `path` when omitted.
    #[serde(default)]
    pub from_commit: Option<String>,
    /// JSONPath of the array receiving the commits (no `[]` suffix).
    pub target: String,
    /// Empties the existing array first when `true`; appends otherwise.
    #[serde(default)]
    pub r#override: bool,
}

impl CollectCommitsStep {
    /// Static checks performable without a document or Git: `target`'s
    /// JSONPath syntax — used by `lint`.
    pub(crate) fn lint(&self) -> Result<()> {
        jsonpath::literal(&self.target)
            .map(|_| ())
            .with_context(|| format!("invalid target '{}'", self.target))
    }
}

impl Action for CollectCommitsStep {
    fn apply(&self, doc: &mut Value, ctx: &StepContext) -> Result<()> {
        let target = jsonpath::literal(&self.target)
            .with_context(|| format!("step '{}': invalid target '{}'", ctx.step_id, self.target))?;

        let existing = match jsonpath::get(doc, &target) {
            Some(Value::Array(items)) => items.clone(),
            Some(_) => bail!(
                "step '{}': target '{}' exists but is not an array",
                ctx.step_id,
                self.target
            ),
            None => Vec::new(),
        };

        let resolved = ctx.base_dir.join(&self.path);
        let commits = self
            .collect(&resolved)
            .with_context(|| format!("step '{}'", ctx.step_id))?;

        if self.from_commit.is_none() {
            eprintln!(
                "warning: step '{}': no 'from_commit' set, the whole history of '{}' is listed (this may be very long)",
                ctx.step_id,
                self.path.display()
            );
        }
        if commits.is_empty() {
            eprintln!(
                "warning: step '{}': no commit found for '{}'",
                ctx.step_id,
                self.path.display()
            );
        }

        let mut merged = if self.r#override {
            Vec::new()
        } else {
            existing
        };
        for commit in commits {
            let uid = commit.get("uid");
            if !merged.iter().any(|item| item.get("uid") == uid) {
                merged.push(commit);
            }
        }

        jsonpath::set(doc, &target, Value::Array(merged)).with_context(|| {
            format!(
                "step '{}': unable to write to '{}'",
                ctx.step_id, self.target
            )
        })
    }
}

impl CollectCommitsStep {
    fn collect(&self, path: &Path) -> Result<Vec<Value>> {
        let path = path
            .canonicalize()
            .with_context(|| format!("path '{}' does not exist", path.display()))?;
        let dir = if path.is_dir() {
            path.clone()
        } else {
            path.parent()
                .map(Path::to_path_buf)
                .with_context(|| format!("'{}' has no parent directory", path.display()))?
        };

        let toplevel = git(&dir, &["rev-parse", "--show-toplevel"])
            .with_context(|| format!("'{}' is not inside a Git repository", path.display()))?;
        let repo = PathBuf::from(toplevel.trim());

        let range = match &self.from_commit {
            Some(from) => {
                if from.is_empty() || from.starts_with('-') {
                    bail!("unknown commit '{from}'");
                }
                let spec = format!("{from}^{{commit}}");
                git(&repo, &["rev-parse", "--verify", "--quiet", &spec])
                    .map_err(|_| anyhow::anyhow!("unknown commit '{from}'"))?;
                Some(format!("{from}..HEAD"))
            }
            None => None,
        };

        let format_arg = format!("--format={LOG_FORMAT}");
        let mut args = vec!["log", &format_arg];
        if let Some(range) = &range {
            args.push(range);
        }
        args.push("--");
        let path_arg = path.to_string_lossy();
        args.push(&path_arg);

        let output = git(&repo, &args)?;
        let base_url = remote_base_url(&repo);

        output
            .split(RECORD_SEP)
            .map(|record| record.trim_start_matches('\n'))
            .filter(|record| !record.is_empty())
            .map(|record| parse_commit(record, base_url.as_deref()))
            .collect()
    }
}

fn parse_commit(record: &str, base_url: Option<&str>) -> Result<Value> {
    let fields: Vec<&str> = record.splitn(8, FIELD_SEP).collect();
    let [hash, an, ae, ad, cn, ce, cd, message] = fields[..] else {
        bail!("unexpected `git log` output: '{record}'");
    };

    let mut commit = Map::new();
    commit.insert("uid".into(), json!(hash));
    if let Some(base) = base_url {
        commit.insert("url".into(), json!(format!("{base}/commit/{hash}")));
    }
    commit.insert(
        "author".into(),
        json!({"timestamp": ad, "name": an, "email": ae}),
    );
    commit.insert(
        "committer".into(),
        json!({"timestamp": cd, "name": cn, "email": ce}),
    );
    commit.insert("message".into(), json!(message.trim_end()));
    Ok(Value::Object(commit))
}

/// Runs `git` in `dir`, returning its stdout. Fails if `git` is missing or
/// exits with an error.
fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .context("unable to run 'git' (is it installed and in the PATH?)")?;
    if !output.status.success() {
        bail!(
            "`git {}` failed: {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Web base URL (`https://host/owner/repo`) of the `origin` remote when it
/// is a GitHub or GitLab URL (HTTPS or SSH); `None` otherwise.
fn remote_base_url(repo: &Path) -> Option<String> {
    let url = git(repo, &["config", "--get", "remote.origin.url"]).ok()?;
    web_base_url(url.trim())
}

fn web_base_url(remote: &str) -> Option<String> {
    let (host, path) = if let Some((_, rest)) = remote.split_once("://") {
        let rest = rest.split_once('@').map_or(rest, |(_, after)| after);
        let (host, path) = rest.split_once('/')?;
        (host.split(':').next()?, path)
    } else {
        // scp-like syntax: git@github.com:owner/repo.git
        let rest = remote.split_once('@').map_or(remote, |(_, after)| after);
        rest.split_once(':')?
    };

    if !(host.contains("github") || host.contains("gitlab")) {
        return None;
    }
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if path.is_empty() {
        return None;
    }
    Some(format!("https://{host}/{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn run_git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    /// Repo with commits: c1 (a.txt), c2 (b.txt), c3 (a.txt). Returns the
    /// directory and the hashes [c1, c2, c3].
    fn repo() -> (TempDir, Vec<String>) {
        let dir = TempDir::new().unwrap();
        run_git(dir.path(), &["init", "-q"]);
        let mut hashes = Vec::new();
        for (file, content, msg) in [
            ("a.txt", "1", "first"),
            ("b.txt", "1", "second"),
            ("a.txt", "2", "third"),
        ] {
            fs::write(dir.path().join(file), content).unwrap();
            run_git(dir.path(), &["add", "."]);
            run_git(dir.path(), &["commit", "-q", "-m", msg]);
            hashes.push(
                git(dir.path(), &["rev-parse", "HEAD"])
                    .unwrap()
                    .trim()
                    .to_string(),
            );
        }
        (dir, hashes)
    }

    fn ctx(dir: &Path) -> StepContext {
        StepContext {
            base_dir: dir.to_path_buf(),
            step_id: "collect".into(),
            description: None,
        }
    }

    fn step(value: Value) -> CollectCommitsStep {
        serde_json::from_value(value).unwrap()
    }

    fn uids(doc: &Value) -> Vec<String> {
        doc["pedigree"]["commits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["uid"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn lists_commits_for_a_path_most_recent_first() {
        let (dir, h) = repo();
        let s = step(json!({"path": "a.txt", "target": "$.pedigree.commits"}));
        let mut doc = json!({});
        s.apply(&mut doc, &ctx(dir.path())).unwrap();
        assert_eq!(uids(&doc), vec![h[2].clone(), h[0].clone()]);

        let commit = &doc["pedigree"]["commits"][0];
        assert_eq!(commit["message"], "third");
        assert_eq!(commit["author"]["name"], "Test");
        assert_eq!(commit["author"]["email"], "test@example.com");
        assert!(commit["committer"]["timestamp"].is_string());
        assert!(commit.get("url").is_none());
    }

    #[test]
    fn excludes_from_commit() {
        let (dir, h) = repo();
        let s = step(json!({
            "path": ".", "from_commit": h[0], "target": "$.pedigree.commits"
        }));
        let mut doc = json!({});
        s.apply(&mut doc, &ctx(dir.path())).unwrap();
        assert_eq!(uids(&doc), vec![h[2].clone(), h[1].clone()]);
    }

    #[test]
    fn override_replaces_and_default_appends_without_duplicates() {
        let (dir, h) = repo();
        let mut doc = json!({"pedigree": {"commits": [{"uid": "old"}]}});

        let append = step(json!({"path": "a.txt", "target": "$.pedigree.commits"}));
        append.apply(&mut doc, &ctx(dir.path())).unwrap();
        append.apply(&mut doc, &ctx(dir.path())).unwrap();
        assert_eq!(
            uids(&doc),
            vec!["old".to_string(), h[2].clone(), h[0].clone()]
        );

        let replace = step(json!({
            "path": "a.txt", "target": "$.pedigree.commits", "override": true
        }));
        replace.apply(&mut doc, &ctx(dir.path())).unwrap();
        assert_eq!(uids(&doc), vec![h[2].clone(), h[0].clone()]);
    }

    #[test]
    fn no_commit_after_from_commit_creates_empty_array() {
        let (dir, h) = repo();
        let s = step(json!({
            "path": ".", "from_commit": h[2], "target": "$.pedigree.commits", "override": true
        }));
        let mut doc = json!({"pedigree": {"commits": [{"uid": "old"}]}});
        s.apply(&mut doc, &ctx(dir.path())).unwrap();
        assert_eq!(doc["pedigree"]["commits"], json!([]));
    }

    #[test]
    fn missing_target_is_rejected_at_deserialization() {
        let err = serde_json::from_value::<CollectCommitsStep>(json!({"path": "."})).unwrap_err();
        assert!(err.to_string().contains("target"));
    }

    #[test]
    fn unknown_or_empty_from_commit_is_an_error() {
        let (dir, _) = repo();
        for from in ["deadbeef", ""] {
            let s = step(json!({"path": ".", "from_commit": from, "target": "$.c"}));
            let err = s.apply(&mut json!({}), &ctx(dir.path())).unwrap_err();
            assert!(format!("{err:#}").contains("unknown commit"), "{err:#}");
        }
    }

    #[test]
    fn path_outside_a_repository_or_missing_is_an_error() {
        let dir = TempDir::new().unwrap();
        let s = step(json!({"path": ".", "target": "$.c"}));
        assert!(s.apply(&mut json!({}), &ctx(dir.path())).is_err());

        let s = step(json!({"path": "nope", "target": "$.c"}));
        let err = s.apply(&mut json!({}), &ctx(dir.path())).unwrap_err();
        assert!(format!("{err:#}").contains("does not exist"));
    }

    #[test]
    fn non_array_target_is_an_error() {
        let (dir, _) = repo();
        let s = step(json!({"path": ".", "target": "$.c"}));
        let err = s
            .apply(&mut json!({"c": "x"}), &ctx(dir.path()))
            .unwrap_err();
        assert!(err.to_string().contains("not an array"));
    }

    #[test]
    fn url_is_built_from_a_github_origin() {
        let (dir, h) = repo();
        run_git(
            dir.path(),
            &["remote", "add", "origin", "git@github.com:acme/widget.git"],
        );
        let s = step(json!({"path": "b.txt", "target": "$.pedigree.commits"}));
        let mut doc = json!({});
        s.apply(&mut doc, &ctx(dir.path())).unwrap();
        assert_eq!(
            doc["pedigree"]["commits"][0]["url"],
            format!("https://github.com/acme/widget/commit/{}", h[1])
        );
    }

    #[test]
    fn web_base_url_handles_supported_remotes() {
        assert_eq!(
            web_base_url("https://github.com/o/r.git").as_deref(),
            Some("https://github.com/o/r")
        );
        assert_eq!(
            web_base_url("ssh://git@gitlab.com/g/s/r.git").as_deref(),
            Some("https://gitlab.com/g/s/r")
        );
        assert_eq!(
            web_base_url("git@gitlab.example.org:g/r").as_deref(),
            Some("https://gitlab.example.org/g/r")
        );
        assert_eq!(web_base_url("https://example.com/o/r.git"), None);
    }

    #[test]
    fn lint_checks_target_syntax() {
        assert!(step(json!({"path": ".", "target": "$.a.b"})).lint().is_ok());
        assert!(
            step(json!({"path": ".", "target": "$.a[*]"}))
                .lint()
                .is_err()
        );
    }
}
