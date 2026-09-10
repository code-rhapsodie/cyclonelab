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
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Generator {
    Uuid,
    Timestamp,
}

impl Action for AddStep {
    fn apply(&self, doc: &mut Value, ctx: &StepContext) -> Result<()> {
        let path = jsonpath::literal(&self.target)
            .with_context(|| format!("step '{}': invalid target '{}'", ctx.step_id, self.target))?;

        if let Some(when) = self.when {
            match jsonpath::get(doc, &path) {
                Some(existing) if jsonpath::matches(existing, Some(when)) => {}
                _ => return Ok(()),
            }
        }

        let computed = self.compute_value(ctx)?;
        jsonpath::set(doc, &path, computed).with_context(|| {
            format!(
                "step '{}': unable to write to '{}'",
                ctx.step_id, self.target
            )
        })
    }
}

impl AddStep {
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
            (None, Some(generator)) => Ok(generator.generate(self.format.as_deref())),
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
    fn generate(self, format: Option<&str>) -> Value {
        match self {
            Generator::Uuid => Value::String(Uuid::new_v4().to_string()),
            Generator::Timestamp => {
                let format = format.unwrap_or("%Y-%m-%dT%H:%M:%SZ");
                Value::String(Utc::now().format(format).to_string())
            }
        }
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
}
