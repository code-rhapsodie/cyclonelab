//! Description of this generator as a CycloneDX component
//! (`metadata.tools.components[]`), to be reused by any command that
//! produces or modifies an SBOM.

use anyhow::Result;
use serde_json::{Map, Value, json};

use crate::cyclonedx::{Component, LicenseChoice};
use crate::version;

pub fn component() -> Component {
    Component::new("application", "cyclonelab")
        .with_group("coderhapsodie")
        .with_publisher("Code Rhapsodie")
        .with_version(version::VERSION)
        .with_purl(format!(
            "pkg:generic/coderhapsodie/cyclonelab@{}",
            version::VERSION
        ))
        .with_license(LicenseChoice::named(
            "European Union Public License 1.2",
            "https://spdx.org/licenses/EUPL-1.2.html",
            "declared",
        ))
}

/// Records this generator's own entry in `metadata.tools`, alongside
/// whatever tools are already listed there, preserving the provenance of
/// the tools that already produced the SBOM. Running it again (e.g. a
/// second `transform` pass) refreshes this generator's own entry in place
/// instead of appending a duplicate.
///
/// Operates directly on the `serde_json::Value` document (see
/// `doc/transform/README.md` §7.2) rather than the typed [`Bom`], since the
/// transform engine never deserializes into it. `metadata.tools` has two
/// valid shapes depending on the CycloneDX schema version and on what
/// produced the SBOM (both remain valid at 1.5+, and real-world SBOMs, this
/// project's own fixtures included, still commonly use the legacy one):
/// - the current `{components: [...], services: [...]}` object form, where
///   this generator adds itself as a full `Component` (matched for
///   updates by `group`+`name`);
/// - the deprecated bare-array form (`[{vendor, name, version}, ...]`),
///   whose schema only allows `vendor`/`name`/`version`/`hashes`/
///   `externalReferences` — this generator adds itself in that reduced
///   shape instead (matched for updates by `vendor`+`name`), rather than
///   forcing a migration to the object form as a side effect of an
///   unrelated transformation.
///
/// `metadata.tools` is created in the (non-deprecated) object form when
/// absent.
pub fn register_as_tool(document: &mut Value) -> Result<()> {
    let Some(root) = document.as_object_mut() else {
        anyhow::bail!("the SBOM root is not a JSON object");
    };
    let metadata = root
        .entry("metadata")
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(metadata) = metadata.as_object_mut() else {
        anyhow::bail!("'metadata' is not a JSON object");
    };
    let tools = metadata
        .entry("tools")
        .or_insert_with(|| Value::Object(Map::new()));

    match tools {
        Value::Array(tools) => register_in_legacy_array(tools),
        Value::Object(tools) => register_in_components(tools)?,
        _ => anyhow::bail!("'metadata.tools' is neither a JSON object nor an array"),
    }
    Ok(())
}

fn register_in_components(tools: &mut Map<String, Value>) -> Result<()> {
    let components = tools
        .entry("components")
        .or_insert_with(|| Value::Array(Vec::new()));
    let Some(components) = components.as_array_mut() else {
        anyhow::bail!("'metadata.tools.components' is not a JSON array");
    };

    let entry = serde_json::to_value(component())?;
    let is_this_generator = |c: &&mut Value| {
        c.get("group").and_then(Value::as_str) == Some("coderhapsodie")
            && c.get("name").and_then(Value::as_str) == Some("cyclonelab")
    };
    match components.iter_mut().find(is_this_generator) {
        Some(existing) => *existing = entry,
        None => components.push(entry),
    }
    Ok(())
}

fn register_in_legacy_array(tools: &mut Vec<Value>) {
    let entry = json!({
        "vendor": "Code Rhapsodie",
        "name": "cyclonelab",
        "version": version::VERSION,
    });
    let is_this_generator = |t: &&mut Value| {
        t.get("vendor").and_then(Value::as_str) == Some("Code Rhapsodie")
            && t.get("name").and_then(Value::as_str) == Some("cyclonelab")
    };
    match tools.iter_mut().find(is_this_generator) {
        Some(existing) => *existing = entry,
        None => tools.push(entry),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn appends_alongside_an_existing_tool() {
        let mut doc = json!({
            "metadata": {"tools": {"components": [{"type": "application", "name": "cargo-cyclonedx"}]}}
        });
        register_as_tool(&mut doc).unwrap();

        let components = doc["metadata"]["tools"]["components"].as_array().unwrap();
        assert_eq!(components.len(), 2);
        assert_eq!(components[0]["name"], "cargo-cyclonedx");
        assert_eq!(components[1]["name"], "cyclonelab");
        assert_eq!(components[1]["group"], "coderhapsodie");
        assert_eq!(components[1]["version"], version::VERSION);
    }

    #[test]
    fn creates_missing_metadata_tools_components() {
        let mut doc = json!({});
        register_as_tool(&mut doc).unwrap();

        let components = doc["metadata"]["tools"]["components"].as_array().unwrap();
        assert_eq!(components.len(), 1);
        assert_eq!(components[0]["name"], "cyclonelab");
    }

    #[test]
    fn running_it_twice_refreshes_its_own_entry_instead_of_duplicating_it() {
        let mut doc = json!({
            "metadata": {"tools": {"components": [
                {"type": "application", "group": "coderhapsodie", "name": "cyclonelab", "version": "0.0.0-stale"}
            ]}}
        });
        register_as_tool(&mut doc).unwrap();

        let components = doc["metadata"]["tools"]["components"].as_array().unwrap();
        assert_eq!(components.len(), 1);
        assert_eq!(components[0]["version"], version::VERSION);
    }

    #[test]
    fn appends_to_a_legacy_array_form_in_the_reduced_legacy_shape() {
        let mut doc = json!({
            "metadata": {"tools": [{"vendor": "CycloneDX", "name": "cargo-cyclonedx", "version": "0.5.9"}]}
        });
        register_as_tool(&mut doc).unwrap();

        let tools = doc["metadata"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["name"], "cargo-cyclonedx");
        assert_eq!(tools[1]["vendor"], "Code Rhapsodie");
        assert_eq!(tools[1]["name"], "cyclonelab");
        assert_eq!(tools[1]["version"], version::VERSION);
        assert!(
            tools[1].get("group").is_none(),
            "the legacy Tool schema has no 'group' field"
        );
    }

    #[test]
    fn running_it_twice_on_a_legacy_array_refreshes_its_own_entry_instead_of_duplicating_it() {
        let mut doc = json!({
            "metadata": {"tools": [
                {"vendor": "Code Rhapsodie", "name": "cyclonelab", "version": "0.0.0-stale"}
            ]}
        });
        register_as_tool(&mut doc).unwrap();

        let tools = doc["metadata"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["version"], version::VERSION);
    }
}
