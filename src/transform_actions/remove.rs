//! `remove` action: deletes the value present at a location in the document
//! (see `doc/transform/action-remove.md`).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::util::jsonpath::{self, JsonType};

use super::{Action, StepContext};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoveStep {
    pub target: String,
    #[serde(default)]
    pub when: Option<JsonType>,
}

impl Action for RemoveStep {
    fn apply(&self, doc: &mut Value, _ctx: &StepContext) -> Result<()> {
        let locations = jsonpath::resolve(doc, &self.target)?;
        let matching = locations
            .into_iter()
            .filter(|path| {
                jsonpath::get(doc, path).is_some_and(|v| jsonpath::matches(v, self.when))
            })
            .collect();
        jsonpath::remove_many(doc, matching);
        Ok(())
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
    fn removes_a_present_target_of_any_shape() {
        for value in [json!("scalar"), json!({"a": 1}), json!([1, 2])] {
            let step: RemoveStep = serde_json::from_value(json!({"target": "$.field"})).unwrap();
            let mut doc = json!({"field": value, "other": true});
            step.apply(&mut doc, &ctx()).unwrap();
            assert_eq!(doc, json!({"other": true}));
        }
    }

    #[test]
    fn absent_target_is_a_noop() {
        let step: RemoveStep = serde_json::from_value(json!({"target": "$.missing"})).unwrap();
        let mut doc = json!({"a": 1});
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(doc, json!({"a": 1}));
    }

    #[test]
    fn recursive_pattern_removes_every_occurrence_at_any_depth() {
        let step: RemoveStep = serde_json::from_value(json!({"target": "$..field"})).unwrap();
        let mut doc = json!({"field": 1, "nested": {"field": 2, "other": 3}});
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(doc, json!({"nested": {"other": 3}}));
    }

    #[test]
    fn when_excludes_occurrences_of_a_different_shape() {
        let step: RemoveStep =
            serde_json::from_value(json!({"target": "$..author", "when": "string"})).unwrap();
        let mut doc = json!({
            "author": "top",
            "commit": {"author": {"name": "kept"}}
        });
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(doc, json!({"commit": {"author": {"name": "kept"}}}));
    }
}
