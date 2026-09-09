//! CycloneDX JSON schema validation, shared by the `validate` and
//! `transform` commands: extracts `specVersion` from a document, picks the
//! matching bundled schema, and validates against it.

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

    let errors = validator
        .iter_errors(instance)
        .map(|error| SchemaError {
            instance_path: error.instance_path().to_string(),
            message: error.to_string(),
        })
        .collect();

    Ok(ValidationOutcome {
        spec_version: spec_version.to_string(),
        errors,
    })
}
