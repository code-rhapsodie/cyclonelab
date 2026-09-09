use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::commands::{generate_extension_sbom, validate};

#[derive(Debug, Parser)]
#[command(
    name = "cyclonelab",
    about = "Generator and manipulation tool for CycloneDX 1.7 SBOMs",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Instantiates an SBOM template for each compiled PHP extension in an artifact folder.
    GenerateExtensionSbom(generate_extension_sbom::GenerateExtensionSbomArgs),
    /// Checks that a file is valid JSON and conforms to the CycloneDX schema.
    Validate(validate::ValidateArgs),
}

impl Cli {
    pub fn run(self) -> Result<()> {
        match &self.command {
            Commands::GenerateExtensionSbom(args) => generate_extension_sbom::run(args),
            Commands::Validate(args) => validate::run(args),
        }
    }
}
