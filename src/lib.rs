//! Library for generating and manipulating CycloneDX 1.7 SBOMs.
//!
//! Organization, designed to stay easy to extend:
//! - [`cyclonedx`]: CycloneDX data model (`serde` structs).
//! - [`commands`]: one CLI subcommand per module.
//! - [`generator_tool`]: description of this tool as a CycloneDX component,
//!   reusable by any command that produces an SBOM.
//! - [`util`]: domain-independent building blocks (hashing, HTTP, templating).

pub mod cli;
pub mod commands;
pub mod cyclonedx;
pub mod generator_tool;
pub mod util;
