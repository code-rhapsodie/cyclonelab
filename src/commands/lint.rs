//! `lint` subcommand: statically checks a transformation YAML file for
//! well-formedness, without requiring an SBOM (see
//! `doc/transform/README.md`).

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
    check_variables(&transform_file)?;

    println!("'{}' looks valid.", args.transform_file.display());
    Ok(())
}

/// Cross-checks declared variables against every `{$name}` placeholder used
/// across the file's steps: a name used but never declared (almost always a
/// typo, since `util::template::render` leaves an unknown placeholder
/// untouched instead of failing) is a hard error; a declared variable never
/// referenced is only a warning.
fn check_variables(transform_file: &transform::TransformFile) -> Result<()> {
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

    for name in transform_file.variables.keys() {
        if !used.contains(name.as_str()) {
            println!("warning: variable '{name}' is declared but never used");
        }
    }

    Ok(())
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
