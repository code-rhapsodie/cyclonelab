//! `transform` subcommand: applies a declarative YAML transformation recipe
//! to a CycloneDX SBOM (see `doc/transform/README.md`).

use std::collections::HashMap;
use std::fs;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Args;
use serde::Deserialize;
use serde_json::Value;

use crate::cyclonedx::validation::validate_bom;
use crate::generator_tool;
use crate::transform_actions::{self, Action, Step, StepContext};
use crate::util::template::{matches_single_wildcard, render};

/// Names of the ambient variables `foreach` injects for each matched file
/// (see `doc/transform/foreach.md` §"Variables d'itération"): reserved, so a
/// declared `variables:` entry cannot reuse one of them.
const FOREACH_VAR_NAMES: [&str; 3] = ["artifact_name", "artifact_stem", "artifact_path"];

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
    #[serde(default)]
    foreach: Option<ForeachDecl>,
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

/// `foreach:` root key (see `doc/transform/foreach.md`): repeats the whole
/// `steps` pipeline once per file found in `dir` matching `pattern`, instead
/// of running it once on a fixed `OUTPUT_FILE`.
#[derive(Debug, Deserialize)]
struct ForeachDecl {
    dir: PathBuf,
    pattern: String,
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
    check_foreach_variable_conflicts(transform_file.foreach.as_ref(), &transform_file.variables)?;

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

    match &transform_file.foreach {
        None => {
            run_pipeline(
                &mut document,
                &transform_file.steps,
                &base_dir,
                &variables.resolved,
            )?;
            write_sbom(&args.output_file, &document)
        }
        Some(foreach) => run_foreach(
            &document,
            &transform_file.steps,
            &base_dir,
            &variables.resolved,
            foreach,
            &args.output_file,
        ),
    }
}

/// Runs the `steps` pipeline once against `document` (already substituting
/// `{$var}` placeholders and revalidating after each step, see
/// `doc/transform/README.md` §2), then registers `cyclonelab` as a tool —
/// shared between the single-document path and each `foreach` iteration.
fn run_pipeline(
    document: &mut Value,
    steps: &[Step],
    base_dir: &Path,
    vars: &[(String, String)],
) -> Result<()> {
    for step in steps {
        let substituted = transform_actions::substitute_vars(step, vars)?;
        let ctx = StepContext::new(base_dir, &substituted);

        substituted
            .apply(document, &ctx)
            .with_context(|| format!("step '{}'", substituted.id))?;

        let outcome = validate_bom(document).with_context(|| {
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

    generator_tool::register_as_tool(document)
        .context("unable to register cyclonelab in 'metadata.tools'")?;

    let outcome = validate_bom(document)
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

    Ok(())
}

/// Scans `foreach.dir` for files matching `foreach.pattern` (sorted by name)
/// and, for each one, runs
/// the `steps` pipeline on a fresh clone of `document` with the iteration's
/// ambient variables (`$artifact_name`/`$artifact_stem`/`$artifact_path`)
/// added to `vars`, then writes the result to `output_template` rendered
/// with that same set of variables (see `doc/transform/foreach.md`).
fn run_foreach(
    document: &Value,
    steps: &[Step],
    base_dir: &Path,
    vars: &[(String, String)],
    foreach: &ForeachDecl,
    output_template: &Path,
) -> Result<()> {
    // Relative to `base_dir` (the transformation file's directory), like
    // `valueFrom.file`/`valueFrom.path` — an already-absolute `dir` is used
    // as-is (see `doc/transform/foreach.md` and `action-add.md`).
    let dir = base_dir.join(&foreach.dir);
    if !dir.is_dir() {
        bail!("Unable to find directory '{}'", dir.display());
    }

    let mut artifacts: Vec<PathBuf> = fs::read_dir(&dir)
        .with_context(|| format!("Unable to read '{}'", dir.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| matches_single_wildcard(name, &foreach.pattern))
        })
        .collect();
    artifacts.sort();

    if artifacts.is_empty() {
        bail!(
            "No file for '{}' was found in '{}'",
            foreach.pattern,
            dir.display()
        );
    }

    for artifact_path in &artifacts {
        // Absolute, so that it stays correct wherever it is later reused
        // (e.g. as `valueFrom.path`/`valueFrom.file`): `artifact_path`
        // already has `base_dir` baked in through `dir` above, and those
        // fields independently join `base_dir` onto whatever they are given
        // — a relative `artifact_path` would get `base_dir` applied twice.
        let artifact_path = std::path::absolute(artifact_path).with_context(|| {
            format!(
                "Unable to resolve an absolute path for '{}'",
                artifact_path.display()
            )
        })?;
        let artifact_name = artifact_path
            .file_name()
            .and_then(|name| name.to_str())
            .with_context(|| format!("Invalid file name '{}'", artifact_path.display()))?;
        let artifact_stem = artifact_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .with_context(|| format!("Invalid file name '{}'", artifact_path.display()))?;
        let artifact_path_str = artifact_path
            .to_str()
            .with_context(|| format!("Invalid file path '{}'", artifact_path.display()))?;

        let mut iteration_vars = vars.to_vec();
        iteration_vars.push(("$artifact_name".to_string(), artifact_name.to_string()));
        iteration_vars.push(("$artifact_stem".to_string(), artifact_stem.to_string()));
        iteration_vars.push(("$artifact_path".to_string(), artifact_path_str.to_string()));

        let mut iteration_document = document.clone();
        run_pipeline(&mut iteration_document, steps, base_dir, &iteration_vars)?;

        let output_file = render_output_path(output_template, &iteration_vars)?;
        write_sbom(&output_file, &iteration_document)?;
    }

    Ok(())
}

/// Checks that no `variables:` entry reuses one of `foreach`'s reserved
/// iteration variable names (see `doc/transform/foreach.md` §"Variables
/// d'itération") — checked once at load time, regardless of how many (if
/// any) files `foreach` will later match.
fn check_foreach_variable_conflicts(
    foreach: Option<&ForeachDecl>,
    declared: &HashMap<String, VariableDecl>,
) -> Result<()> {
    if foreach.is_none() {
        return Ok(());
    }
    for name in FOREACH_VAR_NAMES {
        if declared.contains_key(name) {
            bail!(
                "variable '{name}' conflicts with the 'foreach' iteration variable of the same name"
            );
        }
    }
    Ok(())
}

/// Renders `template` (an `OUTPUT_FILE` argument) with `vars`, the same way
/// a step's textual fields are substituted — used only when `foreach` is
/// present (see `doc/transform/foreach.md` §"OUTPUT_FILE devient un
/// gabarit"); without `foreach`, `OUTPUT_FILE` is used as-is.
fn render_output_path(template: &Path, vars: &[(String, String)]) -> Result<PathBuf> {
    let template_str = template
        .to_str()
        .with_context(|| format!("'{}' is not valid UTF-8", template.display()))?;
    let pairs: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    Ok(PathBuf::from(render(template_str, &pairs)))
}

fn write_sbom(output_file: &Path, document: &Value) -> Result<()> {
    let output = serde_json::to_string_pretty(document)?;
    fs::write(output_file, output)
        .with_context(|| format!("Unable to write '{}'", output_file.display()))?;

    println!("'{}' written.", output_file.display());
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
