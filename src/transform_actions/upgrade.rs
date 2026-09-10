//! `upgrade` action: brings the document from its current `specVersion` to
//! `version_target` by injecting, at this exact point, the steps of every
//! embedded `schema/upgrade-X-to-Y.yaml` recipe needed to chain from one to
//! the other (see `doc/transform/action-upgrade.md`).

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Action, Step, StepContext};

/// Upgrade recipes bundled at compile time (see `schema/`), applied in
/// sequence to reach any `version_target` beyond the immediately next one.
/// Kept independent of the user's transformation file location, unlike
/// `merge`/`add`'s `valueFrom.file`, which resolve relative to `ctx.base_dir`.
const EMBEDDED_RECIPES: &[&str] = &[
    include_str!("../../schema/upgrade-1.5-to-1.6.yaml"),
    include_str!("../../schema/upgrade-1.6-to-1.7.yaml"),
];

#[derive(Debug, Deserialize)]
struct Recipe {
    from: String,
    to: String,
    steps: Vec<Step>,
}

fn load_recipes() -> Result<Vec<Recipe>> {
    EMBEDDED_RECIPES
        .iter()
        .map(|source| {
            yaml_serde::from_str(source)
                .context("embedded upgrade recipe is not valid YAML (this is a bug in cyclonelab)")
        })
        .collect()
}

/// Every `version_target` an `upgrade` step could legitimately declare: the
/// `to` of each embedded recipe. Used by [`super::validate_steps`] to reject
/// a `version_target` no chain of recipes ever produces (e.g. `"1.5"`,
/// which is only ever a `from`) before any document is processed.
pub fn known_version_targets() -> Result<Vec<String>> {
    Ok(load_recipes()?.into_iter().map(|r| r.to).collect())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpgradeStep {
    pub version_target: String,
}

impl Action for UpgradeStep {
    fn apply(&self, doc: &mut Value, ctx: &StepContext) -> Result<()> {
        let recipes = load_recipes()?;

        let current = doc
            .get("specVersion")
            .and_then(Value::as_str)
            .with_context(|| {
                format!(
                    "step '{}': document has no (or a non-string) 'specVersion' field",
                    ctx.step_id
                )
            })?
            .to_string();

        let chain = build_chain(&recipes, &current, &self.version_target).with_context(|| {
            format!(
                "step '{}': unable to upgrade to '{}'",
                ctx.step_id, self.version_target
            )
        })?;

        for recipe in chain {
            for step in &recipe.steps {
                let step_ctx = StepContext::new(&ctx.base_dir, step);
                step.apply(doc, &step_ctx).with_context(|| {
                    format!(
                        "step '{}': upgrade '{}' -> '{}': step '{}'",
                        ctx.step_id, recipe.from, recipe.to, step.id
                    )
                })?;
            }
        }

        Ok(())
    }
}

/// Orders every version appearing in `recipes`, from the one that is never a
/// `to` (the oldest known version) up to the one that is never a `from` (the
/// newest), by following each recipe's `from` -> `to` edge. Assumes the
/// embedded recipes form a single chain, true of the two bundled today.
fn version_order(recipes: &[Recipe]) -> Vec<String> {
    let next: HashMap<&str, &str> = recipes
        .iter()
        .map(|r| (r.from.as_str(), r.to.as_str()))
        .collect();
    let froms: HashSet<&str> = recipes.iter().map(|r| r.from.as_str()).collect();
    let tos: HashSet<&str> = recipes.iter().map(|r| r.to.as_str()).collect();

    let Some(&root) = froms.difference(&tos).next() else {
        return Vec::new();
    };

    let mut order = vec![root.to_string()];
    let mut current = root;
    while let Some(&to) = next.get(current) {
        order.push(to.to_string());
        current = to;
    }
    order
}

/// Resolves the ordered list of recipes to apply to bring `current` to
/// `target`, per `doc/transform/action-upgrade.md` §Sémantique:
/// - `target` equal to or behind `current` (rule 2) -> empty chain (no-op).
/// - `target` ahead of `current` (rule 3) -> the recipes chaining them, in order.
/// - `target` unknown to the engine (rule 4) -> explicit error listing the
///   versions actually reachable from `current`.
fn build_chain<'a>(recipes: &'a [Recipe], current: &str, target: &str) -> Result<Vec<&'a Recipe>> {
    let order = version_order(recipes);

    let Some(current_idx) = order.iter().position(|v| v == current) else {
        bail!("specVersion '{current}' is not recognized by the upgrade engine");
    };

    // Index 0 is the root, which is never a legitimate target (no recipe
    // leads to it) — filtered out so it always falls into the "unreachable"
    // branch below, regardless of where `current` sits.
    let target_idx = order
        .iter()
        .position(|v| v == target)
        .filter(|&index| index > 0);

    match target_idx {
        Some(target_idx) if current_idx >= target_idx => Ok(Vec::new()),
        Some(target_idx) => Ok(order[current_idx..target_idx]
            .iter()
            .map(|from| {
                recipes
                    .iter()
                    .find(|r| r.from == *from)
                    .expect("order is derived from each recipe's 'from'")
            })
            .collect()),
        None => {
            let reachable = &order[current_idx + 1..];
            bail!(
                "version_target '{target}' is not reachable from specVersion '{current}' (reachable versions: {})",
                if reachable.is_empty() {
                    "none".to_string()
                } else {
                    reachable.join(", ")
                }
            )
        }
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

    fn step(version_target: &str) -> UpgradeStep {
        UpgradeStep {
            version_target: version_target.to_string(),
        }
    }

    #[test]
    fn a_1_5_document_upgraded_to_1_6_runs_the_1_5_to_1_6_recipe() {
        let mut doc = json!({
            "specVersion": "1.5",
            "metadata": {"manufacture": {"name": "Acme"}},
        });
        step("1.6").apply(&mut doc, &ctx()).unwrap();
        assert_eq!(doc["specVersion"], "1.6");
        assert!(doc["metadata"].get("manufacture").is_none());
        assert_eq!(doc["metadata"]["manufacturer"]["name"], "Acme");
    }

    #[test]
    fn a_1_5_document_upgraded_to_1_7_chains_both_recipes() {
        let mut doc = json!({
            "specVersion": "1.5",
            "components": [{
                "cryptoProperties": {
                    "algorithmProperties": {"curve": "P-256"},
                },
            }],
        });
        step("1.7").apply(&mut doc, &ctx()).unwrap();
        assert_eq!(doc["specVersion"], "1.7");
        assert_eq!(
            doc["components"][0]["cryptoProperties"]["algorithmProperties"]["ellipticCurve"],
            "P-256"
        );
        assert!(
            doc["components"][0]["cryptoProperties"]["algorithmProperties"]
                .get("curve")
                .is_none()
        );
    }

    #[test]
    fn a_document_already_at_the_target_version_is_left_untouched() {
        let mut doc = json!({"specVersion": "1.6", "metadata": {}});
        let before = doc.clone();
        step("1.6").apply(&mut doc, &ctx()).unwrap();
        assert_eq!(doc, before);
    }

    #[test]
    fn a_document_more_recent_than_the_target_is_left_untouched() {
        let mut doc = json!({"specVersion": "1.7", "metadata": {}});
        let before = doc.clone();
        step("1.6").apply(&mut doc, &ctx()).unwrap();
        assert_eq!(doc, before);
    }

    #[test]
    fn an_unreachable_target_is_an_explicit_step_error_and_leaves_the_document_untouched() {
        let mut doc = json!({"specVersion": "1.6", "metadata": {}});
        let before = doc.clone();
        let err = step("1.5").apply(&mut doc, &ctx()).unwrap_err();
        assert!(err.to_string().contains("test-step"));
        assert_eq!(doc, before);
    }

    #[test]
    fn an_unknown_target_version_is_an_explicit_step_error_and_leaves_the_document_untouched() {
        let mut doc = json!({"specVersion": "1.6", "metadata": {}});
        let before = doc.clone();
        let err = step("2.0").apply(&mut doc, &ctx()).unwrap_err();
        assert!(err.to_string().contains("test-step"));
        assert_eq!(doc, before);
    }

    #[test]
    fn a_manual_step_in_the_chain_still_runs_and_leaves_its_field_untouched() {
        let mut doc = json!({
            "specVersion": "1.6",
            "components": [{
                "cryptoProperties": {
                    "protocolProperties": {
                        "ikev2TransformTypes": {"encr": "some-ref"},
                    },
                },
            }],
        });
        step("1.7").apply(&mut doc, &ctx()).unwrap();
        assert_eq!(
            doc["components"][0]["cryptoProperties"]["protocolProperties"]["ikev2TransformTypes"]["encr"],
            "some-ref"
        );
    }

    #[test]
    fn a_field_that_only_exists_from_1_6_onward_is_available_to_a_step_right_after_upgrade() {
        let mut doc = json!({"specVersion": "1.5", "metadata": {}});
        step("1.6").apply(&mut doc, &ctx()).unwrap();

        let merge = crate::transform_actions::merge::MergeStep {
            target: "$.metadata.tools.components[]".to_string(),
            value: "[{\"type\": \"application\", \"name\": \"cyclonedx\"}]".to_string(),
        };
        merge.apply(&mut doc, &ctx()).unwrap();

        assert_eq!(
            doc["metadata"]["tools"]["components"][0]["name"],
            "cyclonedx"
        );
    }

    #[test]
    fn known_version_targets_lists_every_embedded_recipes_to() {
        let known = known_version_targets().unwrap();
        assert_eq!(known, vec!["1.6".to_string(), "1.7".to_string()]);
    }
}
