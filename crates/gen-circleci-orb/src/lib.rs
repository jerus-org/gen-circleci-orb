//! # gen-circleci-orb
//!
//! Generate a CircleCI orb from a CLI program definition.

use anyhow::Result;
use clap::Parser;

/// Command-line interface for gen-circleci-orb.
#[derive(Debug, Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

/// Available subcommands.
#[derive(Debug, clap::Subcommand)]
pub enum Commands {}

impl Cli {
    /// Execute the selected command.
    pub fn run(&self) -> Result<()> {
        unreachable!()
    }
}
