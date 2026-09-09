//! `move` action: renames/relocates the value present at `source` to
//! `target` (see `doc/transform/action-move.md`).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::util::jsonpath::{self, JsonType, PathElem};

use super::{Action, StepContext};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveStep {
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub when: Option<JsonType>,
}

impl Action for MoveStep {
    fn apply(&self, doc: &mut Value, ctx: &StepContext) -> Result<()> {
        let target_key = jsonpath::paired_target_key(&self.source, &self.target).with_context(|| {
            format!(
                "step '{}': move: source and target resolved to a different number of locations",
                ctx.step_id
            )
        })?;

        let matched: Vec<_> = jsonpath::resolve(doc, &self.source)?
            .into_iter()
            .filter(|path| {
                jsonpath::get(doc, path).is_some_and(|v| jsonpath::matches(v, self.when))
            })
            .collect();

        for source_path in matched {
            let Some(value) = jsonpath::remove(doc, &source_path) else {
                continue;
            };
            let mut target_path = source_path;
            target_path.pop();
            target_path.push(PathElem::Key(target_key.clone()));
            jsonpath::set(doc, &target_path, value).with_context(|| {
                format!(
                    "step '{}': unable to write to '{}'",
                    ctx.step_id, self.target
                )
            })?;
        }
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
    fn simple_rename() {
        let step: MoveStep =
            serde_json::from_value(json!({"source": "$.a", "target": "$.b"})).unwrap();
        let mut doc = json!({"a": 1});
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(doc, json!({"b": 1}));
    }

    #[test]
    fn absent_source_is_a_noop() {
        let step: MoveStep =
            serde_json::from_value(json!({"source": "$.missing", "target": "$.b"})).unwrap();
        let mut doc = json!({"a": 1});
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(doc, json!({"a": 1}));
    }

    #[test]
    fn recursive_pattern_pairs_source_and_target_per_occurrence() {
        let step: MoveStep =
            serde_json::from_value(json!({"source": "$..old", "target": "$..new"})).unwrap();
        let mut doc = json!({"old": 1, "nested": {"old": 2, "other": 3}});
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(doc, json!({"new": 1, "nested": {"new": 2, "other": 3}}));
    }

    #[test]
    fn when_excludes_an_occurrence_of_a_different_shape() {
        let step: MoveStep = serde_json::from_value(
            json!({"source": "$..author", "target": "$..authorName", "when": "string"}),
        )
        .unwrap();
        let mut doc = json!({
            "author": "top",
            "commit": {"author": {"name": "kept"}}
        });
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(
            doc,
            json!({
                "authorName": "top",
                "commit": {"author": {"name": "kept"}}
            })
        );
    }
}
