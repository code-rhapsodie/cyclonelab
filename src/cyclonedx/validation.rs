//! CycloneDX JSON schema validation, shared by the `validate` and
//! `transform` commands: extracts `specVersion` from a document, picks the
//! matching bundled schema, and validates against it.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use jsonschema::{Retrieve, Uri};
use serde_json::Value;

/// CycloneDX JSON schemas bundled at compile time (see `schema/`), keyed by
/// the `specVersion` they describe.
const SCHEMAS: &[(&str, &str)] = &[
    ("1.5", include_str!("../../schema/bom-1.5.schema.json")),
    ("1.6", include_str!("../../schema/bom-1.6.schema.json")),
    ("1.7", include_str!("../../schema/bom-1.7.schema.json")),
];

/// Companion schemas that the `bom-*.schema.json` files above reference by
/// URI (their `$id`), bundled at compile time so validation never needs
/// network access.
const EXTERNAL_SCHEMAS: &[(&str, &str)] = &[
    (
        "http://cyclonedx.org/schema/spdx.schema.json",
        include_str!("../../schema/spdx.schema.json"),
    ),
    (
        "http://cyclonedx.org/schema/jsf-0.82.schema.json",
        include_str!("../../schema/jsf-0.82.schema.json"),
    ),
    (
        "http://cyclonedx.org/schema/cryptography-defs.schema.json",
        include_str!("../../schema/cryptography-defs.schema.json"),
    ),
];

/// Resolves the external schemas referenced by the CycloneDX schemas from
/// the bundled copies in `EXTERNAL_SCHEMAS`, instead of fetching them over
/// the network.
struct EmbeddedRetriever;

impl Retrieve for EmbeddedRetriever {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let uri = uri.as_str();
        let source = EXTERNAL_SCHEMAS
            .iter()
            .find(|(id, _)| *id == uri)
            .map(|(_, source)| *source)
            .ok_or_else(|| format!("No bundled schema for external reference '{uri}'"))?;
        Ok(serde_json::from_str(source)?)
    }
}

/// One schema validation failure, in the shape callers print as
/// `"  - {instance_path}: {message}"`.
pub struct SchemaError {
    pub instance_path: String,
    pub message: String,
}

/// Result of validating a document against its own `specVersion`'s schema.
pub struct ValidationOutcome {
    pub spec_version: String,
    pub errors: Vec<SchemaError>,
}

/// Extracts `specVersion` from `instance`, picks the matching bundled
/// schema, and validates `instance` against it. Fails if `specVersion` is
/// missing or unsupported; the caller decides how to report `errors`.
pub fn validate_bom(instance: &Value) -> Result<ValidationOutcome> {
    let spec_version = instance
        .get("specVersion")
        .and_then(Value::as_str)
        .context("Document has no (or a non-string) 'specVersion' field")?;

    let schema_source = SCHEMAS
        .iter()
        .find(|(version, _)| *version == spec_version)
        .map(|(_, schema)| *schema)
        .with_context(|| {
            let supported = SCHEMAS
                .iter()
                .map(|(version, _)| *version)
                .collect::<Vec<_>>()
                .join(", ");
            format!("Unsupported specVersion '{spec_version}' (supported: {supported})")
        })?;

    let schema: Value = serde_json::from_str(schema_source)
        .context("Embedded CycloneDX schema is not valid JSON (this is a bug in cyclonelab)")?;

    let validator = jsonschema::options()
        .with_retriever(EmbeddedRetriever)
        .build(&schema)
        .context("Embedded CycloneDX schema failed to compile (this is a bug in cyclonelab)")?;

    let mut errors: Vec<SchemaError> = validator
        .iter_errors(instance)
        .map(|error| SchemaError {
            instance_path: error.instance_path().to_string(),
            message: error.to_string(),
        })
        .collect();

    errors.extend(find_duplicate_bom_refs(instance));
    errors.extend(find_dangling_bom_refs(instance));

    Ok(ValidationOutcome {
        spec_version: spec_version.to_string(),
        errors,
    })
}

/// Every `bom-ref` in the document must be unique (components, services,
/// vulnerabilities, annotations, and any other object carrying one), a
/// constraint the JSON Schema itself cannot express. Walks the whole
/// document looking for a `"bom-ref"` string field on any object, wherever
/// it is nested, and reports every value used more than once.
fn find_duplicate_bom_refs(instance: &Value) -> Vec<SchemaError> {
    let mut occurrences: Vec<(String, String)> = Vec::new();
    collect_bom_refs(instance, "", &mut occurrences);

    let mut paths_by_ref: HashMap<&str, Vec<&str>> = HashMap::new();
    for (bom_ref, path) in &occurrences {
        paths_by_ref
            .entry(bom_ref.as_str())
            .or_default()
            .push(path.as_str());
    }

    let mut errors: Vec<SchemaError> = paths_by_ref
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .flat_map(|(bom_ref, mut paths)| {
            paths.sort_unstable();
            let first = paths[0].to_string();
            paths.into_iter().skip(1).map(move |path| SchemaError {
                instance_path: path.to_string(),
                message: format!("duplicate bom-ref '{bom_ref}', also defined at '{first}'"),
            })
        })
        .collect();

    errors.sort_by(|a, b| a.instance_path.cmp(&b.instance_path));
    errors
}

/// Object fields whose value is a single `bom-ref` string that must resolve
/// to an object defined elsewhere in the document (e.g. `dependency.ref`,
/// `vulnerability.affects[].ref`, or the cryptographic `*Ref` fields).
const SCALAR_REF_FIELDS: &[&str] = &[
    "ref",
    "signatureAlgorithmRef",
    "subjectPublicKeyRef",
    "algorithmRef",
];

/// Object fields whose value, when it is an array of strings, is a list of
/// `bom-ref` values that must each resolve to an object defined elsewhere in
/// the document (e.g. `dependency.dependsOn`, `dependency.provides`, or
/// `compositions[].assemblies`).
///
/// A couple of these names (`dependencies`, `vulnerabilities`) are reused
/// elsewhere in the schema for arrays of *objects* rather than arrays of
/// `bom-ref` strings (the top-level `dependencies` and `vulnerabilities`
/// properties, for instance). The "all items are strings" check in
/// `collect_bom_ref_references` tells the two apart: an array of objects is
/// left alone here and walked recursively instead, so references nested
/// inside those objects (e.g. `dependencies[].ref`) are still found.
const ARRAY_REF_FIELDS: &[&str] = &[
    "dependsOn",
    "provides",
    "assemblies",
    "dependencies",
    "vulnerabilities",
];

/// Every `bom-ref` *reference* (as opposed to a `bom-ref` *definition*, see
/// [`find_duplicate_bom_refs`]) must resolve to an object defined somewhere
/// in the document, a constraint the JSON Schema itself cannot express since
/// it only checks that these fields are strings. Walks the whole document
/// looking for the fields listed in `SCALAR_REF_FIELDS` and
/// `ARRAY_REF_FIELDS`, and reports any value among them that isn't a known
/// `bom-ref` and isn't a BOM-Link URN (`urn:cdx:...`, which intentionally
/// points outside this document and can't be resolved locally).
fn find_dangling_bom_refs(instance: &Value) -> Vec<SchemaError> {
    let mut definitions: Vec<(String, String)> = Vec::new();
    collect_bom_refs(instance, "", &mut definitions);
    let known: HashSet<&str> = definitions
        .iter()
        .map(|(bom_ref, _)| bom_ref.as_str())
        .collect();

    let mut references: Vec<(String, String)> = Vec::new();
    collect_bom_ref_references(instance, "", &mut references);

    let mut errors: Vec<SchemaError> = references
        .into_iter()
        .filter(|(bom_ref, _)| !bom_ref.starts_with("urn:cdx:") && !known.contains(bom_ref.as_str()))
        .map(|(bom_ref, path)| SchemaError {
            instance_path: path,
            message: format!(
                "dangling reference to bom-ref '{bom_ref}', which is not defined anywhere in the document"
            ),
        })
        .collect();

    errors.sort_by(|a, b| a.instance_path.cmp(&b.instance_path));
    errors
}

/// Recursively walks `value`, recording the JSON pointer path of every
/// `bom-ref` reference found in a field listed in `SCALAR_REF_FIELDS` or
/// `ARRAY_REF_FIELDS`.
fn collect_bom_ref_references(value: &Value, path: &str, out: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let child_path = format!("{path}/{}", escape_pointer_segment(key));

                if SCALAR_REF_FIELDS.contains(&key.as_str()) {
                    if let Some(bom_ref) = child.as_str() {
                        out.push((bom_ref.to_string(), child_path.clone()));
                    }
                } else if ARRAY_REF_FIELDS.contains(&key.as_str())
                    && let Some(items) = child.as_array()
                    && items.iter().all(Value::is_string)
                {
                    for (index, item) in items.iter().enumerate() {
                        out.push((
                            item.as_str().unwrap().to_string(),
                            format!("{child_path}/{index}"),
                        ));
                    }
                }

                collect_bom_ref_references(child, &child_path, out);
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                collect_bom_ref_references(item, &format!("{path}/{index}"), out);
            }
        }
        _ => {}
    }
}

/// Recursively walks `value`, recording the JSON pointer path of every
/// object that has a `"bom-ref"` string field.
fn collect_bom_refs(value: &Value, path: &str, out: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) => {
            if let Some(bom_ref) = map.get("bom-ref").and_then(Value::as_str) {
                out.push((bom_ref.to_string(), path.to_string()));
            }
            for (key, child) in map {
                collect_bom_refs(
                    child,
                    &format!("{path}/{}", escape_pointer_segment(key)),
                    out,
                );
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                collect_bom_refs(item, &format!("{path}/{index}"), out);
            }
        }
        _ => {}
    }
}

/// Escapes a JSON object key for use as a segment of a JSON pointer (RFC
/// 6901): `~` becomes `~0` and `/` becomes `~1`.
fn escape_pointer_segment(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_duplicates_reports_no_errors() {
        let instance = serde_json::json!({
            "components": [
                {"bom-ref": "a", "type": "library", "name": "a"},
                {"bom-ref": "b", "type": "library", "name": "b"},
            ],
        });

        assert!(find_duplicate_bom_refs(&instance).is_empty());
    }

    #[test]
    fn detects_duplicate_components() {
        let instance = serde_json::json!({
            "components": [
                {"bom-ref": "dup", "type": "library", "name": "a"},
                {"bom-ref": "dup", "type": "library", "name": "b"},
            ],
        });

        let errors = find_duplicate_bom_refs(&instance);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].instance_path, "/components/1");
        assert!(errors[0].message.contains("dup"));
        assert!(errors[0].message.contains("/components/0"));
    }

    #[test]
    fn detects_duplicates_across_nested_components_and_other_object_kinds() {
        let instance = serde_json::json!({
            "components": [
                {
                    "bom-ref": "shared",
                    "type": "library",
                    "name": "outer",
                    "components": [
                        {"bom-ref": "nested-ok", "type": "library", "name": "inner"},
                    ],
                },
            ],
            "services": [
                {"bom-ref": "shared", "name": "svc"},
            ],
            "vulnerabilities": [
                {"bom-ref": "vuln", "id": "CVE-0000-0000"},
            ],
        });

        let errors = find_duplicate_bom_refs(&instance);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("shared"));
    }

    #[test]
    fn ignores_objects_without_a_bom_ref() {
        let instance = serde_json::json!({
            "components": [
                {"type": "library", "name": "a"},
                {"type": "library", "name": "b"},
            ],
        });

        assert!(find_duplicate_bom_refs(&instance).is_empty());
    }

    #[test]
    fn no_dangling_references_reports_no_errors() {
        let instance = serde_json::json!({
            "components": [
                {"bom-ref": "a", "type": "library", "name": "a"},
                {"bom-ref": "b", "type": "library", "name": "b"},
            ],
            "dependencies": [
                {"ref": "a", "dependsOn": ["b"], "provides": ["b"]},
                {"ref": "b"},
            ],
        });

        assert!(find_dangling_bom_refs(&instance).is_empty());
    }

    #[test]
    fn detects_dangling_reference_in_depends_on() {
        let instance = serde_json::json!({
            "components": [
                {"bom-ref": "a", "type": "library", "name": "a"},
            ],
            "dependencies": [
                {"ref": "a", "dependsOn": ["missing"]},
            ],
        });

        let errors = find_dangling_bom_refs(&instance);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].instance_path, "/dependencies/0/dependsOn/0");
        assert!(errors[0].message.contains("missing"));
    }

    #[test]
    fn ignores_bom_link_urns() {
        let instance = serde_json::json!({
            "dependencies": [
                {
                    "ref": "urn:cdx:3e671687-395b-41f5-a30f-a58921a69b79/1#a",
                    "dependsOn": ["urn:cdx:3e671687-395b-41f5-a30f-a58921a69b79/1#b"],
                },
            ],
        });

        assert!(find_dangling_bom_refs(&instance).is_empty());
    }

    #[test]
    fn distinguishes_top_level_dependencies_from_composition_ref_arrays() {
        let instance = serde_json::json!({
            "components": [
                {"bom-ref": "a", "type": "library", "name": "a"},
            ],
            // A `dependency` object under the top-level `dependencies`
            // array, not a bare bom-ref string: its own `ref` is checked,
            // but the array itself must not be treated as a ref list.
            "dependencies": [
                {"ref": "missing-dependency"},
            ],
            "compositions": [
                {"assemblies": ["a"], "dependencies": ["missing-composition-ref"]},
            ],
        });

        let mut errors = find_dangling_bom_refs(&instance);
        errors.sort_by(|a, b| a.instance_path.cmp(&b.instance_path));

        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0].instance_path, "/compositions/0/dependencies/0");
        assert!(errors[0].message.contains("missing-composition-ref"));
        assert_eq!(errors[1].instance_path, "/dependencies/0/ref");
        assert!(errors[1].message.contains("missing-dependency"));
    }

    #[test]
    fn detects_dangling_vulnerability_affects_ref() {
        let instance = serde_json::json!({
            "vulnerabilities": [
                {
                    "id": "CVE-0000-0000",
                    "affects": [
                        {"ref": "missing-component"},
                    ],
                },
            ],
        });

        let errors = find_dangling_bom_refs(&instance);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].instance_path, "/vulnerabilities/0/affects/0/ref");
        assert!(errors[0].message.contains("missing-component"));
    }
}
