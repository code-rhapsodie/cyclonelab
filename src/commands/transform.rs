//! `transform` subcommand: applies a declarative YAML transformation recipe
//! to a CycloneDX SBOM (see `doc/transform/README.md`).

use std::collections::HashMap;
use std::fs;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Args;
use serde::Deserialize;
use serde_json::Value;

use crate::cyclonedx::validation::validate_bom;
use crate::generator_tool;
use crate::transform_actions::{self, Action, Step, StepContext};

#[derive(Debug, Args)]
pub struct TransformArgs {
    /// CycloneDX SBOM (JSON) to transform.
    sbom_file: PathBuf,

    /// YAML file describing the transformation steps.
    transform_file: PathBuf,

    /// Path the transformed SBOM is written to (created or overwritten).
    output_file: PathBuf,

    /// Overrides a variable declared in `transform_file` (`name=value`). Repeatable.
    #[arg(long = "variable")]
    variables: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct TransformFile {
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    to: Option<String>,
    #[serde(default)]
    variables: HashMap<String, VariableDecl>,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
struct VariableDecl {
    #[serde(default)]
    env: Option<String>,
    #[serde(default)]
    value: Option<Value>,
    #[serde(default)]
    required: bool,
}

/// The outcome of resolving every declared variable: the ones that got a
/// value, plus the (non-required) ones that stayed undefined — kept around
/// only to detect a step that uses one of them (see
/// [`check_unresolved_variables_are_unused`]).
struct VariableResolution {
    resolved: Vec<(String, String)>,
    unresolved: Vec<String>,
}

pub fn run(args: &TransformArgs) -> Result<()> {
    if !args.sbom_file.is_file() {
        bail!("Unable to find SBOM file '{}'", args.sbom_file.display());
    }
    if !args.transform_file.is_file() {
        bail!(
            "Unable to find transformation file '{}'",
            args.transform_file.display()
        );
    }
    if let Some(output_dir) = non_empty_parent(&args.output_file)
        && !output_dir.is_dir()
    {
        bail!("Unable to find output directory '{}'", output_dir.display());
    }

    let mut document = load_and_validate_sbom(&args.sbom_file)?;
    let transform_file = load_transform_file(&args.transform_file)?;
    transform_actions::validate_steps(&transform_file.steps)?;

    if let Some(from) = &transform_file.from {
        let spec_version = document
            .get("specVersion")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if spec_version != from {
            bail!(
                "'{}' declares specVersion '{spec_version}', but '{}' expects '{from}'",
                args.sbom_file.display(),
                args.transform_file.display(),
            );
        }
    }

    let variables = resolve_variables(&transform_file.variables, &args.variables)?;
    check_unresolved_variables_are_unused(&transform_file.steps, &variables.unresolved)?;

    let base_dir = non_empty_parent(&args.transform_file)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    for step in &transform_file.steps {
        let substituted = transform_actions::substitute_vars(step, &variables.resolved)?;
        let ctx = StepContext::new(&base_dir, &substituted);

        substituted
            .apply(&mut document, &ctx)
            .with_context(|| format!("step '{}'", substituted.id))?;

        let outcome = validate_bom(&document).with_context(|| {
            format!("step '{}': unable to validate the document", substituted.id)
        })?;
        if !outcome.errors.is_empty() {
            println!(
                "step '{}' produced a document that does not conform to the CycloneDX {} schema ({} error{}):",
                substituted.id,
                outcome.spec_version,
                outcome.errors.len(),
                if outcome.errors.len() > 1 { "s" } else { "" }
            );
            for error in &outcome.errors {
                println!("  - {}: {}", error.instance_path, error.message);
            }
            bail!("step '{}' failed schema validation", substituted.id);
        }
    }

    generator_tool::register_as_tool(&mut document)
        .context("unable to register cyclonelab in 'metadata.tools'")?;

    let outcome = validate_bom(&document)
        .context("unable to validate the document after registering cyclonelab as a tool")?;
    if !outcome.errors.is_empty() {
        println!(
            "registering cyclonelab as a tool produced a document that does not conform to the CycloneDX {} schema ({} error{}):",
            outcome.spec_version,
            outcome.errors.len(),
            if outcome.errors.len() > 1 { "s" } else { "" }
        );
        for error in &outcome.errors {
            println!("  - {}: {}", error.instance_path, error.message);
        }
        bail!("registering cyclonelab as a tool failed schema validation");
    }

    let output = serde_json::to_string_pretty(&document)?;
    fs::write(&args.output_file, output)
        .with_context(|| format!("Unable to write '{}'", args.output_file.display()))?;

    println!("'{}' written.", args.output_file.display());
    Ok(())
}

fn non_empty_parent(path: &std::path::Path) -> Option<&std::path::Path> {
    path.parent().filter(|dir| !dir.as_os_str().is_empty())
}

fn load_and_validate_sbom(sbom_file: &std::path::Path) -> Result<Value> {
    let content = fs::read_to_string(sbom_file)
        .with_context(|| format!("Unable to read '{}'", sbom_file.display()))?;
    let document: Value = match serde_json::from_str(&content) {
        Ok(value) => value,
        Err(err) => {
            println!("'{}' is not valid JSON: {err}", sbom_file.display());
            bail!("Invalid JSON in '{}'", sbom_file.display());
        }
    };

    let outcome = validate_bom(&document)
        .with_context(|| format!("Unable to validate '{}'", sbom_file.display()))?;
    if !outcome.errors.is_empty() {
        println!(
            "'{}' does not conform to the CycloneDX {} schema ({} error{}):",
            sbom_file.display(),
            outcome.spec_version,
            outcome.errors.len(),
            if outcome.errors.len() > 1 { "s" } else { "" }
        );
        for error in &outcome.errors {
            println!("  - {}: {}", error.instance_path, error.message);
        }
        bail!("'{}' failed schema validation", sbom_file.display());
    }

    Ok(document)
}

fn load_transform_file(transform_file: &std::path::Path) -> Result<TransformFile> {
    let content = fs::read_to_string(transform_file)
        .with_context(|| format!("Unable to read '{}'", transform_file.display()))?;
    yaml_serde::from_str(&content).map_err(|err| {
        let (line, column) = err
            .location()
            .map(|loc| (loc.line(), loc.column()))
            .unwrap_or((0, 0));
        anyhow::anyhow!("{}:{line}:{column}: {err}", transform_file.display())
    })
}

fn resolve_variables(
    declared: &HashMap<String, VariableDecl>,
    overrides: &[String],
) -> Result<VariableResolution> {
    let overrides = parse_variable_overrides(overrides)?;

    let mut resolved = Vec::new();
    let mut unresolved = Vec::new();
    for (name, decl) in declared {
        let value = overrides
            .get(name.as_str())
            .map(|v| v.to_string())
            .or_else(|| {
                decl.env
                    .as_ref()
                    .and_then(|env_name| std::env::var(env_name).ok())
            })
            .or_else(|| decl.value.as_ref().map(scalar_to_string));

        match value {
            Some(value) => resolved.push((format!("${name}"), value)),
            None if decl.required => {
                resolved.push((format!("${name}"), prompt_for_variable(name)?))
            }
            None => unresolved.push(name.clone()),
        }
    }
    Ok(VariableResolution {
        resolved,
        unresolved,
    })
}

fn parse_variable_overrides(overrides: &[String]) -> Result<HashMap<&str, &str>> {
    overrides
        .iter()
        .map(|entry| {
            entry
                .split_once('=')
                .with_context(|| format!("invalid --variable '{entry}', expected 'name=value'"))
        })
        .collect()
}

fn scalar_to_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn prompt_for_variable(name: &str) -> Result<String> {
    if !std::io::stdin().is_terminal() {
        bail!(
            "variable '{name}' is required but could not be resolved, and standard input is not \
             interactive; pass --variable {name}=<value>"
        );
    }
    print!("Value for variable '{name}': ");
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

/// A variable that stayed unresolved (not required, no override/env/default)
/// is only a problem if some step actually references it — checked once,
/// before any step runs (see `doc/transform/README.md` §4).
fn check_unresolved_variables_are_unused(steps: &[Step], unresolved: &[String]) -> Result<()> {
    if unresolved.is_empty() {
        return Ok(());
    }
    let serialized = serde_json::to_string(steps)?;
    for name in unresolved {
        if serialized.contains(&format!("{{${name}}}")) {
            bail!(
                "variable '{name}' is used in a step but could not be resolved (declare a default \
                 'value', mark it 'required', or pass --variable {name}=<value>)"
            );
        }
    }
    Ok(())
}
