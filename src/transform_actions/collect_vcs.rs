//! `collect-vcs` action: describes the component published at a forge
//! release/tag URL and writes it to `target` (see
//! `doc/transform/action-collect-vcs.md`). Knows no forge in particular: only
//! the neutral types of `util::forge`.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::util::forge::{self, Forge, RepoRef};
use crate::util::http::{HttpClient, UreqClient};
use crate::util::jsonpath;

use super::{Action, StepContext};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectVcsStep {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forge: Option<String>,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bom_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purl: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_cdx: Option<String>,
    #[serde(default)]
    pub license: bool,
}

/// Splits a trailing `[]` (append form) off `target`.
fn split_target(target: &str) -> (&str, bool) {
    match target.strip_suffix("[]") {
        Some(prefix) => (prefix, true),
        None => (target, false),
    }
}

impl CollectVcsStep {
    /// Static checks performable without network or document: `target`
    /// syntax and `forge` against the registered forge names — used by `lint`.
    pub(crate) fn lint(&self) -> Result<()> {
        let (target, _) = split_target(&self.target);
        jsonpath::literal(target).with_context(|| format!("invalid target '{}'", self.target))?;

        if let Some(name) = &self.forge
            && !name.contains("{$")
        {
            let known = forge::names();
            if !known.contains(&name.as_str()) {
                bail!(
                    "unsupported forge '{name}' (accepted values: {})",
                    known.join(", ")
                );
            }
        }
        Ok(())
    }

    /// The network-free core of [`Action::apply`]: everything forge-related
    /// goes through `forge` and `http`.
    fn collect(
        &self,
        doc: &mut Value,
        ctx: &StepContext,
        forge: &dyn Forge,
        repo: &RepoRef,
        http: &dyn HttpClient,
    ) -> Result<()> {
        let release = forge.fetch_release(repo, http)?;

        let tag = repo.tag.clone().unwrap_or_default();
        let purl = self.purl.clone().unwrap_or_else(|| forge.purl(repo));
        let repo_url = forge.repo_url(repo);

        let mut references = vec![json!({"type": "vcs", "url": repo_url})];
        if let Some(release) = &release {
            references.push(json!({"type": "distribution", "url": release.web_url}));
        } else if !tag.is_empty() {
            references.push(json!({"type": "distribution", "url": self.url}));
        }

        if let Some(asset_name) = &self.artifact_cdx {
            let asset = release
                .as_ref()
                .and_then(|r| r.assets.iter().find(|a| &a.name == asset_name))
                .with_context(|| {
                    format!(
                        "asset '{asset_name}' not found in the release of '{}'",
                        self.url
                    )
                })?;
            let mut reference = json!({"type": "bom", "url": asset.download_url});
            if let Some(sha256) = &asset.sha256 {
                reference["hashes"] = json!([{"alg": "SHA-256", "content": sha256}]);
            }
            references.push(reference);
        }

        let (target, append) = split_target(&self.target);
        let path = jsonpath::literal(target)
            .with_context(|| format!("invalid target '{}'", self.target))?;
        let existing: Vec<Value> = match jsonpath::get(doc, &path) {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(items)) => items.clone(),
            Some(_) if append => bail!(
                "target '{}' ends with '[]' but the existing value is not an array",
                self.target
            ),
            Some(_) => Vec::new(),
        };

        let bom_ref = match &self.bom_ref {
            Some(bom_ref) => bom_ref.clone(),
            None => existing
                .iter()
                .find(|c| c.get("purl").and_then(Value::as_str) == Some(purl.as_str()))
                .and_then(|c| c.get("bom-ref").and_then(Value::as_str))
                .map(String::from)
                .unwrap_or_else(|| purl.clone()),
        };

        let mut component = Map::new();
        component.insert("type".into(), json!("library"));
        component.insert("bom-ref".into(), json!(bom_ref));
        component.insert("name".into(), json!(repo.name));
        component.insert("group".into(), json!(repo.namespace));
        if !tag.is_empty() {
            let version = tag.strip_prefix('v').unwrap_or(&tag);
            component.insert("version".into(), json!(version));
        }
        component.insert("purl".into(), json!(purl));

        if self.license {
            match forge.fetch_license(repo, http)? {
                Some(id) => {
                    component.insert(
                        "licenses".into(),
                        json!([{"license": {"id": id, "acknowledgement": "declared"}}]),
                    );
                }
                None => eprintln!(
                    "warning: step '{}': no identifiable license declared by the forge for {}/{}; 'licenses' not written",
                    ctx.step_id, repo.namespace, repo.name
                ),
            }
        }
        component.insert("externalReferences".into(), Value::Array(references));
        let component = Value::Object(component);

        let written = if append {
            let mut items = existing;
            let same_purl = items
                .iter()
                .position(|c| c.get("purl").and_then(Value::as_str) == Some(purl.as_str()));
            match same_purl {
                Some(index) => items[index] = component,
                None => items.push(component),
            }
            Value::Array(items)
        } else {
            Value::Array(vec![component])
        };

        jsonpath::set(doc, &path, written)
            .with_context(|| format!("unable to write to '{}'", self.target))
    }
}

impl Action for CollectVcsStep {
    fn apply(&self, doc: &mut Value, ctx: &StepContext) -> Result<()> {
        let mut run = || -> Result<()> {
            let (forge, repo) = forge::resolve(&self.url, self.forge.as_deref())?;
            self.collect(doc, ctx, forge.as_ref(), &repo, &UreqClient)
        };
        run().with_context(|| format!("step '{}'", ctx.step_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cyclonedx::validation::validate_bom;
    use crate::util::forge::github::GitHub;
    use crate::util::forge::testing::FakeHttp;

    const RELEASE_URL: &str = "https://github.com/code-rhapsodie/cyclonelab/releases/tag/v1.1.0";
    const API_RELEASE: &str =
        "https://api.github.com/repos/code-rhapsodie/cyclonelab/releases/tags/v1.1.0";
    const API_REPO: &str = "https://api.github.com/repos/code-rhapsodie/cyclonelab";

    fn ctx() -> StepContext {
        StepContext {
            base_dir: ".".into(),
            step_id: "collect".to_string(),
            description: None,
        }
    }

    fn http(license: Value) -> FakeHttp {
        let mut http = FakeHttp::default();
        http.responses.insert(
            API_RELEASE.into(),
            json!({
                "tag_name": "v1.1.0",
                "html_url": RELEASE_URL,
                "assets": [{
                    "name": "cyclonelab-sbom.json",
                    "browser_download_url": "https://github.com/code-rhapsodie/cyclonelab/releases/download/v1.1.0/cyclonelab-sbom.json",
                    "digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                }]
            }),
        );
        http.responses
            .insert(API_REPO.into(), json!({"license": license}));
        http
    }

    fn step(extra: Value) -> CollectVcsStep {
        let mut base =
            json!({"url": RELEASE_URL, "target": "$.metadata.component.pedigree.ancestors[]"});
        base.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(base).unwrap()
    }

    fn run(step: &CollectVcsStep, doc: &mut Value, http: &FakeHttp) -> Result<()> {
        let (forge, repo) = forge::resolve(&step.url, step.forge.as_deref())?;
        step.collect(doc, &ctx(), forge.as_ref(), &repo, http)
    }

    fn ancestors(doc: &Value) -> &Vec<Value> {
        doc["metadata"]["component"]["pedigree"]["ancestors"]
            .as_array()
            .unwrap()
    }

    #[test]
    fn builds_the_component_with_bom_ref_artifact_and_license() {
        let step = step(json!({
            "bom_ref": "pkg:github/code-rhapsodie/cyclonelab@1.1.0",
            "artifact_cdx": "cyclonelab-sbom.json",
            "license": true,
        }));
        let mut doc = json!({});
        run(&step, &mut doc, &http(json!({"spdx_id": "EUPL-1.2"}))).unwrap();
        assert_eq!(
            ancestors(&doc)[0],
            json!({
                "type": "library",
                "bom-ref": "pkg:github/code-rhapsodie/cyclonelab@1.1.0",
                "name": "cyclonelab",
                "group": "code-rhapsodie",
                "version": "1.1.0",
                "purl": "pkg:github/code-rhapsodie/cyclonelab@v1.1.0",
                "licenses": [{"license": {"id": "EUPL-1.2", "acknowledgement": "declared"}}],
                "externalReferences": [
                    {"type": "vcs", "url": "https://github.com/code-rhapsodie/cyclonelab"},
                    {"type": "distribution", "url": RELEASE_URL},
                    {
                        "type": "bom",
                        "url": "https://github.com/code-rhapsodie/cyclonelab/releases/download/v1.1.0/cyclonelab-sbom.json",
                        "hashes": [{"alg": "SHA-256", "content": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]
                    }
                ]
            })
        );
    }

    #[test]
    fn a_forced_purl_replaces_the_forge_one() {
        let mut doc = json!({});
        run(
            &step(json!({"purl": "pkg:composer/code-rhapsodie/cyclonelab@1.1.0"})),
            &mut doc,
            &http(Value::Null),
        )
        .unwrap();
        let component = &ancestors(&doc)[0];
        assert_eq!(
            component["purl"],
            "pkg:composer/code-rhapsodie/cyclonelab@1.1.0"
        );
        assert_eq!(component["bom-ref"], component["purl"]);
    }

    #[test]
    fn a_forced_purl_is_used_to_find_the_existing_component() {
        let mut doc = json!({"metadata": {"component": {"pedigree": {"ancestors": [
            {"purl": "pkg:composer/a/b@1", "bom-ref": "keep-me", "name": "old"}
        ]}}}});
        run(
            &step(json!({"purl": "pkg:composer/a/b@1"})),
            &mut doc,
            &http(Value::Null),
        )
        .unwrap();
        let items = ancestors(&doc);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["bom-ref"], "keep-me");
        assert_eq!(items[0]["purl"], "pkg:composer/a/b@1");
    }

    #[test]
    fn without_bom_ref_the_purl_is_used() {
        let mut doc = json!({});
        run(&step(json!({})), &mut doc, &http(Value::Null)).unwrap();
        let component = &ancestors(&doc)[0];
        assert_eq!(component["bom-ref"], component["purl"]);
        assert!(component.get("licenses").is_none());
        assert_eq!(component["externalReferences"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn an_existing_bom_ref_is_kept_and_the_component_replaced_not_duplicated() {
        let mut doc = json!({"metadata": {"component": {"pedigree": {"ancestors": [
            {"purl": "pkg:github/code-rhapsodie/cyclonelab@v1.1.0", "bom-ref": "keep-me", "name": "old"}
        ]}}}});
        run(&step(json!({})), &mut doc, &http(Value::Null)).unwrap();
        assert_eq!(ancestors(&doc).len(), 1);
        assert_eq!(ancestors(&doc)[0]["bom-ref"], "keep-me");
        assert_eq!(ancestors(&doc)[0]["name"], "cyclonelab");
    }

    #[test]
    fn an_explicit_bom_ref_overrides_an_existing_one() {
        let mut doc = json!({"metadata": {"component": {"pedigree": {"ancestors": [
            {"purl": "pkg:github/code-rhapsodie/cyclonelab@v1.1.0", "bom-ref": "old"}
        ]}}}});
        run(
            &step(json!({"bom_ref": "new"})),
            &mut doc,
            &http(Value::Null),
        )
        .unwrap();
        assert_eq!(ancestors(&doc)[0]["bom-ref"], "new");
    }

    #[test]
    fn append_keeps_other_components() {
        let mut doc = json!({"metadata": {"component": {"pedigree": {"ancestors": [{"name": "other", "purl": "pkg:x"}]}}}});
        run(&step(json!({})), &mut doc, &http(Value::Null)).unwrap();
        assert_eq!(ancestors(&doc).len(), 2);
    }

    #[test]
    fn target_without_brackets_replaces_the_array() {
        let step = step(json!({"target": "$.metadata.component.pedigree.ancestors"}));
        let mut doc =
            json!({"metadata": {"component": {"pedigree": {"ancestors": [{"name": "other"}]}}}});
        run(&step, &mut doc, &http(Value::Null)).unwrap();
        assert_eq!(ancestors(&doc).len(), 1);
        assert_eq!(ancestors(&doc)[0]["name"], "cyclonelab");
    }

    #[test]
    fn append_to_a_non_array_is_an_error() {
        let mut doc = json!({"metadata": {"component": {"pedigree": {"ancestors": "x"}}}});
        assert!(run(&step(json!({})), &mut doc, &http(Value::Null)).is_err());
    }

    #[test]
    fn missing_artifact_is_an_error() {
        let mut doc = json!({});
        let err = run(
            &step(json!({"artifact_cdx": "nope.json"})),
            &mut doc,
            &http(Value::Null),
        )
        .unwrap_err();
        assert!(err.to_string().contains("nope.json"));
    }

    #[test]
    fn tree_url_without_release_builds_from_the_url_alone() {
        let step = step(json!({"url": "https://github.com/code-rhapsodie/cyclonelab/tree/v1.1.0"}));
        let mut doc = json!({});
        run(&step, &mut doc, &FakeHttp::default()).unwrap();
        assert_eq!(ancestors(&doc)[0]["version"], "1.1.0");
    }

    #[test]
    fn missing_release_on_a_release_url_is_an_error() {
        let mut doc = json!({});
        assert!(run(&step(json!({})), &mut doc, &FakeHttp::default()).is_err());
    }

    #[test]
    fn noassertion_license_is_not_written() {
        let mut doc = json!({});
        run(
            &step(json!({"license": true})),
            &mut doc,
            &http(json!({"spdx_id": "NOASSERTION"})),
        )
        .unwrap();
        assert!(ancestors(&doc)[0].get("licenses").is_none());
    }

    #[test]
    fn license_omitted_makes_no_license_call() {
        let http = http(json!({"spdx_id": "MIT"}));
        let mut doc = json!({});
        run(&step(json!({})), &mut doc, &http).unwrap();
        assert!(!http.requested.borrow().iter().any(|u| u == API_REPO));
    }

    #[test]
    fn github_forge_is_used_directly() {
        let (forge, _) = forge::resolve(RELEASE_URL, None).unwrap();
        assert_eq!(forge.name(), GitHub.name());
    }

    #[test]
    fn lint_rejects_unknown_forge_and_accepts_known() {
        assert!(step(json!({"forge": "gitlab"})).lint().is_err());
        step(json!({"forge": "github"})).lint().unwrap();
        step(json!({"forge": "{$forge}"})).lint().unwrap();
    }

    #[test]
    fn target_is_required() {
        let result: Result<CollectVcsStep, _> = serde_json::from_value(json!({"url": RELEASE_URL}));
        assert!(result.is_err());
    }

    #[test]
    fn lint_rejects_an_invalid_target() {
        assert!(step(json!({"target": "$.a["})).lint().is_err());
    }

    #[test]
    fn document_stays_schema_valid() {
        let mut doc = json!({
            "bomFormat": "CycloneDX",
            "specVersion": "1.6",
            "version": 1,
            "metadata": {"component": {"type": "library", "name": "x", "bom-ref": "x"}}
        });
        let step = step(json!({"artifact_cdx": "cyclonelab-sbom.json", "license": true}));
        run(&step, &mut doc, &http(json!({"spdx_id": "EUPL-1.2"}))).unwrap();
        let outcome = validate_bom(&doc).unwrap();
        assert!(
            outcome.errors.is_empty(),
            "{:?}",
            outcome
                .errors
                .iter()
                .map(|e| &e.message)
                .collect::<Vec<_>>()
        );
    }
}
