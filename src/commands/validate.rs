//! `validate` subcommand: checks that a file is well-formed JSON and
//! conforms to the CycloneDX JSON schema matching its `specVersion`.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Args;
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

#[derive(Debug, Args)]
pub struct ValidateArgs {
    /// SBOM file (JSON) to validate.
    file: PathBuf,
}

pub fn run(args: &ValidateArgs) -> Result<()> {
    let content = fs::read_to_string(&args.file)
        .with_context(|| format!("Unable to read '{}'", args.file.display()))?;

    let instance: Value = match serde_json::from_str(&content) {
        Ok(value) => value,
        Err(err) => {
            println!("'{}' is not valid JSON: {err}", args.file.display());
            bail!("Invalid JSON in '{}'", args.file.display());
        }
    };

    let spec_version = instance
        .get("specVersion")
        .and_then(Value::as_str)
        .with_context(|| {
            format!(
                "'{}' has no (or a non-string) 'specVersion' field",
                args.file.display()
            )
        })?;

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

    let errors: Vec<_> = validator.iter_errors(&instance).collect();

    if errors.is_empty() {
        println!(
            "'{}' is a valid CycloneDX {spec_version} SBOM.",
            args.file.display()
        );
        return Ok(());
    }

    println!(
        "'{}' does not conform to the CycloneDX {spec_version} schema ({} error{}):",
        args.file.display(),
        errors.len(),
        if errors.len() > 1 { "s" } else { "" }
    );
    for error in &errors {
        println!("  - {}: {}", error.instance_path(), error);
    }

    bail!("'{}' failed schema validation", args.file.display());
}
