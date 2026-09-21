//! Transformation actions applied by the `transform` command (see
//! `doc/transform/README.md` §7.1). Adding a new action means adding a new
//! file here, a new [`StepAction`] variant, and its own struct implementing
//! [`Action`] — no other part of the engine (`commands::transform`) needs to
//! change.

pub mod add;
pub mod manual;
pub mod merge;
mod r#move;
pub mod remove;
pub mod structural;
pub mod upgrade;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Applies one transformation step to a document already loaded as JSON.
pub trait Action {
    /// Applies this step to `doc`. Fields have already had their `{$var}`
    /// placeholders substituted (see [`substitute_vars`]) before this is
    /// called; any further substitution the action itself needs to perform
    /// (e.g. `{@value}`) is its own responsibility.
    fn apply(&self, doc: &mut Value, ctx: &StepContext) -> Result<()>;
}

/// Ambient information steps need beyond the document itself: where to
/// resolve `valueFrom.file` paths from, and which step is currently
/// running, for warnings/error messages (see `manual`).
pub struct StepContext {
    pub base_dir: PathBuf,
    pub step_id: String,
    pub description: Option<String>,
}

impl StepContext {
    pub fn new(base_dir: &Path, step: &Step) -> Self {
        Self {
            base_dir: base_dir.to_path_buf(),
            step_id: step.id.clone(),
            description: step.description.clone(),
        }
    }
}

/// One entry of a transformation file's `steps` list: the fields common to
/// every action (`id`, `description`), plus the action-specific ones,
/// distinguished by the `action` tag.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub id: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(flatten)]
    pub action: StepAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum StepAction {
    Add(add::AddStep),
    Remove(remove::RemoveStep),
    Move(r#move::MoveStep),
    Merge(merge::MergeStep),
    Transform(structural::TransformStep),
    Manual(manual::ManualStep),
    Upgrade(upgrade::UpgradeStep),
}

impl Action for Step {
    fn apply(&self, doc: &mut Value, ctx: &StepContext) -> Result<()> {
        match &self.action {
            StepAction::Add(step) => step.apply(doc, ctx),
            StepAction::Remove(step) => step.apply(doc, ctx),
            StepAction::Move(step) => step.apply(doc, ctx),
            StepAction::Merge(step) => step.apply(doc, ctx),
            StepAction::Transform(step) => step.apply(doc, ctx),
            StepAction::Manual(step) => step.apply(doc, ctx),
            StepAction::Upgrade(step) => step.apply(doc, ctx),
        }
    }
}

/// Replaces every `{$name}` placeholder found in any textual field of
/// `step` with its resolved value from `vars`, before the step is applied.
/// Generic over the whole step (round-tripping it through [`Value`]) so
/// this needs no per-action code: a new action's textual fields are
/// substituted for free.
pub fn substitute_vars(step: &Step, vars: &[(String, String)]) -> Result<Step> {
    let pairs: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let mut value = serde_json::to_value(step)?;
    substitute_strings(&mut value, &pairs);
    Ok(serde_json::from_value(value)?)
}

/// Checks the two invariants a transformation file's `steps` list must
/// satisfy beyond what deserialization already guarantees (see
/// `doc/transform/README.md` §5.4): every step `id` is unique, and every
/// `manual` step carries a non-empty `description` (its only purpose being
/// to display it).
pub fn validate_steps(steps: &[Step]) -> Result<()> {
    let known_upgrade_targets = upgrade::known_version_targets()?;

    let mut seen: HashMap<&str, usize> = HashMap::new();
    for (index, step) in steps.iter().enumerate() {
        if let Some(&first) = seen.get(step.id.as_str()) {
            bail!(
                "duplicate step id '{}' (step #{} and step #{})",
                step.id,
                first + 1,
                index + 1
            );
        }
        seen.insert(&step.id, index);

        if matches!(step.action, StepAction::Manual(_))
            && step
                .description
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
        {
            bail!(
                "step '{}': 'manual' action requires a non-empty 'description'",
                step.id
            );
        }

        if let StepAction::Upgrade(upgrade_step) = &step.action
            && !known_upgrade_targets
                .iter()
                .any(|target| target == &upgrade_step.version_target)
        {
            bail!(
                "step '{}': 'version_target' \"{}\" is not reachable by any embedded upgrade recipe (known targets: {})",
                step.id,
                upgrade_step.version_target,
                known_upgrade_targets.join(", ")
            );
        }
    }
    Ok(())
}

/// Runs every step's own static checks — JSONPath syntax, action-specific
/// option combinations — without touching any document (see the `lint`
/// method each action module implements on its step struct). Used by
/// `commands::lint`, on top of [`validate_steps`], which the `lint` command
/// also runs first.
pub fn lint_steps(steps: &[Step]) -> Result<()> {
    for step in steps {
        let result = match &step.action {
            StepAction::Add(s) => s.lint(),
            StepAction::Remove(s) => s.lint(),
            StepAction::Move(s) => s.lint(),
            StepAction::Merge(s) => s.lint(),
            StepAction::Transform(s) => s.lint(),
            StepAction::Manual(s) => s.lint(),
            StepAction::Upgrade(_) => Ok(()),
        };
        result.with_context(|| format!("step '{}'", step.id))?;
    }
    Ok(())
}

fn substitute_strings(value: &mut Value, vars: &[(&str, &str)]) {
    match value {
        Value::String(s) => *s = crate::util::template::render(s, vars),
        Value::Array(items) => items.iter_mut().for_each(|v| substitute_strings(v, vars)),
        Value::Object(map) => map.values_mut().for_each(|v| substitute_strings(v, vars)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn substitute_vars_rewrites_every_textual_field() {
        let step: Step = serde_json::from_value(json!({
            "id": "s1",
            "action": "add",
            "target": "$.metadata.component.version",
            "value": "{$version}",
        }))
        .unwrap();

        let vars = vec![("$version".to_string(), "1.2.3".to_string())];
        let substituted = substitute_vars(&step, &vars).unwrap();

        match substituted.action {
            StepAction::Add(add) => assert_eq!(add.value, Some(json!("1.2.3"))),
            other => panic!("unexpected action: {other:?}"),
        }
    }

    #[test]
    fn substitute_vars_leaves_unknown_placeholders_untouched() {
        let step: Step = serde_json::from_value(json!({
            "id": "s1",
            "action": "add",
            "target": "$.a",
            "value": "{$unknown}",
        }))
        .unwrap();

        let substituted = substitute_vars(&step, &[]).unwrap();
        match substituted.action {
            StepAction::Add(add) => assert_eq!(add.value, Some(json!("{$unknown}"))),
            other => panic!("unexpected action: {other:?}"),
        }
    }

    fn add_step(id: &str) -> Step {
        serde_json::from_value(json!({"id": id, "action": "add", "target": "$.a", "value": 1}))
            .unwrap()
    }

    #[test]
    fn validate_steps_rejects_duplicate_ids() {
        let steps = vec![add_step("dup"), add_step("dup")];
        let err = validate_steps(&steps).unwrap_err();
        assert_eq!(
            err.to_string(),
            "duplicate step id 'dup' (step #1 and step #2)"
        );
    }

    #[test]
    fn validate_steps_rejects_manual_steps_without_a_description() {
        let step: Step =
            serde_json::from_value(json!({"id": "note", "action": "manual", "target": "$.a"}))
                .unwrap();
        let err = validate_steps(&[step]).unwrap_err();
        assert!(
            err.to_string()
                .contains("requires a non-empty 'description'")
        );
    }

    #[test]
    fn validate_steps_accepts_a_manual_step_with_a_description() {
        let step: Step = serde_json::from_value(json!({
            "id": "note",
            "action": "manual",
            "description": "needs a human",
            "target": "$.a",
        }))
        .unwrap();
        validate_steps(&[step]).unwrap();
    }

    #[test]
    fn validate_steps_rejects_an_upgrade_step_targeting_a_version_no_recipe_leads_to() {
        let step: Step = serde_json::from_value(json!({
            "id": "upgrade",
            "action": "upgrade",
            "version_target": "1.5",
        }))
        .unwrap();
        let err = validate_steps(&[step]).unwrap_err();
        assert!(
            err.to_string()
                .contains("not reachable by any embedded upgrade recipe")
        );
    }

    #[test]
    fn validate_steps_accepts_an_upgrade_step_targeting_a_known_version() {
        let step: Step = serde_json::from_value(json!({
            "id": "upgrade",
            "action": "upgrade",
            "version_target": "1.7",
        }))
        .unwrap();
        validate_steps(&[step]).unwrap();
    }
}
