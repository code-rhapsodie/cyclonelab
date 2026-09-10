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
    assert!(
        bom.get("$schema").is_none(),
        "the fixture never set $schema, it must stay absent"
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
