//! `transform` action: structural changes a plain `move` cannot express
//! (type change, wrapping, field remapping). Named `structural` (not
//! `transform`) to avoid confusion with `commands::transform` (see
//! `doc/transform/action-transform.md`).

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::util::jsonpath::{self, JsonType, PathElem};
use crate::util::template::render_value;

use super::{Action, StepContext};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformStep {
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub when: Option<JsonType>,
    #[serde(default)]
    pub remove_source: bool,
    #[serde(flatten)]
    pub strategy: Strategy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "strategy", rename_all = "kebab-case")]
pub enum Strategy {
    WrapInArray(WrapInArrayFields),
    LegacyArrayToObject(LegacyArrayToObjectFields),
    MapArray(MapArrayFields),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrapInArrayFields {
    #[serde(default)]
    pub append: bool,
    #[serde(default)]
    pub item: Option<Map<String, Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacyArrayToObjectFields {
    pub build: BTreeMap<String, BuildEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildEntry {
    pub each: EachKind,
    pub map: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EachKind {
    Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapArrayFields {
    pub item: Map<String, Value>,
}

impl Action for TransformStep {
    fn apply(&self, doc: &mut Value, ctx: &StepContext) -> Result<()> {
        let target_key = jsonpath::paired_target_key(&self.source, &self.target).with_context(|| {
            format!(
                "step '{}': transform: source and target resolved to a different number of locations",
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
            let source_value = jsonpath::get(doc, &source_path)
                .context("source value disappeared mid-step")?
                .clone();

            let mut target_path = source_path.clone();
            target_path.pop();
            target_path.push(PathElem::Key(target_key.clone()));

            let existing_target = jsonpath::get(doc, &target_path).cloned();
            let new_value = self
                .strategy
                .build(&source_value, existing_target.as_ref())?;

            jsonpath::set(doc, &target_path, new_value).with_context(|| {
                format!(
                    "step '{}': unable to write to '{}'",
                    ctx.step_id, self.target
                )
            })?;

            if self.remove_source && target_path != source_path {
                jsonpath::remove(doc, &source_path);
            }
        }
        Ok(())
    }
}

impl Strategy {
    fn build(&self, source_value: &Value, existing_target: Option<&Value>) -> Result<Value> {
        match self {
            Strategy::WrapInArray(fields) => Ok(fields.build(source_value, existing_target)),
            Strategy::LegacyArrayToObject(fields) => fields.build(source_value),
            Strategy::MapArray(fields) => fields.build(source_value),
        }
    }
}

impl WrapInArrayFields {
    fn build(&self, source_value: &Value, existing_target: Option<&Value>) -> Value {
        let item = match &self.item {
            Some(template) => render_value(
                &Value::Object(template.clone()),
                &[("@value", source_value)],
            ),
            None => source_value.clone(),
        };

        if self.append
            && let Some(Value::Array(existing)) = existing_target
        {
            let mut items = existing.clone();
            items.push(item);
            return Value::Array(items);
        }
        Value::Array(vec![item])
    }
}

impl LegacyArrayToObjectFields {
    fn build(&self, source_value: &Value) -> Result<Value> {
        let elements = source_value
            .as_array()
            .context("legacy-array-to-object: source value is not an array")?;

        let mut result = Map::new();
        for (key, entry) in &self.build {
            match entry.each {
                EachKind::Value => {}
            }
            let items: Vec<Value> = elements
                .iter()
                .map(|elem| render_item(&entry.map, elem))
                .collect();
            result.insert(key.clone(), Value::Array(items));
        }
        Ok(Value::Object(result))
    }
}

impl MapArrayFields {
    fn build(&self, source_value: &Value) -> Result<Value> {
        let elements = source_value
            .as_array()
            .context("map-array: source value is not an array")?;

        let items: Vec<Value> = elements
            .iter()
            .map(|elem| render_item(&self.item, elem))
            .collect();
        Ok(Value::Array(items))
    }
}

/// Binds `{@item}` to `item` as a whole, plus `{@item.<field>}` for each of
/// its top-level fields when it is an object — covers both the
/// whole-scalar and per-field template shapes used by `map-array` and
/// `legacy-array-to-object`.
fn item_vars(item: &Value) -> Vec<(String, Value)> {
    let mut vars = vec![("@item".to_string(), item.clone())];
    if let Some(map) = item.as_object() {
        for (key, value) in map {
            vars.push((format!("@item.{key}"), value.clone()));
        }
    }
    vars
}

/// Renders `template` against `elem`, but drops any key whose value is
/// exactly a `{@item.<field>}` placeholder for a field `elem` does not have
/// (e.g. a legacy tool without `hashes`) instead of leaving the literal,
/// unsubstituted placeholder text in the built object.
fn render_item(template: &Map<String, Value>, elem: &Value) -> Value {
    let vars = item_vars(elem);
    let known: std::collections::HashSet<&str> =
        vars.iter().map(|(name, _)| name.as_str()).collect();

    let filtered: Map<String, Value> = template
        .iter()
        .filter(|(_, value)| match value.as_str() {
            Some(s) if s.starts_with("{@item.") && s.ends_with('}') => {
                known.contains(&s[1..s.len() - 1])
            }
            _ => true,
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();

    let refs: Vec<(&str, &Value)> = vars.iter().map(|(k, v)| (k.as_str(), v)).collect();
    render_value(&Value::Object(filtered), &refs)
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
    fn wrap_in_array_without_item_wraps_the_value_as_is() {
        let step: TransformStep = serde_json::from_value(json!({
            "source": "$..evidence.identity",
            "target": "$..evidence.identity",
            "strategy": "wrap-in-array",
            "when": "object",
        }))
        .unwrap();
        let mut doc = json!({"evidence": {"identity": {"field": "name"}}});
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(doc, json!({"evidence": {"identity": [{"field": "name"}]}}));
    }

    #[test]
    fn wrap_in_array_with_item_rebuilds_the_element_and_removes_source() {
        let step: TransformStep = serde_json::from_value(json!({
            "source": "$..author",
            "target": "$..authors",
            "strategy": "wrap-in-array",
            "when": "string",
            "remove_source": true,
            "item": {"name": "{@value}"},
        }))
        .unwrap();
        let mut doc = json!({
            "author": "Jane",
            "commit": {"author": {"name": "not-a-match"}}
        });
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(
            doc,
            json!({
                "authors": [{"name": "Jane"}],
                "commit": {"author": {"name": "not-a-match"}}
            })
        );
    }

    #[test]
    fn wrap_in_array_with_append_extends_a_target_populated_by_a_previous_step() {
        let step1: TransformStep = serde_json::from_value(json!({
            "source": "$.cryptoProperties.certificateProperties.signatureAlgorithmRef",
            "target": "$.cryptoProperties.certificateProperties.relatedCryptographicAssets",
            "strategy": "wrap-in-array",
            "when": "string",
            "remove_source": true,
            "append": true,
            "item": {"type": "algorithm", "ref": "{@value}"},
        }))
        .unwrap();
        let step2: TransformStep = serde_json::from_value(json!({
            "source": "$.cryptoProperties.certificateProperties.subjectPublicKeyRef",
            "target": "$.cryptoProperties.certificateProperties.relatedCryptographicAssets",
            "strategy": "wrap-in-array",
            "when": "string",
            "remove_source": true,
            "append": true,
            "item": {"type": "publicKey", "ref": "{@value}"},
        }))
        .unwrap();
        let mut doc = json!({
            "cryptoProperties": {
                "certificateProperties": {
                    "signatureAlgorithmRef": "urn:cdx:sig",
                    "subjectPublicKeyRef": "urn:cdx:key"
                }
            }
        });
        step1.apply(&mut doc, &ctx()).unwrap();
        step2.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(
            doc,
            json!({
                "cryptoProperties": {
                    "certificateProperties": {
                        "relatedCryptographicAssets": [
                            {"type": "algorithm", "ref": "urn:cdx:sig"},
                            {"type": "publicKey", "ref": "urn:cdx:key"}
                        ]
                    }
                }
            })
        );
    }

    #[test]
    fn legacy_array_to_object_on_an_empty_array_produces_an_empty_array_under_the_key() {
        let step: TransformStep = serde_json::from_value(json!({
            "source": "$.metadata.tools",
            "target": "$.metadata.tools",
            "strategy": "legacy-array-to-object",
            "when": "array",
            "build": {
                "components": {"each": "value", "map": {"type": "application", "name": "{@item.name}"}}
            },
        }))
        .unwrap();
        let mut doc = json!({"metadata": {"tools": []}});
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(doc, json!({"metadata": {"tools": {"components": []}}}));
    }

    #[test]
    fn legacy_array_to_object_maps_every_element() {
        let step: TransformStep = serde_json::from_value(json!({
            "source": "$.metadata.tools",
            "target": "$.metadata.tools",
            "strategy": "legacy-array-to-object",
            "when": "array",
            "build": {
                "components": {
                    "each": "value",
                    "map": {"type": "application", "name": "{@item.name}", "version": "{@item.version}"}
                }
            },
        }))
        .unwrap();
        let mut doc = json!({"metadata": {"tools": [
            {"name": "tool-a", "version": "1.0"},
            {"name": "tool-b", "version": "2.0"}
        ]}});
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(
            doc,
            json!({"metadata": {"tools": {"components": [
                {"type": "application", "name": "tool-a", "version": "1.0"},
                {"type": "application", "name": "tool-b", "version": "2.0"}
            ]}}})
        );
    }

    #[test]
    fn legacy_array_to_object_omits_a_mapped_field_absent_from_the_source_item() {
        let step: TransformStep = serde_json::from_value(json!({
            "source": "$.metadata.tools",
            "target": "$.metadata.tools",
            "strategy": "legacy-array-to-object",
            "when": "array",
            "build": {
                "components": {
                    "each": "value",
                    "map": {
                        "type": "application",
                        "name": "{@item.name}",
                        "hashes": "{@item.hashes}"
                    }
                }
            },
        }))
        .unwrap();
        let mut doc = json!({"metadata": {"tools": [{"name": "cargo-cyclonedx"}]}});
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(
            doc,
            json!({"metadata": {"tools": {"components": [
                {"type": "application", "name": "cargo-cyclonedx"}
            ]}}})
        );
    }

    #[test]
    fn map_array_over_scalars_uses_the_whole_item() {
        let step: TransformStep = serde_json::from_value(json!({
            "source": "$.refs",
            "target": "$.assets",
            "strategy": "map-array",
            "when": "array",
            "remove_source": true,
            "item": {"type": "algorithm", "ref": "{@item}"},
        }))
        .unwrap();
        let mut doc = json!({"refs": ["a", "b"]});
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(
            doc,
            json!({"assets": [
                {"type": "algorithm", "ref": "a"},
                {"type": "algorithm", "ref": "b"}
            ]})
        );
    }

    #[test]
    fn map_array_over_objects_uses_per_field_access_without_removing_source() {
        let step: TransformStep = serde_json::from_value(json!({
            "source": "$.items",
            "target": "$.mapped",
            "strategy": "map-array",
            "when": "array",
            "item": {"label": "{@item.name}"},
        }))
        .unwrap();
        let mut doc = json!({"items": [{"name": "x"}, {"name": "y"}]});
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(
            doc,
            json!({
                "items": [{"name": "x"}, {"name": "y"}],
                "mapped": [{"label": "x"}, {"label": "y"}]
            })
        );
    }

    #[test]
    fn when_excludes_a_source_already_in_the_expected_final_shape() {
        let step: TransformStep = serde_json::from_value(json!({
            "source": "$.evidence.identity",
            "target": "$.evidence.identity",
            "strategy": "wrap-in-array",
            "when": "object",
        }))
        .unwrap();
        let mut doc = json!({"evidence": {"identity": [{"field": "already-array"}]}});
        let before = doc.clone();
        step.apply(&mut doc, &ctx()).unwrap();
        assert_eq!(doc, before);
    }
}
