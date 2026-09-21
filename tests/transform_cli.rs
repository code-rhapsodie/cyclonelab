//! End-to-end integration tests for `cyclonelab transform`, invoking the
//! compiled binary (see `doc/transform/README.md` §8).

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn cyclonelab() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cyclonelab"))
}

fn repo_path(relative: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn run(dir: &Path, args: &[&str]) -> Output {
    cyclonelab()
        .current_dir(dir)
        .arg("transform")
        .args(args)
        .output()
        .expect("the cyclonelab binary must be able to run")
}

#[test]
fn upgrades_the_real_fixture_from_1_5_to_1_7_through_both_recipes() {
    let dir = tempfile::tempdir().unwrap();
    let sbom_1_5 = dir.path().join("sbom-1.5.json");
    fs::copy(repo_path("tests/fixtures/sbom-1.5.cdx.json"), &sbom_1_5).unwrap();

    let sbom_1_6 = dir.path().join("sbom-1.6.json");
    let output = run(
        dir.path(),
        &[
            sbom_1_5.to_str().unwrap(),
            repo_path("schema/upgrade-1.5-to-1.6.yaml")
                .to_str()
                .unwrap(),
            sbom_1_6.to_str().unwrap(),
        ],
    );
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let sbom_1_7 = dir.path().join("sbom-1.7.json");
    let output = run(
        dir.path(),
        &[
            sbom_1_6.to_str().unwrap(),
            repo_path("schema/upgrade-1.6-to-1.7.yaml")
                .to_str()
                .unwrap(),
            sbom_1_7.to_str().unwrap(),
        ],
    );
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let content = fs::read_to_string(&sbom_1_7).expect("the upgraded SBOM must be written");
    let bom: serde_json::Value = serde_json::from_str(&content).unwrap();

    assert_eq!(bom["specVersion"], "1.7");
    assert_eq!(
        bom["$schema"], "http://cyclonedx.org/schema/bom-1.7.schema.json",
        "the upgrade must point $schema at the target version's schema, even when the fixture never set it"
    );
    assert!(bom["metadata"].get("manufacture").is_none());
    assert!(bom["metadata"]["tools"].is_object());
    assert!(bom["metadata"]["tools"]["components"][0]["name"] == "cargo-cyclonedx");

    let components = bom["components"].as_array().unwrap();
    let with_author = components
        .iter()
        .find(|c| c["name"] == "example")
        .expect("the example component must survive the migration");
    assert!(with_author.get("author").is_none());
    assert_eq!(
        with_author["authors"][0]["name"],
        "Jane Doe <jane@example.com>"
    );
}

#[test]
fn an_upgrade_step_lets_a_later_step_target_a_field_that_only_exists_from_the_target_version() {
    let dir = tempfile::tempdir().unwrap();
    let sbom = dir.path().join("sbom-1.5.json");
    fs::copy(repo_path("tests/fixtures/sbom-1.5.cdx.json"), &sbom).unwrap();

    let transform_file = dir.path().join("upgrade-then-add.yaml");
    fs::write(
        &transform_file,
        "steps:\n\
         \x20 - id: upgrade to 1.6\n\
         \x20   action: upgrade\n\
         \x20   version_target: \"1.6\"\n\
         \n\
         \x20 - id: add generator\n\
         \x20   action: merge\n\
         \x20   description: Add tools.\n\
         \x20   target: $.metadata.tools.components[]\n\
         \x20   value: '[{\"type\": \"application\", \"publisher\": \"test\", \"name\": \"cyclonedx\", \"version\": \"0.0.1\"}]'\n",
    )
    .unwrap();

    let output_file = dir.path().join("out.json");
    let output = run(
        dir.path(),
        &[
            sbom.to_str().unwrap(),
            transform_file.to_str().unwrap(),
            output_file.to_str().unwrap(),
        ],
    );

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let content = fs::read_to_string(&output_file).expect("the transformed SBOM must be written");
    let bom: serde_json::Value = serde_json::from_str(&content).unwrap();

    assert_eq!(bom["specVersion"], "1.6");
    let components = bom["metadata"]["tools"]["components"].as_array().unwrap();
    assert!(components.iter().any(|c| c["name"] == "cargo-cyclonedx"));
    assert!(components.iter().any(|c| c["name"] == "cyclonedx"));
}

#[test]
fn registers_cyclonelab_as_a_tool_alongside_the_sbom_s_existing_tools() {
    let dir = tempfile::tempdir().unwrap();
    let sbom = dir.path().join("sbom-1.5.json");
    fs::copy(repo_path("tests/fixtures/sbom-1.5.cdx.json"), &sbom).unwrap();

    let transform_file = dir.path().join("noop.yaml");
    fs::write(
        &transform_file,
        "steps:\n  - id: noop\n    action: add\n    target: $.metadata.timestamp\n    value: \"2020-01-01T00:00:00Z\"\n",
    )
    .unwrap();

    let output_file = dir.path().join("out.json");
    let output = run(
        dir.path(),
        &[
            sbom.to_str().unwrap(),
            transform_file.to_str().unwrap(),
            output_file.to_str().unwrap(),
        ],
    );
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let content = fs::read_to_string(&output_file).expect("the transformed SBOM must be written");
    let bom: serde_json::Value = serde_json::from_str(&content).unwrap();

    // The fixture keeps `metadata.tools` in its legacy (pre-1.5) array form,
    // still valid at specVersion 1.5: cyclonelab registers itself there in
    // the matching reduced shape rather than migrating the whole field.
    let tools = bom["metadata"]["tools"].as_array().unwrap();
    assert!(
        tools.iter().any(|c| c["name"] == "cargo-cyclonedx"),
        "the SBOM's original tool must survive: {tools:?}"
    );
    let cyclonelab = tools
        .iter()
        .find(|c| c["name"] == "cyclonelab")
        .expect("cyclonelab must register itself as a tool");
    assert_eq!(cyclonelab["vendor"], "Code Rhapsodie");
}

#[test]
fn fails_when_the_transform_file_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let sbom = dir.path().join("sbom.json");
    fs::copy(repo_path("tests/fixtures/sbom-1.5.cdx.json"), &sbom).unwrap();

    let output = run(
        dir.path(),
        &[sbom.to_str().unwrap(), "missing-transform.yaml", "out.json"],
    );

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Unable to find transformation file"));
    assert!(!dir.path().join("out.json").exists());
}

#[test]
fn reports_the_line_and_column_of_malformed_yaml() {
    let dir = tempfile::tempdir().unwrap();
    let sbom = dir.path().join("sbom.json");
    fs::copy(repo_path("tests/fixtures/sbom-1.5.cdx.json"), &sbom).unwrap();

    let transform_file = dir.path().join("broken.yaml");
    fs::write(
        &transform_file,
        "steps:\n  - id: broken\n    action: [this is not valid\n",
    )
    .unwrap();

    let output = run(
        dir.path(),
        &[
            sbom.to_str().unwrap(),
            transform_file.to_str().unwrap(),
            "out.json",
        ],
    );

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("broken.yaml:"), "stderr: {stderr}");
    assert!(!dir.path().join("out.json").exists());
}

#[test]
fn fails_when_from_does_not_match_the_sbom_spec_version() {
    let dir = tempfile::tempdir().unwrap();
    let sbom = dir.path().join("sbom.json");
    fs::copy(repo_path("tests/fixtures/sbom-1.5.cdx.json"), &sbom).unwrap();

    let transform_file = dir.path().join("mismatched-from.yaml");
    fs::write(
        &transform_file,
        "from: \"1.6\"\nto: \"1.7\"\nsteps:\n  - id: noop\n    action: add\n    target: $.metadata.timestamp\n    value: \"2020-01-01T00:00:00Z\"\n",
    )
    .unwrap();

    let output = run(
        dir.path(),
        &[
            sbom.to_str().unwrap(),
            transform_file.to_str().unwrap(),
            "out.json",
        ],
    );

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("specVersion"), "stderr: {stderr}");
    assert!(!dir.path().join("out.json").exists());
}

#[test]
fn does_not_write_the_output_file_when_a_step_breaks_schema_validation() {
    let dir = tempfile::tempdir().unwrap();
    let sbom = dir.path().join("sbom.json");
    fs::copy(repo_path("tests/fixtures/sbom-1.5.cdx.json"), &sbom).unwrap();

    let transform_file = dir.path().join("breaks-validation.yaml");
    fs::write(
        &transform_file,
        "steps:\n  - id: corrupt-spec-version\n    action: add\n    target: $.specVersion\n    value: \"not-a-real-version\"\n",
    )
    .unwrap();

    let output_file = dir.path().join("out.json");
    let output = run(
        dir.path(),
        &[
            sbom.to_str().unwrap(),
            transform_file.to_str().unwrap(),
            output_file.to_str().unwrap(),
        ],
    );

    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("corrupt-spec-version"),
        "output: {combined}"
    );
    assert!(
        !output_file.exists(),
        "OUTPUT_FILE must not be written when a step fails validation"
    );
}

#[test]
fn foreach_produces_one_output_file_per_matching_artifact() {
    let dir = tempfile::tempdir().unwrap();
    let sbom = dir.path().join("sbom.json");
    fs::copy(repo_path("tests/fixtures/sbom-1.5.cdx.json"), &sbom).unwrap();

    fs::create_dir_all(dir.path().join("artifacts")).unwrap();
    fs::write(dir.path().join("artifacts/a.zip"), b"a").unwrap();
    fs::write(dir.path().join("artifacts/b.zip"), b"b").unwrap();
    fs::write(dir.path().join("artifacts/note.txt"), b"not a zip").unwrap();
    fs::create_dir_all(dir.path().join("dist")).unwrap();

    let transform_file = dir.path().join("recipe.yaml");
    fs::write(
        &transform_file,
        "foreach:\n  dir: artifacts\n  pattern: \"*.zip\"\nsteps:\n  - id: noop\n    action: add\n    target: $.metadata.timestamp\n    value: \"2020-01-01T00:00:00Z\"\n",
    )
    .unwrap();

    let output = run(
        dir.path(),
        &[
            sbom.to_str().unwrap(),
            transform_file.to_str().unwrap(),
            "dist/{$artifact_stem}-sbom.cdx.json",
        ],
    );
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(dir.path().join("dist/a-sbom.cdx.json").is_file());
    assert!(dir.path().join("dist/b-sbom.cdx.json").is_file());
    assert!(!dir.path().join("dist/note-sbom.cdx.json").exists());
}

#[test]
fn foreach_fails_and_writes_nothing_when_no_file_matches() {
    let dir = tempfile::tempdir().unwrap();
    let sbom = dir.path().join("sbom.json");
    fs::copy(repo_path("tests/fixtures/sbom-1.5.cdx.json"), &sbom).unwrap();

    fs::create_dir_all(dir.path().join("artifacts")).unwrap();
    fs::write(dir.path().join("artifacts/note.txt"), b"not a zip").unwrap();
    fs::create_dir_all(dir.path().join("dist")).unwrap();

    let transform_file = dir.path().join("recipe.yaml");
    fs::write(
        &transform_file,
        "foreach:\n  dir: artifacts\n  pattern: \"*.zip\"\nsteps:\n  - id: noop\n    action: add\n    target: $.metadata.timestamp\n    value: \"2020-01-01T00:00:00Z\"\n",
    )
    .unwrap();

    let output = run(
        dir.path(),
        &[
            sbom.to_str().unwrap(),
            transform_file.to_str().unwrap(),
            "dist/{$artifact_stem}-sbom.cdx.json",
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let expected_dir = dir.path().join("artifacts");
    assert!(
        stderr.contains(&format!(
            "No file for '*.zip' was found in '{}'",
            expected_dir.display()
        )),
        "stderr: {stderr}"
    );
    assert_eq!(
        fs::read_dir(dir.path().join("dist")).unwrap().count(),
        0,
        "no output file must be written when nothing matches"
    );
}

#[test]
fn foreach_iteration_variables_are_substituted_in_a_step_and_in_output_file() {
    let dir = tempfile::tempdir().unwrap();
    let sbom = dir.path().join("sbom.json");
    fs::copy(repo_path("tests/fixtures/sbom-1.5.cdx.json"), &sbom).unwrap();

    fs::create_dir_all(dir.path().join("artifacts")).unwrap();
    fs::write(
        dir.path().join("artifacts/cyclonelab-linux-x86_64.zip"),
        b"a",
    )
    .unwrap();
    fs::create_dir_all(dir.path().join("dist")).unwrap();

    let transform_file = dir.path().join("recipe.yaml");
    fs::write(
        &transform_file,
        "foreach:\n  dir: artifacts\n  pattern: \"cyclonelab*\"\nsteps:\n  \
         - id: set-name\n    action: add\n    target: $.metadata.component.group\n    value: \"{$artifact_name}\"\n  \
         - id: set-stem\n    action: add\n    target: $.metadata.component.version\n    value: \"{$artifact_stem}\"\n  \
         - id: set-path\n    action: add\n    target: $.metadata.component.description\n    value: \"{$artifact_path}\"\n",
    )
    .unwrap();

    let output = run(
        dir.path(),
        &[
            sbom.to_str().unwrap(),
            transform_file.to_str().unwrap(),
            "dist/{$artifact_stem}-sbom.cdx.json",
        ],
    );
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let output_file = dir
        .path()
        .join("dist/cyclonelab-linux-x86_64-sbom.cdx.json");
    let content =
        fs::read_to_string(&output_file).expect("the templated OUTPUT_FILE must be written");
    let bom: serde_json::Value = serde_json::from_str(&content).unwrap();
    let component = &bom["metadata"]["component"];

    assert_eq!(component["group"], "cyclonelab-linux-x86_64.zip");
    assert_eq!(component["version"], "cyclonelab-linux-x86_64");
    let description = component["description"].as_str().unwrap();
    assert!(
        description.ends_with("artifacts/cyclonelab-linux-x86_64.zip")
            || description.ends_with("artifacts\\cyclonelab-linux-x86_64.zip"),
        "description: {description}"
    );
}

#[test]
fn foreach_artifact_path_stays_correct_when_the_process_runs_outside_the_transform_files_directory()
{
    // Regression test for issue #36: `foreach.dir` and `valueFrom.path` are
    // both resolved relative to the transformation file's directory (see
    // `doc/transform/foreach.md`), which can differ from the process's
    // current directory. `{$artifact_path}` must stay correct either way.
    let dir = tempfile::tempdir().unwrap();
    let sbom = dir.path().join("sbom.json");
    fs::copy(repo_path("tests/fixtures/sbom-1.5.cdx.json"), &sbom).unwrap();

    fs::create_dir_all(dir.path().join("recipe/artifacts")).unwrap();
    fs::write(
        dir.path().join("recipe/artifacts/artifact.bin"),
        b"hello world",
    )
    .unwrap();

    let transform_file = dir.path().join("recipe/recipe.yaml");
    fs::write(
        &transform_file,
        "foreach:\n  dir: artifacts\n  pattern: \"*.bin\"\nsteps:\n  \
         - id: hash\n    action: add\n    target: $.metadata.component.hashes\n    \
         valueFrom: {generator: hash, algo: sha256, path: \"{$artifact_path}\"}\n    \
         value: [{alg: \"SHA-256\", content: \"{@value}\"}]\n",
    )
    .unwrap();

    let output_file = dir.path().join("sbom-out.json");
    let output = run(
        dir.path(),
        &[
            sbom.to_str().unwrap(),
            transform_file.to_str().unwrap(),
            output_file.to_str().unwrap(),
        ],
    );
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let content = fs::read_to_string(&output_file).unwrap();
    let bom: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(
        bom["metadata"]["component"]["hashes"],
        // sha256("hello world"), verified with `sha256sum` outside this test
        // (same digest as `add.rs`'s `HELLO_WORLD_SHA256`).
        serde_json::json!([{
            "alg": "SHA-256",
            "content": "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9",
        }])
    );
}

#[test]
fn foreach_iterations_do_not_contaminate_each_other() {
    let dir = tempfile::tempdir().unwrap();
    let sbom = dir.path().join("sbom.json");
    fs::copy(repo_path("tests/fixtures/sbom-1.5.cdx.json"), &sbom).unwrap();

    fs::create_dir_all(dir.path().join("artifacts")).unwrap();
    fs::write(dir.path().join("artifacts/a.zip"), b"a").unwrap();
    fs::write(dir.path().join("artifacts/b.zip"), b"b").unwrap();
    fs::create_dir_all(dir.path().join("dist")).unwrap();

    let transform_file = dir.path().join("recipe.yaml");
    fs::write(
        &transform_file,
        "foreach:\n  dir: artifacts\n  pattern: \"*.zip\"\nsteps:\n  \
         - id: append-marker\n    action: merge\n    target: $.components[]\n    value: '[{\"type\": \"file\", \"bom-ref\": \"{$artifact_name}\", \"name\": \"{$artifact_name}\"}]'\n",
    )
    .unwrap();

    let output = run(
        dir.path(),
        &[
            sbom.to_str().unwrap(),
            transform_file.to_str().unwrap(),
            "dist/{$artifact_stem}-sbom.cdx.json",
        ],
    );
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    for stem in ["a", "b"] {
        let content =
            fs::read_to_string(dir.path().join(format!("dist/{stem}-sbom.cdx.json"))).unwrap();
        let bom: serde_json::Value = serde_json::from_str(&content).unwrap();
        let components = bom["components"].as_array().unwrap();
        assert_eq!(
            components.len(),
            2,
            "iteration for '{stem}' must start from the original document (1 component) plus its own marker, not accumulate previous iterations': {components:?}"
        );
    }
}

#[test]
fn foreach_rejects_a_declared_variable_that_collides_with_an_iteration_variable_name() {
    let dir = tempfile::tempdir().unwrap();
    let sbom = dir.path().join("sbom.json");
    fs::copy(repo_path("tests/fixtures/sbom-1.5.cdx.json"), &sbom).unwrap();

    fs::create_dir_all(dir.path().join("artifacts")).unwrap();

    let transform_file = dir.path().join("recipe.yaml");
    fs::write(
        &transform_file,
        "foreach:\n  dir: artifacts\n  pattern: \"*.zip\"\nvariables:\n  artifact_name:\n    value: whatever\nsteps:\n  - id: noop\n    action: add\n    target: $.metadata.timestamp\n    value: \"2020-01-01T00:00:00Z\"\n",
    )
    .unwrap();

    let output = run(
        dir.path(),
        &[
            sbom.to_str().unwrap(),
            transform_file.to_str().unwrap(),
            "out.json",
        ],
    );

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("artifact_name"), "stderr: {stderr}");
    assert!(!dir.path().join("out.json").exists());
}
