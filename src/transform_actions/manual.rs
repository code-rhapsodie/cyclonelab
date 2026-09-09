//! `manual` action: flags a field the recipe does not migrate
//! automatically. Never touches the document — a plain warning (see
//! `doc/transform/action-manual.md`).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::util::jsonpath;

use super::{Action, StepContext};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum PathsField {
    Single { target: String },
    Many { paths: Vec<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManualStep {
    #[serde(flatten)]
    paths: PathsField,
}

impl ManualStep {
    fn paths(&self) -> &[String] {
        match &self.paths {
            PathsField::Single { target } => std::slice::from_ref(target),
            PathsField::Many { paths } => paths,
        }
    }
}

impl Action for ManualStep {
    fn apply(&self, doc: &mut Value, ctx: &StepContext) -> Result<()> {
        let matched: Vec<_> = self
            .paths()
            .iter()
            .flat_map(|pattern| jsonpath::resolve(doc, pattern).unwrap_or_default())
            .collect();

        if matched.is_empty() {
            return Ok(());
        }

        println!(
            "step '{}': {}",
            ctx.step_id,
            ctx.description.as_deref().unwrap_or("")
        );
        for path in &matched {
            println!("  {}", jsonpath::display(path));
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
            description: Some("needs manual review".to_string()),
        }
    }

    #[test]
    fn present_path_leaves_the_document_untouched() {
        let step: ManualStep = serde_json::from_value(json!({"target": "$.a"})).unwrap();
        let mut doc = json!({"a": 1});
        let before = doc.clone();
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(doc, before);
    }

    #[test]
    fn absent_path_leaves_the_document_untouched() {
        let step: ManualStep = serde_json::from_value(json!({"target": "$.missing"})).unwrap();
        let mut doc = json!({"a": 1});
        let before = doc.clone();
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(doc, before);
    }

    #[test]
    fn multiple_paths_only_present_ones_are_resolved() {
        let step: ManualStep =
            serde_json::from_value(json!({"paths": ["$.a", "$.missing", "$.b"]})).unwrap();
        let mut doc = json!({"a": 1, "b": 2});
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(doc, json!({"a": 1, "b": 2}));
    }
}
