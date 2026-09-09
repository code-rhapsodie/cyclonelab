//! Description of this generator as a CycloneDX component
//! (`metadata.tools.components[]`), to be reused by any command that
//! produces or modifies an SBOM.

use crate::cyclonedx::{Bom, Component, LicenseChoice, Tools};

pub fn component() -> Component {
    Component::new("application", "cyclonelab")
        .with_group("coderhapsodie")
        .with_publisher("Code Rhapsodie")
        .with_version(env!("CARGO_PKG_VERSION"))
        .with_purl(format!(
            "pkg:generic/coderhapsodie/cyclonelab@{}",
            env!("CARGO_PKG_VERSION")
        ))
        .with_license(LicenseChoice::named(
            "European Union Public License 1.2",
            "https://spdx.org/licenses/EUPL-1.2.html",
            "declared",
        ))
}

/// Declares this generator as the sole tool that produced the BOM,
/// replacing `metadata.tools` if it already existed.
pub fn set_as_sole_tool(bom: &mut Bom) {
    bom.metadata_mut().tools = Some(Tools::set_single_component(component()));
}
