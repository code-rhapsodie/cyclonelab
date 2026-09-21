use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::commands::{lint, suggest, transform, validate};
use crate::version;

#[derive(Debug, Parser)]
#[command(
    name = "cyclonelab",
    about = "Generator and manipulation tool for CycloneDX 1.7 SBOMs",
    version = version::FULL
)]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Checks that a file is valid JSON and conforms to the CycloneDX schema.
    Validate(validate::ValidateArgs),
    /// Applies a declarative transformation recipe to a CycloneDX SBOM.
    Transform(transform::TransformArgs),
    /// Checks that a transformation YAML file is well-formed, without requiring an SBOM.
    Lint(lint::LintArgs),
    /// Suggests useful component fields missing from a CycloneDX SBOM.
    Suggest(suggest::SuggestArgs),
}

impl Cli {
    pub fn run(self) -> Result<()> {
        match &self.command {
            Commands::Validate(args) => validate::run(args),
            Commands::Transform(args) => transform::run(args),
            Commands::Lint(args) => lint::run(args),
            Commands::Suggest(args) => suggest::run(args),
        }
    }
}
