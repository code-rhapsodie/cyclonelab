//! Generator and manipulation tool for CycloneDX 1.7 SBOMs.
//!
//! Organization, designed to stay easy to extend:
//! - [`cyclonedx`]: CycloneDX data model (`serde` structs).
//! - [`commands`]: one CLI subcommand per module.
//! - [`generator_tool`]: description of this tool as a CycloneDX component,
//!   reusable by any command that produces an SBOM.
//! - [`util`]: domain-independent building blocks (hashing, HTTP, templating).

mod cli;
mod commands;
mod cyclonedx;
mod generator_tool;
mod transform_actions;
mod util;
mod version;

use clap::Parser;
use cli::Cli;

fn main() -> anyhow::Result<()> {
    Cli::parse().run()
}
