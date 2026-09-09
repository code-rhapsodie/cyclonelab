//! `validate` subcommand: checks that a file is well-formed JSON and
//! conforms to the CycloneDX JSON schema matching its `specVersion`.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Args;
use serde_json::Value;

use crate::cyclonedx::validation::validate_bom;

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

    let outcome = validate_bom(&instance)
        .with_context(|| format!("Unable to validate '{}'", args.file.display()))?;

    if outcome.errors.is_empty() {
        println!(
            "'{}' is a valid CycloneDX {} SBOM.",
            args.file.display(),
            outcome.spec_version
        );
        return Ok(());
    }

    println!(
        "'{}' does not conform to the CycloneDX {} schema ({} error{}):",
        args.file.display(),
        outcome.spec_version,
        outcome.errors.len(),
        if outcome.errors.len() > 1 { "s" } else { "" }
    );
    for error in &outcome.errors {
        println!("  - {}: {}", error.instance_path, error.message);
    }

    bail!("'{}' failed schema validation", args.file.display());
}
