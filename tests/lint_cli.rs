//! End-to-end integration tests for `cyclonelab lint`, invoking the compiled
//! binary. `lint` checks a transformation YAML file's well-formedness
//! without requiring an SBOM (see GitHub issue #44).

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn cyclonelab() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cyclonelab"))
}

fn run(dir: &Path, args: &[&str]) -> Output {
    cyclonelab()
        .current_dir(dir)
        .arg("lint")
        .args(args)
        .output()
        .expect("the cyclonelab binary must be able to run")
}

#[test]
fn accepts_a_well_formed_transform_file() {
    let dir = tempfile::tempdir().unwrap();
    let transform_file = dir.path().join("recipe.yaml");
    fs::write(
        &transform_file,
        "variables:\n  version:\n    value: \"1.2.3\"\nsteps:\n  \
         - id: set-version\n    action: add\n    target: $.metadata.component.version\n    value: \"{$version}\"\n",
    )
    .unwrap();

    let output = run(dir.path(), &[transform_file.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("looks valid"));
}

#[test]
fn fails_when_the_transform_file_is_missing() {
    let dir = tempfile::tempdir().unwrap();

    let output = run(dir.path(), &["missing-transform.yaml"]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Unable to find transformation file"));
}

#[test]
fn reports_the_line_and_column_of_malformed_yaml() {
    let dir = tempfile::tempdir().unwrap();
    let transform_file = dir.path().join("broken.yaml");
    fs::write(
        &transform_file,
        "steps:\n  - id: broken\n    action: [this is not valid\n",
    )
    .unwrap();

    let output = run(dir.path(), &[transform_file.to_str().unwrap()]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("broken.yaml:"), "stderr: {stderr}");
}

#[test]
fn rejects_an_unknown_action() {
    let dir = tempfile::tempdir().unwrap();
    let transform_file = dir.path().join("recipe.yaml");
    fs::write(
        &transform_file,
        "steps:\n  - id: bogus\n    action: not-a-real-action\n    target: $.a\n",
    )
    .unwrap();

    let output = run(dir.path(), &[transform_file.to_str().unwrap()]);

    assert!(!output.status.success());
}

#[test]
fn rejects_an_invalid_jsonpath_target() {
    let dir = tempfile::tempdir().unwrap();
    let transform_file = dir.path().join("recipe.yaml");
    fs::write(
        &transform_file,
        "steps:\n  - id: bad-target\n    action: add\n    target: \"$.a[\"\n    value: 1\n",
    )
    .unwrap();

    let output = run(dir.path(), &[transform_file.to_str().unwrap()]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("bad-target"), "stderr: {stderr}");
}

#[test]
fn rejects_a_duplicate_step_id() {
    let dir = tempfile::tempdir().unwrap();
    let transform_file = dir.path().join("recipe.yaml");
    fs::write(
        &transform_file,
        "steps:\n  \
         - id: dup\n    action: add\n    target: $.a\n    value: 1\n  \
         - id: dup\n    action: add\n    target: $.b\n    value: 2\n",
    )
    .unwrap();

    let output = run(dir.path(), &[transform_file.to_str().unwrap()]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("duplicate step id"), "stderr: {stderr}");
}

#[test]
fn rejects_a_variable_used_but_never_declared() {
    let dir = tempfile::tempdir().unwrap();
    let transform_file = dir.path().join("recipe.yaml");
    fs::write(
        &transform_file,
        "steps:\n  - id: typo\n    action: add\n    target: $.a\n    value: \"{$versoin}\"\n",
    )
    .unwrap();

    let output = run(dir.path(), &[transform_file.to_str().unwrap()]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("versoin") && stderr.contains("never declared"),
        "stderr: {stderr}"
    );
}

#[test]
fn warns_about_a_variable_declared_but_never_used() {
    let dir = tempfile::tempdir().unwrap();
    let transform_file = dir.path().join("recipe.yaml");
    fs::write(
        &transform_file,
        "variables:\n  unused:\n    value: whatever\nsteps:\n  - id: noop\n    action: add\n    target: $.a\n    value: 1\n",
    )
    .unwrap();

    let output = run(dir.path(), &[transform_file.to_str().unwrap()]);

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("unused") && stdout.contains("never used"),
        "stdout: {stdout}"
    );
}

#[test]
#[cfg(feature = "json-output")]
fn prints_json_when_the_env_var_is_set() {
    let dir = tempfile::tempdir().unwrap();
    let transform_file = dir.path().join("recipe.yaml");
    fs::write(
        &transform_file,
        "variables:\n  unused:\n    value: whatever\nsteps:\n  - id: noop\n    action: add\n    target: $.a\n    value: 1\n",
    )
    .unwrap();

    let output = cyclonelab()
        .current_dir(dir.path())
        .env("CYCLONELAB_LINT_JSON", "1")
        .arg("lint")
        .arg(transform_file.to_str().unwrap())
        .output()
        .expect("the cyclonelab binary must be able to run");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON: {e}\nstdout: {stdout}"));
    assert_eq!(json["valid"], true);
    let warnings = json["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].as_str().unwrap().contains("unused"));
}

#[test]
#[cfg(feature = "json-output")]
fn prints_a_json_error_when_a_hard_error_occurs_and_the_env_var_is_set() {
    let dir = tempfile::tempdir().unwrap();
    let transform_file = dir.path().join("recipe.yaml");
    fs::write(
        &transform_file,
        "steps:\n  - id: typo\n    action: add\n    target: $.a\n    value: \"{$versoin}\"\n",
    )
    .unwrap();

    let output = cyclonelab()
        .current_dir(dir.path())
        .env("CYCLONELAB_LINT_JSON", "1")
        .arg("lint")
        .arg(transform_file.to_str().unwrap())
        .output()
        .expect("the cyclonelab binary must be able to run");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).is_empty(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON: {e}\nstdout: {stdout}"));
    assert_eq!(json["valid"], false);
    let error = json["error"].as_str().unwrap();
    assert!(
        error.contains("versoin") && error.contains("never declared"),
        "error: {error}"
    );
}

#[test]
#[cfg(feature = "json-output")]
fn prints_a_json_error_when_the_transform_file_is_missing() {
    let dir = tempfile::tempdir().unwrap();

    let output = cyclonelab()
        .current_dir(dir.path())
        .env("CYCLONELAB_LINT_JSON", "1")
        .arg("lint")
        .arg("missing-transform.yaml")
        .output()
        .expect("the cyclonelab binary must be able to run");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON: {e}\nstdout: {stdout}"));
    assert_eq!(json["valid"], false);
    assert!(
        json["error"]
            .as_str()
            .unwrap()
            .contains("Unable to find transformation file")
    );
}

#[test]
#[cfg(feature = "json-output")]
fn does_not_print_json_when_the_env_var_is_unset() {
    let dir = tempfile::tempdir().unwrap();
    let transform_file = dir.path().join("recipe.yaml");
    fs::write(
        &transform_file,
        "steps:\n  - id: noop\n    action: add\n    target: $.a\n    value: 1\n",
    )
    .unwrap();

    let output = run(dir.path(), &[transform_file.to_str().unwrap()]);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("looks valid"));
    assert!(serde_json::from_str::<serde_json::Value>(&stdout).is_err());
}

#[test]
fn accepts_foreach_iteration_variables_without_declaring_them() {
    let dir = tempfile::tempdir().unwrap();
    let transform_file = dir.path().join("recipe.yaml");
    fs::write(
        &transform_file,
        "foreach:\n  dir: artifacts\n  pattern: \"*.zip\"\nsteps:\n  \
         - id: set-name\n    action: add\n    target: $.metadata.component.group\n    value: \"{$artifact_name}\"\n",
    )
    .unwrap();

    let output = run(dir.path(), &[transform_file.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
