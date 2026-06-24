//! # gen-circleci-orb
//!
//! Generate a CircleCI orb from a CLI program definition.

use anyhow::Result;
use clap::Parser;

pub mod ci_patcher;
pub mod commands;
pub mod help_parser;
pub mod orb_config;
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
    /// Manage the gen-circleci-orb.toml configuration file.
    Config(commands::config::Config),
    /// Ensure a CircleCI orb is registered, creating it if it does not exist.
    EnsureOrbRegistered(commands::ensure_orb_registered::EnsureOrbRegistered),
    /// Generate orb source files from a CLI binary's --help output.
    Generate(Box<commands::generate::Generate>),
    /// Wire orb generation into an existing repo's CI configuration.
    Init(Box<commands::init::Init>),
    /// Re-sync an existing repo's orb-managed CI wiring to the current flow.
    Update(commands::update::Update),
}

impl Cli {
    /// Execute the selected command.
    pub fn run(&self) -> Result<()> {
        match &self.command {
            Commands::Config(cmd) => cmd.run(),
            Commands::EnsureOrbRegistered(cmd) => cmd.run(),
            Commands::Generate(cmd) => cmd.run(),
            Commands::Init(cmd) => cmd.run(),
            Commands::Update(cmd) => cmd.run(),
        }
    }
}
