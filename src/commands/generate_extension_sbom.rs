//! Reimplementation of `Generate-ExtensionSbom.ps1`: instantiates a
//! CycloneDX SBOM template for each compiled PHP extension archive found
//! in an artifact folder.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use chrono::Utc;
use clap::Args;
use uuid::Uuid;

use crate::cyclonedx::Bom;
use crate::generator_tool;
use crate::util::download::download_file;
use crate::util::hashing::sha256_file;
use crate::util::template::{matches_single_wildcard, render};

#[derive(Debug, Args)]
pub struct GenerateExtensionSbomArgs {
    /// Release version (e.g. "2.3.0"), used to build the source and archive
    /// URLs, and injected into the template via `{@version}`.
    #[arg(long)]
    version: String,

    /// PHP version targeted by the extension, injected via `{@php_version}`.
    #[arg(long)]
    php_version: String,

    /// CycloneDX SBOM template to instantiate.
    #[arg(long, default_value = "template-sbom.cdx.json")]
    template_path: PathBuf,

    /// Folder containing the compiled extensions' ZIP archives.
    #[arg(long, default_value = "artifacts")]
    artifacts_dir: PathBuf,

    /// GitHub repository `owner/name` publishing the sources and releases.
    #[arg(long, default_value = "code-rhapsodie/cyclonelab")]
    repo: String,

    /// Pattern (a single `*`) of the archives to process in `artifacts_dir`.
    #[arg(long, default_value = "cyclonelab*")]
    artifact_pattern: String,

    /// SHA-256 of the source archive, if already known: if provided, the
    /// archive is not re-downloaded. Useful for reproducible/offline runs,
    /// and for tests (see `tests/generate_extension_sbom_cli.rs`).
    #[arg(long)]
    source_hash: Option<String>,
}

/// Values substituted into the template (`{@name}`).
pub struct SbomPlaceholders<'a> {
    pub version: &'a str,
    pub php_version: &'a str,
    pub file_uuid: &'a str,
    pub source_url: &'a str,
    pub source_hash: &'a str,
    pub distribution_url: &'a str,
    pub distribution_hash: &'a str,
    pub date_now: &'a str,
}

impl<'a> SbomPlaceholders<'a> {
    fn as_pairs(&self) -> [(&'static str, &'a str); 8] {
        [
            ("@version", self.version),
            ("@file_uuid", self.file_uuid),
            ("@source_url", self.source_url),
            ("@source_hash", self.source_hash),
            ("@distribution_url", self.distribution_url),
            ("@distribution_hash", self.distribution_hash),
            ("@date_now", self.date_now),
            ("@php_version", self.php_version),
        ]
    }
}

/// Instantiates an SBOM template: placeholder substitution, parsing into
/// [`Bom`], then registering this generator as the sole tool in
/// `metadata.tools`. A pure function (no I/O), so easily testable.
pub fn instantiate_sbom(template_content: &str, placeholders: &SbomPlaceholders) -> Result<Bom> {
    let rendered = render(template_content, &placeholders.as_pairs());
    let mut bom: Bom = serde_json::from_str(&rendered).context(
        "Invalid generated SBOM: JSON is not well-formed after placeholder substitution",
    )?;
    generator_tool::set_as_sole_tool(&mut bom);
    Ok(bom)
}

pub fn run(args: &GenerateExtensionSbomArgs) -> Result<()> {
    if !args.template_path.is_file() {
        bail!(
            "Unable to find template file '{}'",
            args.template_path.display()
        );
    }
    if !args.artifacts_dir.is_dir() {
        bail!(
            "Unable to find artifact folder '{}'",
            args.artifacts_dir.display()
        );
    }

    let mut zip_files: Vec<PathBuf> = fs::read_dir(&args.artifacts_dir)
        .with_context(|| format!("Unable to read '{}'", args.artifacts_dir.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| matches_single_wildcard(name, &args.artifact_pattern))
        })
        .collect();
    zip_files.sort();

    if zip_files.is_empty() {
        println!(
            "Warning: No file for '{}' was found in '{}'.",
            args.artifact_pattern,
            args.artifacts_dir.display()
        );
        return Ok(());
    }

    let source_url = format!(
        "https://github.com/{}/archive/refs/tags/{}.zip",
        args.repo, args.version
    );

    let source_hash = match &args.source_hash {
        Some(hash) => {
            println!("Using provided source hash (no download) : {hash}");
            hash.clone()
        }
        None => {
            println!("Download sources archive : {source_url}");
            let temp_source_zip = std::env::temp_dir().join(format!("{}.zip", Uuid::new_v4()));
            let hash = download_and_hash(&source_url, &temp_source_zip)?;
            println!("Source archive Hash SHA256 : {hash}");
            hash
        }
    };

    let template_content = fs::read_to_string(&args.template_path)
        .with_context(|| format!("Unable to read '{}'", args.template_path.display()))?;

    for zip_path in &zip_files {
        let file_name = zip_path
            .file_name()
            .and_then(|n| n.to_str())
            .context("Invalid file name")?;

        println!("\nArtifact processing : {file_name}");

        let file_uuid = Uuid::new_v4().to_string();
        let distribution_url = format!(
            "https://github.com/{}/releases/download/{}/{file_name}",
            args.repo, args.version
        );
        let distribution_hash = sha256_file(zip_path)?;
        let date_now = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

        let bom = instantiate_sbom(
            &template_content,
            &SbomPlaceholders {
                version: &args.version,
                php_version: &args.php_version,
                file_uuid: &file_uuid,
                source_url: &source_url,
                source_hash: &source_hash,
                distribution_url: &distribution_url,
                distribution_hash: &distribution_hash,
                date_now: &date_now,
            },
        )
        .with_context(|| format!("Failed to generate the SBOM for '{file_name}'"))?;
        let sbom_content = serde_json::to_string_pretty(&bom)?;

        let sbom_file_name = format!(
            "{}-sbom.cdx.json",
            zip_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(file_name)
        );
        let output_path = zip_path.with_file_name(sbom_file_name);
        fs::write(&output_path, sbom_content)
            .with_context(|| format!("Unable to write '{}'", output_path.display()))?;

        println!("SBOM generated : {}", output_path.display());
    }

    Ok(())
}

fn download_and_hash(url: &str, dest: &PathBuf) -> Result<String> {
    download_file(url, dest)?;
    let hash = sha256_file(dest);
    let _ = fs::remove_file(dest);
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The repository's actual template: these tests guarantee that this
    /// crate's CycloneDX model stays able to read and rewrite it losslessly,
    /// even as the template or the 1.7 schema evolve.
    const TEMPLATE: &str = include_str!("../../templates/template-sbom.cdx.json");

    fn sample_placeholders() -> SbomPlaceholders<'static> {
        SbomPlaceholders {
            version: "1.2.3",
            php_version: "8.3",
            file_uuid: "11111111-1111-1111-1111-111111111111",
            source_url: "https://example.test/source.zip",
            source_hash: "1111111111111111111111111111111111111111111111111111111111111111",
            distribution_url: "https://example.test/dist.zip",
            distribution_hash: "2222222222222222222222222222222222222222222222222222222222222222",
            date_now: "2026-01-01T00:00:00Z",
        }
    }

    #[test]
    fn parses_the_real_template_without_error() {
        let bom = instantiate_sbom(TEMPLATE, &sample_placeholders())
            .expect("the repository template must remain parseable");
        assert_eq!(bom.bom_format, "CycloneDX");
        assert_eq!(bom.spec_version, "1.7");
    }

    #[test]
    fn no_placeholder_survives_substitution() {
        let bom = instantiate_sbom(TEMPLATE, &sample_placeholders()).unwrap();
        let serialized = serde_json::to_string(&bom).unwrap();
        assert!(
            !serialized.contains("{@"),
            "a placeholder was not substituted: {serialized}"
        );
    }

    #[test]
    fn metadata_component_reflects_the_requested_version() {
        let placeholders = sample_placeholders();
        let bom = instantiate_sbom(TEMPLATE, &placeholders).unwrap();
        let component = bom.metadata.as_ref().unwrap().component.as_ref().unwrap();

        assert_eq!(component.version.as_deref(), Some(placeholders.version));
        assert_eq!(
            component.purl.as_deref(),
            Some("pkg:generic/code-rhapsodie/cyclonelab@1.2.3")
        );
        assert_eq!(
            component.bom_ref.as_deref(),
            Some("pkg:generic/code-rhapsodie/cyclonelab@1.2.3")
        );
        assert_eq!(component.group.as_deref(), Some("win32service"));
    }

    #[test]
    fn component_license_is_parsed_as_a_named_license() {
        let bom = instantiate_sbom(TEMPLATE, &sample_placeholders()).unwrap();
        let component = bom.metadata.as_ref().unwrap().component.as_ref().unwrap();

        assert_eq!(component.licenses.len(), 1);
        match &component.licenses[0] {
            crate::cyclonedx::LicenseChoice::Single { license } => {
                assert_eq!(license.name.as_deref(), Some("PHP License v3.01"));
                assert_eq!(license.acknowledgement.as_deref(), Some("declared"));
            }
            other => panic!("unexpected license: {other:?}"),
        }
    }

    #[test]
    fn generator_registers_itself_as_the_sole_tool() {
        let bom = instantiate_sbom(TEMPLATE, &sample_placeholders()).unwrap();
        let tools = bom
            .metadata
            .as_ref()
            .unwrap()
            .tools
            .as_ref()
            .expect("tools must be set");
        let components = tools
            .components
            .as_ref()
            .expect("tools.components must be set");

        assert_eq!(components.len(), 1);
        assert_eq!(components[0].name.as_deref(), Some("cyclonelab"));
        assert_eq!(
            components[0].version.as_deref(),
            Some(env!("CYCLONELAB_VERSION"))
        );
        assert_eq!(
            components[0].purl.as_deref(),
            Some(
                format!(
                    "pkg:generic/coderhapsodie/cyclonelab@{}",
                    env!("CYCLONELAB_VERSION")
                )
                .as_str()
            )
        );
    }

    #[test]
    fn unmodeled_component_fields_survive_the_roundtrip() {
        // cpe/authors/evidence/supplier are not explicitly typed in
        // `Component`: this test guarantees they are not lost during the
        // deserialization -> reserialization round trip.
        let bom = instantiate_sbom(TEMPLATE, &sample_placeholders()).unwrap();
        let component = bom.metadata.as_ref().unwrap().component.as_ref().unwrap();

        assert_eq!(
            component.extra.get("cpe").and_then(|v| v.as_str()),
            Some("cpe:2.3:a:win32service:win32service:1.2.3:*:*:*:*:*:*:*")
        );
        assert!(
            component.extra.get("authors").is_some(),
            "authors must be preserved"
        );
        assert!(
            component.extra.get("evidence").is_some(),
            "evidence must be preserved"
        );
        assert!(
            component.extra.get("supplier").is_some(),
            "supplier must be preserved"
        );
    }

    #[test]
    fn top_level_dependencies_and_components_survive_and_are_substituted() {
        // `Bom` does not type `dependencies`/`components`/`vulnerabilities`:
        // verifies they are preserved in `extra`, with placeholders replaced.
        let bom = instantiate_sbom(TEMPLATE, &sample_placeholders()).unwrap();

        let dependencies = bom.extra.get("dependencies").unwrap().as_array().unwrap();
        assert_eq!(
            dependencies[0]["ref"],
            "pkg:generic/code-rhapsodie/cyclonelab@1.2.3"
        );
        assert_eq!(dependencies[0]["dependsOn"][0], "pkg:generic/php@8.3");

        let components = bom.extra.get("components").unwrap().as_array().unwrap();
        assert_eq!(
            components[0]["versionRange"],
            "vers:semver/>=8.3.0|<=8.3.9999"
        );

        assert_eq!(
            bom.extra
                .get("vulnerabilities")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn external_references_carry_the_substituted_hashes_and_urls() {
        let placeholders = sample_placeholders();
        let bom = instantiate_sbom(TEMPLATE, &placeholders).unwrap();
        let component = bom.metadata.as_ref().unwrap().component.as_ref().unwrap();
        let refs = component
            .extra
            .get("externalReferences")
            .unwrap()
            .as_array()
            .unwrap();

        let source_ref = refs
            .iter()
            .find(|r| r["type"] == "source-distribution")
            .unwrap();
        assert_eq!(source_ref["url"], placeholders.source_url);
        assert_eq!(source_ref["hashes"][0]["content"], placeholders.source_hash);

        let dist_ref = refs.iter().find(|r| r["type"] == "distribution").unwrap();
        assert_eq!(dist_ref["url"], placeholders.distribution_url);
        assert_eq!(
            dist_ref["hashes"][0]["content"],
            placeholders.distribution_hash
        );
    }

    #[test]
    fn serial_number_uses_the_generated_file_uuid() {
        let bom = instantiate_sbom(TEMPLATE, &sample_placeholders()).unwrap();
        assert_eq!(
            bom.serial_number.as_deref(),
            Some("urn:uuid:11111111-1111-1111-1111-111111111111")
        );
    }

    #[test]
    fn rejects_a_template_that_is_not_valid_json() {
        let err = instantiate_sbom("{ not json", &sample_placeholders()).unwrap_err();
        assert!(err.to_string().contains("Invalid generated SBOM"));
    }
}
