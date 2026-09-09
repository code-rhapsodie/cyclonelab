use clap::Parser;
use cyclonelab::cli::Cli;

fn main() -> anyhow::Result<()> {
    Cli::parse().run()
}
