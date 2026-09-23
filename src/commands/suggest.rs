//! `suggest` subcommand: flags CycloneDX fields worth adding.
//!
//! Component-level suggestions (`FIELD_SUGGESTIONS`) apply to every
//! component in the document regardless of where it sits
//! (`metadata.component`, a top-level `components[]` entry, a component
//! nested under another one's own `components[]`, a tool component under
//! `metadata.tools.components[]`...). To suggest a new component field, add
//! one entry to [`FIELD_SUGGESTIONS`] — nothing else needs to change, since
//! every component is already located generically (see [`component_paths`]).
//!
//! Document-level suggestions (`METADATA_SUGGESTIONS`) apply once, to the
//! single `metadata` object.
//!
//! `licenses[].license.id` is a nested case that doesn't fit the flat
//! "missing key" shape above (an SPDX id is preferred over a free-text
//! `license.name`), so it gets its own check ([`collect_missing_license_ids`]).
//!
//! ## Experimental JSON output
//!
//! Behind the `json-output` Cargo feature (off by default: not part of the
//! stable CLI yet), setting `CYCLONELAB_SUGGEST_JSON=1` switches the output
//! to a JSON array of `{"path": ..., "reason": ...}` objects instead of the
//! plain-text lines. Without that feature compiled in, the environment
//! variable is never even looked at.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Args;
use serde_json::{Map, Value};

use crate::util::jsonpath::{self, ConcretePath, PathElem};

/// One field worth having on a component, and why.
struct FieldSuggestion {
    /// Key looked up directly on the component object.
    key: &'static str,
    /// Shown to the user next to the JSON path when `key` is missing.
    reason: &'static str,
}

const FIELD_SUGGESTIONS: &[FieldSuggestion] = &[
    FieldSuggestion {
        key: "supplier",
        reason: "identifies who actually supplies the component: needed to know who to contact about a vulnerability and to assess supply-chain trust",
    },
    FieldSuggestion {
        key: "authors",
        reason: "lists the component's author(s), for legal attribution and as a maintainer contact",
    },
    FieldSuggestion {
        key: "manufacturer",
        reason: "identifies who manufactured the component, distinct from its supplier; relevant for OEM/hardware components",
    },
    FieldSuggestion {
        key: "licenses",
        reason: "without license information, consumers cannot assess their legal obligations for this component",
    },
    FieldSuggestion {
        key: "copyright",
        reason: "some licenses (e.g. BSD) require this attribution notice; its absence hides a compliance obligation from consumers",
    },
    FieldSuggestion {
        key: "cpe",
        reason: "a CPE lets vulnerability scanners match this component against the NVD/CVE databases automatically",
    },
    FieldSuggestion {
        key: "swid",
        reason: "an ISO/IEC 19770-2 SWID tag is expected by enterprise software asset-management tools",
    },
    FieldSuggestion {
        key: "omniborId",
        reason: "an ecosystem-independent artifact identifier, useful for cross-tool traceability and build reproducibility",
    },
    FieldSuggestion {
        key: "hashes",
        reason: "hashes let consumers verify that a delivered artifact hasn't been tampered with",
    },
    FieldSuggestion {
        key: "externalReferences",
        reason: "external references (source repository, advisories, distribution...) let consumers audit the component and track its vulnerabilities",
    },
    FieldSuggestion {
        key: "pedigree",
        reason: "if this component was forked or patched from upstream, pedigree tells consumers the code differs from the original, so upstream CVEs may not apply as-is",
    },
    FieldSuggestion {
        key: "evidence",
        reason: "evidence records how this component's presence was established, giving consumers a confidence level for the inventory",
    },
    FieldSuggestion {
        key: "scope",
        reason: "scope (required/optional/excluded) lets consumers avoid over-reacting to vulnerabilities in components that aren't actually shipped",
    },
    FieldSuggestion {
        key: "signature",
        reason: "a digital signature proves this component's authenticity, beyond what a hash alone guarantees",
    },
    FieldSuggestion {
        key: "cryptoProperties",
        reason: "needed for cryptographic inventories (export control, post-quantum readiness) when this component implements cryptography",
    },
    FieldSuggestion {
        key: "data",
        reason: "classifies the data this component processes; relevant for privacy/GDPR obligations if it handles personal data",
    },
    FieldSuggestion {
        key: "modelCard",
        reason: "documents training data, limitations and known biases, if this component is an ML model",
    },
];

const METADATA_SUGGESTIONS: &[FieldSuggestion] = &[
    FieldSuggestion {
        key: "supplier",
        reason: "identifies the publisher of the described product and its security contact (e.g. a PSIRT): the first thing consumers look for when reporting or tracking a vulnerability",
    },
    FieldSuggestion {
        key: "lifecycles",
        reason: "tells consumers at which stage (design, pre-build, build, post-build, operations...) this BOM's data was captured, and so how complete it can be: e.g. a pre-build BOM may miss dependencies only resolved at build time",
    },
];

/// One reported suggestion: where, and why. `Serialize` only exists behind
/// `json-output`, so the JSON (de)serialization code is entirely absent
/// from a default build.
#[cfg_attr(feature = "json-output", derive(serde::Serialize))]
struct Suggestion {
    path: String,
    reason: &'static str,
}

#[derive(Debug, Args)]
pub struct SuggestArgs {
    /// SBOM file (JSON) to analyze.
    file: PathBuf,
}

pub fn run(args: &SuggestArgs) -> Result<()> {
    let content = fs::read_to_string(&args.file)
        .with_context(|| format!("Unable to read '{}'", args.file.display()))?;

    let doc: Value = serde_json::from_str(&content)
        .with_context(|| format!("'{}' is not valid JSON", args.file.display()))?;

    let mut suggestions = Vec::new();

    for path in jsonpath::resolve(&doc, "$.metadata").unwrap_or_default() {
        if let Some(metadata) = jsonpath::get(&doc, &path).and_then(Value::as_object) {
            suggestions.extend(collect_missing_fields(
                &path,
                metadata,
                METADATA_SUGGESTIONS,
            ));
        }
    }

    for path in component_paths(&doc) {
        let Some(component) = jsonpath::get(&doc, &path).and_then(Value::as_object) else {
            continue;
        };

        suggestions.extend(collect_missing_fields(&path, component, FIELD_SUGGESTIONS));
        suggestions.extend(collect_missing_license_ids(&path, component));
    }

    #[cfg(feature = "json-output")]
    if json_output::is_requested() {
        return json_output::print(&suggestions);
    }

    print_text(&args.file, &suggestions);
    Ok(())
}

fn print_text(file: &Path, suggestions: &[Suggestion]) {
    if suggestions.is_empty() {
        println!(
            "'{}': no suggestions, every component already carries all the tracked fields.",
            file.display()
        );
        return;
    }

    for suggestion in suggestions {
        println!("{}: {}", suggestion.path, suggestion.reason);
    }
}

/// Locates every CycloneDX `component` object in `doc`, whatever its
/// position: `metadata.component`, any `components[]` entry (top-level,
/// under `metadata.tools`, or nested inside another component's own
/// `components[]`, at any depth).
fn component_paths(doc: &Value) -> Vec<ConcretePath> {
    // `metadata.component` is a single object, not itself inside a
    // `components[]` array, so it needs its own pattern.
    let mut paths = jsonpath::resolve(doc, "$.metadata.component").unwrap_or_default();
    // `..components[*]` recursively matches a `components` key at any
    // depth, which covers top-level components, tool components, and
    // sub-components alike.
    paths.extend(jsonpath::resolve(doc, "$..components[*]").unwrap_or_default());
    paths
}

/// One [`Suggestion`] for each key of `suggestions` missing from `object`
/// (located at `object_path`).
fn collect_missing_fields(
    object_path: &ConcretePath,
    object: &Map<String, Value>,
    suggestions: &[FieldSuggestion],
) -> Vec<Suggestion> {
    suggestions
        .iter()
        .filter(|suggestion| !object.contains_key(suggestion.key))
        .map(|suggestion| {
            let mut field_path = object_path.clone();
            field_path.push(PathElem::Key(suggestion.key.to_string()));
            Suggestion {
                path: jsonpath::display(&field_path),
                reason: suggestion.reason,
            }
        })
        .collect()
}

/// One [`Suggestion`] for each `licenses[]` entry of `component` that uses
/// a `license` object without an `id` (an SPDX identifier), typically a
/// free-text `license.name` instead.
fn collect_missing_license_ids(
    component_path: &ConcretePath,
    component: &Map<String, Value>,
) -> Vec<Suggestion> {
    let Some(licenses) = component.get("licenses").and_then(Value::as_array) else {
        return Vec::new();
    };

    licenses
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            // An `{ "expression": "..." }` entry (or anything malformed) has
            // no `license` object to check: nothing to suggest here.
            let license = entry.get("license").and_then(Value::as_object)?;
            if license.contains_key("id") {
                return None;
            }

            let mut field_path = component_path.clone();
            field_path.push(PathElem::Key("licenses".to_string()));
            field_path.push(PathElem::Index(index));
            field_path.push(PathElem::Key("license".to_string()));
            field_path.push(PathElem::Key("id".to_string()));
            Some(Suggestion {
                path: jsonpath::display(&field_path),
                reason: "a SPDX license id lets tooling match this license automatically, instead of guessing intent from free text",
            })
        })
        .collect()
}

/// Experimental JSON output, entirely compiled out unless the `json-output`
/// Cargo feature is enabled (`cargo build --features json-output`).
#[cfg(feature = "json-output")]
mod json_output {
    use anyhow::{Context, Result};

    use super::Suggestion;

    /// The env var that switches `suggest`'s output to JSON, once the
    /// `json-output` feature is compiled in.
    const ENV_VAR: &str = "CYCLONELAB_SUGGEST_JSON";

    pub fn is_requested() -> bool {
        std::env::var_os(ENV_VAR).is_some_and(|value| value != "0")
    }

    pub fn print(suggestions: &[Suggestion]) -> Result<()> {
        let json = serde_json::to_string_pretty(suggestions)
            .context("Unable to serialize suggestions to JSON")?;
        println!("{json}");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn flags_missing_fields_on_a_top_level_component() {
        let doc = json!({
            "components": [
                { "type": "library", "name": "left-pad" }
            ]
        });

        let paths = component_paths(&doc);
        assert_eq!(paths.len(), 1);

        let component = jsonpath::get(&doc, &paths[0]).unwrap().as_object().unwrap();
        assert!(!component.contains_key("supplier"));
        assert!(!component.contains_key("licenses"));
    }

    #[test]
    fn does_not_flag_a_field_that_is_present() {
        let doc = json!({
            "components": [
                { "type": "library", "name": "left-pad", "licenses": [] }
            ]
        });

        let paths = component_paths(&doc);
        let component = jsonpath::get(&doc, &paths[0]).unwrap().as_object().unwrap();
        assert!(component.contains_key("licenses"));
    }

    #[test]
    fn locates_metadata_component_and_nested_sub_components() {
        let doc = json!({
            "metadata": {
                "component": {
                    "type": "application",
                    "name": "my-app"
                }
            },
            "components": [
                {
                    "type": "library",
                    "name": "outer",
                    "components": [
                        { "type": "library", "name": "inner" }
                    ]
                }
            ]
        });

        let paths = component_paths(&doc);
        let rendered: Vec<String> = paths.iter().map(|p| jsonpath::display(p)).collect();

        assert!(rendered.contains(&"$.metadata.component".to_string()));
        assert!(rendered.contains(&"$.components[0]".to_string()));
        assert!(rendered.contains(&"$.components[0].components[0]".to_string()));
    }

    #[test]
    fn reports_the_json_path_of_each_missing_field() {
        let doc = json!({
            "metadata": {
                "component": { "type": "application", "name": "my-app" }
            }
        });

        let paths = component_paths(&doc);
        assert_eq!(
            paths,
            vec![jsonpath::literal("$.metadata.component").unwrap()]
        );
    }

    fn missing_metadata_paths(doc: &Value) -> Vec<String> {
        let metadata = doc.get("metadata").unwrap().as_object().unwrap();
        collect_missing_fields(
            &jsonpath::literal("$.metadata").unwrap(),
            metadata,
            METADATA_SUGGESTIONS,
        )
        .into_iter()
        .map(|suggestion| suggestion.path)
        .collect()
    }

    #[test]
    fn flags_a_metadata_without_a_supplier() {
        let doc = json!({ "metadata": { "timestamp": "2024-01-01T00:00:00Z" } });
        assert!(missing_metadata_paths(&doc).contains(&"$.metadata.supplier".to_string()));
    }

    #[test]
    fn does_not_flag_a_metadata_with_a_supplier() {
        let doc = json!({ "metadata": { "supplier": { "name": "Acme" } } });
        assert!(!missing_metadata_paths(&doc).contains(&"$.metadata.supplier".to_string()));
    }

    #[test]
    fn flags_a_metadata_without_lifecycles() {
        let doc = json!({ "metadata": { "timestamp": "2024-01-01T00:00:00Z" } });
        assert!(missing_metadata_paths(&doc).contains(&"$.metadata.lifecycles".to_string()));
    }

    #[test]
    fn does_not_flag_a_metadata_with_lifecycles() {
        let doc = json!({ "metadata": { "lifecycles": [{ "phase": "build" }] } });
        assert!(!missing_metadata_paths(&doc).contains(&"$.metadata.lifecycles".to_string()));
    }

    #[test]
    fn does_not_flag_a_complete_metadata() {
        let doc = json!({
            "metadata": {
                "supplier": { "name": "Acme" },
                "lifecycles": [{ "phase": "build" }]
            }
        });
        assert!(missing_metadata_paths(&doc).is_empty());
    }

    fn a_component_path() -> ConcretePath {
        vec![PathElem::Key("components".to_string()), PathElem::Index(0)]
    }

    #[test]
    fn flags_a_license_using_a_free_text_name_instead_of_a_spdx_id() {
        let component = json!({
            "licenses": [
                { "license": { "name": "Apache License 2.0" } }
            ]
        });
        let component = component.as_object().unwrap();

        let found = collect_missing_license_ids(&a_component_path(), component);
        assert!(!found.is_empty());
    }

    #[test]
    fn does_not_flag_a_license_with_a_spdx_id_or_an_expression() {
        let component = json!({
            "licenses": [
                { "license": { "id": "Apache-2.0" } },
                { "expression": "MIT OR Apache-2.0" }
            ]
        });
        let component = component.as_object().unwrap();

        let found = collect_missing_license_ids(&a_component_path(), component);
        assert!(found.is_empty());
    }
}
