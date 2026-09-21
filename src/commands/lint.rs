//! `lint` subcommand: statically checks a transformation YAML file for
//! well-formedness, without requiring an SBOM (see
//! `doc/transform/README.md`).
//!
//! ## Experimental JSON output
//!
//! Behind the `json-output` Cargo feature (off by default: not part of the
//! stable CLI yet), setting `CYCLONELAB_LINT_JSON=1` switches the output to
//! JSON instead of plain text lines: `{"valid": true, "warnings": [...]}` on
//! success, or `{"valid": false, "error": "..."}` on a hard error (malformed
//! YAML, an unknown action...), printed to stdout before the process exits
//! with a non-zero status. Without that feature compiled in, the environment
//! variable is never even looked at, and errors are reported the usual way
//! (plain text on stderr, via `anyhow`).

use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Args;

use crate::commands::transform;
use crate::transform_actions;

#[derive(Debug, Args)]
pub struct LintArgs {
    /// YAML file describing the transformation steps.
    transform_file: PathBuf,
}

pub fn run(args: &LintArgs) -> Result<()> {
    match run_checks(args) {
        Ok(warnings) => {
            #[cfg(feature = "json-output")]
            if json_output::is_requested() {
                return json_output::print_success(&warnings);
            }

            print_text(&args.transform_file, &warnings);
            Ok(())
        }
        Err(err) => {
            #[cfg(feature = "json-output")]
            if json_output::is_requested() {
                json_output::print_error(&err)?;
                std::process::exit(1);
            }

            Err(err)
        }
    }
}

/// Runs every well-formedness check and returns the "declared but never
/// used" variable warnings on success, or the first hard error encountered.
fn run_checks(args: &LintArgs) -> Result<Vec<String>> {
    if !args.transform_file.is_file() {
        bail!(
            "Unable to find transformation file '{}'",
            args.transform_file.display()
        );
    }

    let transform_file = transform::load_transform_file(&args.transform_file)?;

    transform_actions::validate_steps(&transform_file.steps)?;
    transform_actions::lint_steps(&transform_file.steps)?;
    transform::check_foreach_variable_conflicts(
        transform_file.foreach.as_ref(),
        &transform_file.variables,
    )?;
    check_variables(&transform_file)
}

fn print_text(transform_file: &std::path::Path, warnings: &[String]) {
    for warning in warnings {
        println!("warning: {warning}");
    }
    println!("'{}' looks valid.", transform_file.display());
}

/// Cross-checks declared variables against every `{$name}` placeholder used
/// across the file's steps: a name used but never declared (almost always a
/// typo, since `util::template::render` leaves an unknown placeholder
/// untouched instead of failing) is a hard error; a declared variable never
/// referenced is only a warning, returned so the caller can render it as
/// plain text or JSON.
fn check_variables(transform_file: &transform::TransformFile) -> Result<Vec<String>> {
    let mut declared: HashSet<String> = transform_file.variables.keys().cloned().collect();
    if transform_file.foreach.is_some() {
        declared.extend(transform::FOREACH_VAR_NAMES.iter().map(|s| s.to_string()));
    }

    let serialized = serde_json::to_string(&transform_file.steps)?;
    let used = referenced_variable_names(&serialized);

    for name in &used {
        if !declared.contains(name.as_str()) {
            bail!("variable '{name}' is used in a step but never declared in 'variables:'");
        }
    }

    Ok(transform_file
        .variables
        .keys()
        .filter(|name| !used.contains(name.as_str()))
        .map(|name| format!("variable '{name}' is declared but never used"))
        .collect())
}

/// Every `{$name}` placeholder found in `text` (e.g. every step,
/// JSON-serialized), regardless of whether it resolves to a declared
/// variable.
fn referenced_variable_names(text: &str) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut rest = text;
    while let Some(start) = rest.find("{$") {
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            break;
        };
        let candidate = &after[..end];
        if !candidate.is_empty()
            && candidate
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            names.insert(candidate.to_string());
        }
        rest = &after[end + 1..];
    }
    names
}

/// Experimental JSON output, entirely compiled out unless the `json-output`
/// Cargo feature is enabled (`cargo build --features json-output`).
#[cfg(feature = "json-output")]
mod json_output {
    use anyhow::{Context, Result};
    use serde::Serialize;

    /// The env var that switches `lint`'s output to JSON, once the
    /// `json-output` feature is compiled in.
    const ENV_VAR: &str = "CYCLONELAB_LINT_JSON";

    #[derive(Serialize)]
    struct SuccessReport<'a> {
        valid: bool,
        warnings: &'a [String],
    }

    #[derive(Serialize)]
    struct ErrorReport {
        valid: bool,
        error: String,
    }

    pub fn is_requested() -> bool {
        std::env::var_os(ENV_VAR).is_some_and(|value| value != "0")
    }

    pub fn print_success(warnings: &[String]) -> Result<()> {
        let report = SuccessReport {
            valid: true,
            warnings,
        };
        let json = serde_json::to_string_pretty(&report)
            .context("Unable to serialize lint report to JSON")?;
        println!("{json}");
        Ok(())
    }

    /// Prints `err` (with its full context chain) as `{"valid": false,
    /// "error": "..."}` to stdout. The caller is still responsible for
    /// exiting with a non-zero status: unlike `anyhow`'s default reporting,
    /// this never happens on its own.
    pub fn print_error(err: &anyhow::Error) -> Result<()> {
        let message = err
            .chain()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(": ");
        let report = ErrorReport {
            valid: false,
            error: message,
        };
        let json = serde_json::to_string_pretty(&report)
            .context("Unable to serialize lint error report to JSON")?;
        println!("{json}");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn referenced_variable_names_finds_every_placeholder() {
        let names = referenced_variable_names(r#"{"target":"{$repo}/{$version}"}"#);
        assert_eq!(
            names,
            HashSet::from(["repo".to_string(), "version".to_string()])
        );
    }

    #[test]
    fn referenced_variable_names_ignores_a_dollar_not_forming_a_placeholder() {
        assert!(referenced_variable_names("no placeholder here").is_empty());
    }
}
