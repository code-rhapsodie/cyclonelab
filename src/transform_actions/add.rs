//! `add` action: writes a value at a location in the document, creating
//! missing intermediate objects along the way (see
//! `doc/transform/action-add.md`).

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::util::download::download_and_hash_sha256;
use crate::util::hashing::sha256_file;
use crate::util::jsonpath::{self, JsonType};
use crate::util::template::render_value;

use super::{Action, StepContext};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddStep {
    pub target: String,
    #[serde(default)]
    pub value: Option<Value>,
    #[serde(default, rename = "valueFrom")]
    pub value_from: Option<ValueFrom>,
    /// Guards the step on the JSON type of `target`'s *current* value
    /// (e.g. only touch `$schema` when it is already set, see
    /// `schema/upgrade-1.5-to-1.6.yaml`'s `schema-url` step): a no-op if
    /// `target` is absent or of a different type. Unconditional (creates
    /// or replaces regardless) when omitted.
    #[serde(default)]
    pub when: Option<JsonType>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValueFrom {
    #[serde(default)]
    pub file: Option<PathBuf>,
    #[serde(default)]
    pub generator: Option<Generator>,
    #[serde(default)]
    pub format: Option<String>,
    /// Hash algorithm to use, only for `generator: hash`. Only `sha256` is
    /// currently supported.
    #[serde(default)]
    pub algo: Option<String>,
    /// Path to a file already on disk to hash, only for `generator: hash`.
    /// Resolved the same way as `valueFrom.file` (relative to the
    /// transformation file's directory; an already-absolute path, e.g.
    /// `{$artifact_path}` from `foreach`, is used as-is). Exclusive with
    /// `url`.
    #[serde(default)]
    pub path: Option<PathBuf>,
    /// URL to download and hash, only for `generator: hash`. Exclusive with
    /// `path`.
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Generator {
    Uuid,
    Timestamp,
    Hash,
}

impl Action for AddStep {
    fn apply(&self, doc: &mut Value, ctx: &StepContext) -> Result<()> {
        let targets = self.resolve_targets(doc, ctx)?;

        let targets: Vec<_> = targets
            .into_iter()
            .filter(|path| match self.when {
                None => true,
                Some(when) => {
                    jsonpath::get(doc, path).is_some_and(|v| jsonpath::matches(v, Some(when)))
                }
            })
            .collect();

        if targets.is_empty() {
            return Ok(());
        }

        let computed = self.compute_value(ctx)?;
        for path in &targets {
            jsonpath::set(doc, path, computed.clone()).with_context(|| {
                format!(
                    "step '{}': unable to write to '{}'",
                    ctx.step_id, self.target
                )
            })?;
        }
        Ok(())
    }
}

impl AddStep {
    /// Concrete locations `target` currently designates. A target with a
    /// data-dependent segment (e.g. `[?type==distribution]`, see
    /// `doc/transform/action-add.md#generator-hash`) may resolve to zero,
    /// one, or several existing locations — zero is a no-op, not an error,
    /// consistent with `util::jsonpath`. A plain target (the common case)
    /// always resolves to exactly one location, which — unlike a
    /// data-dependent one — need not already exist: `add` creates missing
    /// intermediate objects along it.
    fn resolve_targets(
        &self,
        doc: &Value,
        ctx: &StepContext,
    ) -> Result<Vec<jsonpath::ConcretePath>> {
        match jsonpath::resolve_add_target(doc, &self.target)
            .with_context(|| format!("step '{}': invalid target '{}'", ctx.step_id, self.target))?
        {
            Some(anchors) => Ok(anchors),
            None => {
                let path = jsonpath::literal(&self.target).with_context(|| {
                    format!("step '{}': invalid target '{}'", ctx.step_id, self.target)
                })?;
                Ok(vec![path])
            }
        }
    }

    fn compute_value(&self, ctx: &StepContext) -> Result<Value> {
        match &self.value_from {
            None => self.value.clone().with_context(|| {
                format!(
                    "step '{}': neither 'value' nor 'valueFrom' is set",
                    ctx.step_id
                )
            }),
            Some(value_from) => {
                let raw = value_from.resolve(ctx)?;
                match &self.value {
                    Some(template) => Ok(render_value(template, &[("@value", &raw)])),
                    None => Ok(raw),
                }
            }
        }
    }
}

impl ValueFrom {
    fn resolve(&self, ctx: &StepContext) -> Result<Value> {
        match (&self.file, &self.generator) {
            (Some(file), None) => Self::read_file(file, ctx),
            (None, Some(generator)) => generator.generate(self, ctx),
            (Some(_), Some(_)) => {
                bail!(
                    "step '{}': 'valueFrom' cannot set both 'file' and 'generator'",
                    ctx.step_id
                )
            }
            (None, None) => bail!(
                "step '{}': 'valueFrom' needs either 'file' or 'generator'",
                ctx.step_id
            ),
        }
    }

    fn read_file(file: &PathBuf, ctx: &StepContext) -> Result<Value> {
        let path = ctx.base_dir.join(file);
        let content = fs::read_to_string(&path).with_context(|| {
            format!(
                "step '{}': unable to read '{}'",
                ctx.step_id,
                path.display()
            )
        })?;
        serde_json::from_str(&content).with_context(|| {
            format!(
                "step '{}': '{}' is not valid JSON",
                ctx.step_id,
                path.display()
            )
        })
    }
}

impl Generator {
    fn generate(self, value_from: &ValueFrom, ctx: &StepContext) -> Result<Value> {
        match self {
            Generator::Uuid => Ok(Value::String(Uuid::new_v4().to_string())),
            Generator::Timestamp => {
                let format = value_from.format.as_deref().unwrap_or("%Y-%m-%dT%H:%M:%SZ");
                Ok(Value::String(Utc::now().format(format).to_string()))
            }
            Generator::Hash => Self::generate_hash(value_from, ctx),
        }
    }

    fn generate_hash(value_from: &ValueFrom, ctx: &StepContext) -> Result<Value> {
        match value_from.algo.as_deref() {
            Some("sha256") => {}
            Some(other) => bail!(
                "step '{}': unsupported hash algorithm '{}' (only 'sha256' is supported)",
                ctx.step_id,
                other
            ),
            None => bail!(
                "step '{}': 'valueFrom.generator: hash' requires 'algo'",
                ctx.step_id
            ),
        }

        let hash = match (&value_from.path, &value_from.url) {
            (Some(path), None) => {
                let resolved = ctx.base_dir.join(path);
                sha256_file(&resolved).with_context(|| {
                    format!(
                        "step '{}': unable to hash '{}'",
                        ctx.step_id,
                        resolved.display()
                    )
                })?
            }
            (None, Some(url)) => {
                let temp_file = std::env::temp_dir().join(format!("{}.tmp", Uuid::new_v4()));
                download_and_hash_sha256(url, &temp_file).with_context(|| {
                    format!(
                        "step '{}': unable to download and hash '{}'",
                        ctx.step_id, url
                    )
                })?
            }
            (Some(_), Some(_)) => bail!(
                "step '{}': 'valueFrom' cannot set both 'path' and 'url'",
                ctx.step_id
            ),
            (None, None) => bail!(
                "step '{}': 'valueFrom.generator: hash' needs either 'path' or 'url'",
                ctx.step_id
            ),
        };

        Ok(Value::String(hash))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx() -> StepContext {
        StepContext {
            base_dir: PathBuf::from("."),
            step_id: "test-step".to_string(),
            description: None,
        }
    }

    #[test]
    fn creates_missing_target_including_nested_object() {
        let step: AddStep = serde_json::from_value(json!({
            "target": "$.metadata.newField",
            "value": "hello",
        }))
        .unwrap();
        let mut doc = json!({"metadata": {}});
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(doc, json!({"metadata": {"newField": "hello"}}));
    }

    #[test]
    fn replaces_an_existing_target() {
        let step: AddStep = serde_json::from_value(json!({
            "target": "$.specVersion",
            "value": "1.6",
        }))
        .unwrap();
        let mut doc = json!({"specVersion": "1.5"});
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(doc, json!({"specVersion": "1.6"}));
    }

    #[test]
    fn generator_uuid_without_value_produces_a_valid_uuid_v4() {
        let step: AddStep = serde_json::from_value(json!({
            "target": "$.serialNumber",
            "valueFrom": {"generator": "uuid"},
        }))
        .unwrap();
        let mut doc = json!({});
        step.apply(&mut doc, &ctx()).unwrap();
        let raw = doc["serialNumber"].as_str().unwrap();
        assert!(Uuid::parse_str(raw).is_ok());
    }

    #[test]
    fn generator_uuid_wrapped_by_a_value_template() {
        let step: AddStep = serde_json::from_value(json!({
            "target": "$.serialNumber",
            "valueFrom": {"generator": "uuid"},
            "value": "urn:uuid:{@value}",
        }))
        .unwrap();
        let mut doc = json!({});
        step.apply(&mut doc, &ctx()).unwrap();
        let raw = doc["serialNumber"].as_str().unwrap();
        let uuid_part = raw.strip_prefix("urn:uuid:").unwrap();
        assert!(Uuid::parse_str(uuid_part).is_ok());
    }

    #[test]
    fn generator_timestamp_respects_the_requested_format() {
        let step: AddStep = serde_json::from_value(json!({
            "target": "$.metadata.timestamp",
            "valueFrom": {"generator": "timestamp", "format": "%Y"},
        }))
        .unwrap();
        let mut doc = json!({"metadata": {}});
        step.apply(&mut doc, &ctx()).unwrap();
        let raw = doc["metadata"]["timestamp"].as_str().unwrap();
        assert_eq!(raw.len(), 4);
        assert!(raw.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn when_guards_against_writing_to_a_missing_target() {
        let step: AddStep = serde_json::from_value(json!({
            "target": "$.[\"$schema\"]",
            "value": "https://example.com/schema.json",
            "when": "string",
        }))
        .unwrap();
        let mut doc = json!({"specVersion": "1.5"});
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(doc, json!({"specVersion": "1.5"}));
    }

    #[test]
    fn when_allows_writing_when_the_current_value_matches_the_type() {
        let step: AddStep = serde_json::from_value(json!({
            "target": "$.[\"$schema\"]",
            "value": "https://example.com/schema.json",
            "when": "string",
        }))
        .unwrap();
        let mut doc = json!({"$schema": "old-schema"});
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(doc, json!({"$schema": "https://example.com/schema.json"}));
    }

    #[test]
    fn filter_target_creates_the_missing_field_on_the_matching_element() {
        let step: AddStep = serde_json::from_value(json!({
            "target": "$.externalReferences[?type==distribution].hashes",
            "value": [{"alg": "SHA-256", "content": "deadbeef"}],
        }))
        .unwrap();
        let mut doc = json!({
            "externalReferences": [
                {"type": "distribution", "url": "test"},
                {"type": "website", "url": "other"},
            ]
        });
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(
            doc,
            json!({
                "externalReferences": [
                    {
                        "type": "distribution",
                        "url": "test",
                        "hashes": [{"alg": "SHA-256", "content": "deadbeef"}],
                    },
                    {"type": "website", "url": "other"},
                ]
            })
        );
    }

    #[test]
    fn filter_target_overwrites_an_existing_field_on_the_matching_element() {
        let step: AddStep = serde_json::from_value(json!({
            "target": "$.externalReferences[?type==distribution].hashes",
            "value": [{"alg": "SHA-256", "content": "new"}],
        }))
        .unwrap();
        let mut doc = json!({
            "externalReferences": [
                {"type": "distribution", "hashes": [{"alg": "SHA-256", "content": "old"}]},
            ]
        });
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(
            doc["externalReferences"][0]["hashes"],
            json!([{"alg": "SHA-256", "content": "new"}])
        );
    }

    #[test]
    fn filter_target_matching_nothing_is_a_noop() {
        let step: AddStep = serde_json::from_value(json!({
            "target": "$.externalReferences[?type==distribution].hashes",
            "value": [{"alg": "SHA-256", "content": "deadbeef"}],
        }))
        .unwrap();
        let mut doc = json!({
            "externalReferences": [{"type": "website", "url": "other"}]
        });
        let before = doc.clone();
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(doc, before);
    }

    #[test]
    fn filter_target_writes_every_matching_element() {
        let step: AddStep = serde_json::from_value(json!({
            "target": "$.externalReferences[?type==distribution].hashes",
            "value": [{"alg": "SHA-256", "content": "deadbeef"}],
        }))
        .unwrap();
        let mut doc = json!({
            "externalReferences": [
                {"type": "distribution", "url": "a"},
                {"type": "distribution", "url": "b"},
            ]
        });
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(
            doc["externalReferences"][0]["hashes"],
            json!([{"alg": "SHA-256", "content": "deadbeef"}])
        );
        assert_eq!(
            doc["externalReferences"][1]["hashes"],
            json!([{"alg": "SHA-256", "content": "deadbeef"}])
        );
    }

    #[test]
    fn value_from_file_pointing_to_invalid_json_is_a_step_error() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("bad.json");
        fs::write(&file_path, "{not json").unwrap();

        let step: AddStep = serde_json::from_value(json!({
            "target": "$.metadata.extra",
            "valueFrom": {"file": "bad.json"},
        }))
        .unwrap();
        let mut doc = json!({"metadata": {}});
        let ctx = StepContext {
            base_dir: dir.path().to_path_buf(),
            step_id: "test-step".to_string(),
            description: None,
        };
        let err = step.apply(&mut doc, &ctx).unwrap_err();
        assert!(err.to_string().contains("bad.json"));
    }

    /// sha256("hello world"), verified with `sha256sum` outside this test.
    const HELLO_WORLD_SHA256: &str =
        "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";

    #[test]
    fn generator_hash_sha256_with_path_matches_the_expected_digest() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("artifact.bin");
        fs::write(&file_path, b"hello world").unwrap();

        let step: AddStep = serde_json::from_value(json!({
            "target": "$.metadata.hash",
            "valueFrom": {"generator": "hash", "algo": "sha256", "path": "artifact.bin"},
        }))
        .unwrap();
        let mut doc = json!({"metadata": {}});
        let ctx = StepContext {
            base_dir: dir.path().to_path_buf(),
            step_id: "test-step".to_string(),
            description: None,
        };
        step.apply(&mut doc, &ctx).unwrap();
        assert_eq!(doc["metadata"]["hash"], HELLO_WORLD_SHA256);
    }

    #[test]
    fn generator_hash_sha256_with_url_hashes_the_downloaded_content() {
        let url = spawn_single_response_http_server(b"hello world");

        let step: AddStep = serde_json::from_value(json!({
            "target": "$.metadata.hash",
            "valueFrom": {"generator": "hash", "algo": "sha256", "url": url},
        }))
        .unwrap();
        let mut doc = json!({"metadata": {}});
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(doc["metadata"]["hash"], HELLO_WORLD_SHA256);
    }

    #[test]
    fn generator_hash_rejects_an_unsupported_algo() {
        let step: AddStep = serde_json::from_value(json!({
            "target": "$.metadata.hash",
            "valueFrom": {"generator": "hash", "algo": "md5", "path": "artifact.bin"},
        }))
        .unwrap();
        let mut doc = json!({"metadata": {}});
        let err = step.apply(&mut doc, &ctx()).unwrap_err();
        assert!(err.to_string().contains("md5"));
    }

    #[test]
    fn generator_hash_requires_either_path_or_url() {
        let step: AddStep = serde_json::from_value(json!({
            "target": "$.metadata.hash",
            "valueFrom": {"generator": "hash", "algo": "sha256"},
        }))
        .unwrap();
        let mut doc = json!({"metadata": {}});
        let err = step.apply(&mut doc, &ctx()).unwrap_err();
        assert!(err.to_string().contains("path") && err.to_string().contains("url"));
    }

    #[test]
    fn generator_hash_rejects_path_and_url_together() {
        let step: AddStep = serde_json::from_value(json!({
            "target": "$.metadata.hash",
            "valueFrom": {
                "generator": "hash",
                "algo": "sha256",
                "path": "artifact.bin",
                "url": "http://example.invalid/artifact.bin",
            },
        }))
        .unwrap();
        let mut doc = json!({"metadata": {}});
        let err = step.apply(&mut doc, &ctx()).unwrap_err();
        assert!(err.to_string().contains("path") && err.to_string().contains("url"));
    }

    #[test]
    fn generator_hash_with_a_missing_path_is_a_step_error() {
        let step: AddStep = serde_json::from_value(json!({
            "target": "$.metadata.hash",
            "valueFrom": {"generator": "hash", "algo": "sha256", "path": "does-not-exist.bin"},
        }))
        .unwrap();
        let mut doc = json!({"metadata": {}});
        let err = step.apply(&mut doc, &ctx()).unwrap_err();
        assert!(err.to_string().contains("does-not-exist.bin"));
    }

    #[test]
    fn generator_hash_wrapped_in_a_native_yaml_hashes_array() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("artifact.bin");
        fs::write(&file_path, b"hello world").unwrap();

        let step: AddStep = serde_json::from_value(json!({
            "target": "$.metadata.component.hashes",
            "valueFrom": {"generator": "hash", "algo": "sha256", "path": "artifact.bin"},
            "value": [{"alg": "SHA-256", "content": "{@value}"}],
        }))
        .unwrap();
        let mut doc = json!({"metadata": {"component": {}}});
        let ctx = StepContext {
            base_dir: dir.path().to_path_buf(),
            step_id: "test-step".to_string(),
            description: None,
        };
        step.apply(&mut doc, &ctx).unwrap();
        assert_eq!(
            doc["metadata"]["component"]["hashes"],
            json!([{"alg": "SHA-256", "content": HELLO_WORLD_SHA256}])
        );
    }

    #[test]
    fn value_as_a_json_looking_quoted_string_is_posed_literally_not_reparsed() {
        let step: AddStep = serde_json::from_value(json!({
            "target": "$.metadata.raw",
            "valueFrom": {"generator": "uuid"},
            "value": "[{\"a\": \"{@value}\"}]",
        }))
        .unwrap();
        let mut doc = json!({"metadata": {}});
        step.apply(&mut doc, &ctx()).unwrap();
        let raw = doc["metadata"]["raw"].as_str().unwrap();
        assert!(raw.starts_with("[{\"a\": \""));
        assert!(raw.ends_with("\"}]"));
    }

    /// Minimal single-shot HTTP/1.1 server returning `body` for one request,
    /// used to exercise `valueFrom.url` without depending on network access
    /// or an HTTP-mocking crate.
    fn spawn_single_response_http_server(body: &'static [u8]) -> String {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(body);
            }
        });
        format!("http://{addr}/")
    }
}
