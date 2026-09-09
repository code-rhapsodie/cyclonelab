//! CLI subcommands.
//!
//! To add a new command: create a `my_module.rs` module exposing an `Args`
//! struct (deriving `clap::Args`) and a `run(args: &Args) -> anyhow::Result<()>`
//! function, then register it in `crate::cli::Commands` and in
//! `crate::cli::Cli::run`.

pub mod generate_extension_sbom;
