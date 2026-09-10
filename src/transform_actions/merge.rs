//! `merge` action: merges a JSON fragment into the document at a location,
//! creating it if absent (see `doc/transform/action-merge.md`).

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::util::jsonpath;

use super::{Action, StepContext};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeStep {
    pub target: String,
    pub value: String,
}

impl Action for MergeStep {
    fn apply(&self, doc: &mut Value, ctx: &StepContext) -> Result<()> {
        let fragment: Value = serde_json::from_str(&self.value)
            .with_context(|| format!("step '{}': 'value' is not valid JSON", ctx.step_id))?;

        // A trailing `[]` selects the "append to array" form instead of the
        // default "replace/deep-merge" form (see doc/transform/action-merge.md).
        let (target, append) = match self.target.strip_suffix("[]") {
            Some(prefix) => (prefix, true),
            None => (self.target.as_str(), false),
        };

        let path = jsonpath::literal(target)
            .with_context(|| format!("step '{}': invalid target '{}'", ctx.step_id, self.target))?;

        let merged = if append {
            append_to_array(doc, &path, fragment, ctx, &self.target)?
        } else {
            match jsonpath::get(doc, &path) {
                Some(existing) => deep_merge(existing, fragment),
                None => fragment,
            }
        };

        jsonpath::set(doc, &path, merged).with_context(|| {
            format!(
                "step '{}': unable to write to '{}'",
                ctx.step_id, self.target
            )
        })
    }
}

/// Resolves the merged value for the `target[]` append form: `fragment` must
/// itself be a JSON array, whose elements are appended to the array already
/// at `path` (or which simply becomes the new array, if `path` is absent).
/// An existing non-array value at `path` is an explicit step error.
fn append_to_array(
    doc: &Value,
    path: &[jsonpath::PathElem],
    fragment: Value,
    ctx: &StepContext,
    display_target: &str,
) -> Result<Value> {
    let Value::Array(fragment_items) = fragment else {
        bail!(
            "step '{}': 'value' must be a JSON array because target '{}' ends with '[]'",
            ctx.step_id,
            display_target
        );
    };

    match jsonpath::get(doc, path) {
        Some(Value::Array(existing)) => {
            let mut merged = existing.clone();
            merged.extend(fragment_items);
            Ok(Value::Array(merged))
        }
        Some(_) => bail!(
            "step '{}': target '{}' ends with '[]' but the existing value is not an array",
            ctx.step_id,
            display_target
        ),
        None => Ok(Value::Array(fragment_items)),
    }
}

/// Merges `fragment` into `base`: when both are objects, recurses key by
/// key (fragment keys win, base-only keys are kept); otherwise `fragment`
/// replaces `base` entirely.
fn deep_merge(base: &Value, fragment: Value) -> Value {
    match (base, fragment) {
        (Value::Object(base_map), Value::Object(fragment_map)) => {
            let mut result = base_map.clone();
            for (key, fragment_value) in fragment_map {
                let merged = match result.get(&key) {
                    Some(base_value) => deep_merge(base_value, fragment_value),
                    None => fragment_value,
                };
                result.insert(key, merged);
            }
            Value::Object(result)
        }
        (_, fragment) => fragment,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx() -> StepContext {
        StepContext {
            base_dir: ".".into(),
            step_id: "test-step".to_string(),
            description: None,
        }
    }

    #[test]
    fn absent_target_is_created_with_the_fragment_as_is() {
        let step: MergeStep = serde_json::from_value(json!({
            "target": "$.metadata.component",
            "value": "{\"type\": \"library\", \"name\": \"cyclonelab\"}",
        }))
        .unwrap();
        let mut doc = json!({"metadata": {}});
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(
            doc,
            json!({"metadata": {"component": {"type": "library", "name": "cyclonelab"}}})
        );
    }

    #[test]
    fn existing_object_target_is_merged_without_losing_unmentioned_keys() {
        let step: MergeStep = serde_json::from_value(json!({
            "target": "$.component",
            "value": "{\"version\": \"2.0\"}",
        }))
        .unwrap();
        let mut doc = json!({"component": {"name": "cyclonelab", "version": "1.0"}});
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(
            doc,
            json!({"component": {"name": "cyclonelab", "version": "2.0"}})
        );
    }

    #[test]
    fn target_of_a_different_type_is_fully_replaced() {
        let step: MergeStep = serde_json::from_value(json!({
            "target": "$.field",
            "value": "{\"a\": 1}",
        }))
        .unwrap();
        let mut doc = json!({"field": [1, 2, 3]});
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(doc, json!({"field": {"a": 1}}));
    }

    #[test]
    fn invalid_json_fragment_is_an_explicit_step_error() {
        let step: MergeStep = serde_json::from_value(json!({
            "target": "$.field",
            "value": "{not json",
        }))
        .unwrap();
        let mut doc = json!({});
        let err = step.apply(&mut doc, &ctx()).unwrap_err();
        assert!(err.to_string().contains("test-step"));
    }

    #[test]
    fn append_target_appends_to_an_existing_array_without_losing_prior_items() {
        let step: MergeStep = serde_json::from_value(json!({
            "target": "$.metadata.tools.components[]",
            "value": "[{\"name\": \"cyclonelab\"}]",
        }))
        .unwrap();
        let mut doc = json!({"metadata": {"tools": {"components": [{"name": "existing"}]}}});
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(
            doc,
            json!({"metadata": {"tools": {"components": [
                {"name": "existing"},
                {"name": "cyclonelab"}
            ]}}})
        );
    }

    #[test]
    fn append_target_creates_the_array_when_absent() {
        let step: MergeStep = serde_json::from_value(json!({
            "target": "$.metadata.tools.components[]",
            "value": "[{\"name\": \"cyclonelab\"}]",
        }))
        .unwrap();
        let mut doc = json!({"metadata": {"tools": {}}});
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(
            doc,
            json!({"metadata": {"tools": {"components": [{"name": "cyclonelab"}]}}})
        );
    }

    #[test]
    fn append_target_rejects_a_non_array_value() {
        let step: MergeStep = serde_json::from_value(json!({
            "target": "$.metadata.tools.components[]",
            "value": "{\"name\": \"cyclonelab\"}",
        }))
        .unwrap();
        let mut doc = json!({"metadata": {"tools": {"components": []}}});
        let err = step.apply(&mut doc, &ctx()).unwrap_err();
        assert!(err.to_string().contains("must be a JSON array"));
    }

    #[test]
    fn append_target_rejects_an_existing_non_array_value() {
        let step: MergeStep = serde_json::from_value(json!({
            "target": "$.metadata.tools.components[]",
            "value": "[{\"name\": \"cyclonelab\"}]",
        }))
        .unwrap();
        let mut doc = json!({"metadata": {"tools": {"components": {"not": "an array"}}}});
        let err = step.apply(&mut doc, &ctx()).unwrap_err();
        assert!(err.to_string().contains("is not an array"));
    }
}
