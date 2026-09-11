//! End-to-end integration tests for `cyclonelab`, invoking the
//! compiled binary. Downloading the source archive is short-circuited via
//! `--source-hash` so these tests stay deterministic and don't depend on
//! the network (necessary to run in CI).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use sha2::{Digest, Sha256};

fn cyclonelab() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cyclonelab"))
}

/// Writes the repository's real template into `dir` and returns its path,
/// to be passed explicitly via `--template-path`.
fn write_template(dir: &Path) -> PathBuf {
    let template = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/templates/template-sbom.cdx.json"
    ));
    let template_path = dir.join("template-sbom.cdx.json");
    fs::write(&template_path, template).unwrap();
    template_path
}

fn run(dir: &Path, template_path: &Path, extra_args: &[&str]) -> Output {
    cyclonelab()
        .current_dir(dir)
        .args([
            "generate-extension-sbom",
            "--version",
            "1.2.3",
            "--php-version",
            "8.3",
            "--template-path",
        ])
        .arg(template_path)
        .args(extra_args)
        .output()
        .expect("the cyclonelab binary must be able to run")
}

#[test]
fn fails_when_template_file_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let template_path = dir.path().join("template-sbom.cdx.json");

    let output = run(dir.path(), &template_path, &[]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Unable to find template file"));
}

#[test]
fn fails_when_artifacts_dir_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let template_path = write_template(dir.path());

    let output = run(dir.path(), &template_path, &[]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Unable to find artifact folder"));
}

#[test]
fn warns_and_exits_successfully_when_no_artifact_matches() {
    let dir = tempfile::tempdir().unwrap();
    let template_path = write_template(dir.path());
    fs::create_dir(dir.path().join("artifacts")).unwrap();

    let output = run(dir.path(), &template_path, &[]);

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("No file for"));
}

#[test]
fn generates_a_sbom_next_to_each_matching_artifact() {
    let dir = tempfile::tempdir().unwrap();
    let template_path = write_template(dir.path());
    let artifacts_dir = dir.path().join("artifacts");
    fs::create_dir(&artifacts_dir).unwrap();

    let zip_path = artifacts_dir.join("cyclonelab-8.3-x64.zip");
    fs::write(&zip_path, b"fake zip content for test").unwrap();
    // A file that doesn't match the pattern must not be processed.
    fs::write(artifacts_dir.join("readme.txt"), b"not an artifact").unwrap();

    let fake_source_hash = "1".repeat(64);
    let output = run(
        dir.path(),
        &template_path,
        &["--source-hash", &fake_source_hash],
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let sbom_path = artifacts_dir.join("cyclonelab-8.3-x64-sbom.cdx.json");
    let sbom_content = fs::read_to_string(&sbom_path).expect("the SBOM must be generated");
    assert!(
        !sbom_content.contains("{@"),
        "a placeholder was not substituted"
    );

    let bom: serde_json::Value = serde_json::from_str(&sbom_content).unwrap();

    assert_eq!(bom["bomFormat"], "CycloneDX");
    assert_eq!(bom["specVersion"], "1.7");
    assert_eq!(bom["metadata"]["component"]["version"], "1.2.3");
    assert_eq!(
        bom["metadata"]["tools"]["components"][0]["name"],
        "cyclonelab"
    );
    assert_eq!(
        bom["metadata"]["tools"]["components"][0]["version"],
        env!("CYCLONELAB_VERSION")
    );

    let zip_content = fs::read(&zip_path).unwrap();
    let expected_distribution_hash: String = Sha256::digest(&zip_content)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let external_references = bom["metadata"]["component"]["externalReferences"]
        .as_array()
        .unwrap();

    let distribution_ref = external_references
        .iter()
        .find(|r| r["type"] == "distribution")
        .expect("the distribution reference must be present");
    assert_eq!(
        distribution_ref["hashes"][0]["content"],
        expected_distribution_hash
    );
    assert!(
        distribution_ref["url"]
            .as_str()
            .unwrap()
            .ends_with("/releases/download/1.2.3/cyclonelab-8.3-x64.zip")
    );

    let source_ref = external_references
        .iter()
        .find(|r| r["type"] == "source-distribution")
        .expect("the source reference must be present");
    assert_eq!(source_ref["hashes"][0]["content"], fake_source_hash);
}
