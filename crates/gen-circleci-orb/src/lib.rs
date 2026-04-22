//! # gen-circleci-orb
//!
//! Generate a CircleCI orb from a CLI program definition.

use anyhow::Result;
use clap::Parser;

pub mod ci_patcher;
pub mod commands;
pub mod help_parser;
pub mod orb_generator;
pub mod output_writer;

/// Command-line interface for gen-circleci-orb.
#[derive(Debug, Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

/// Available subcommands.
#[derive(Debug, clap::Subcommand)]
pub enum Commands {
    /// Generate orb source files from a CLI binary's --help output.
    Generate(commands::generate::Generate),
    /// Wire orb generation into an existing repo's CI configuration.
    Init(commands::init::Init),
}

impl Cli {
    /// Execute the selected command.
    pub fn run(&self) -> Result<()> {
        match &self.command {
            Commands::Generate(cmd) => cmd.run(),
            Commands::Init(cmd) => cmd.run(),
        }
    }
}
