//! Test fixture only: a small clap CLI whose --help output exercises a real
//! ambiguous-subcommand-name collision (gen-circleci-orb#358) through the
//! actual --help-parsing pipeline, not a hand-built CliDefinition literal —
//! a top-level `release` and a nested `ci release` share a bare name. Never
//! shipped — gated behind the `test-fixtures` feature, same as fixture-cli.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "fixture-cli-collision",
    about = "Generator test fixture CLI: ambiguous subcommand name (#358)"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Release the top-level thing
    Release(ReleaseArgs),
    /// CI-related commands
    Ci(CiArgs),
}

#[derive(clap::Args)]
struct ReleaseArgs {
    /// Version to release
    #[arg(long)]
    version: Option<String>,
}

#[derive(clap::Args)]
struct CiArgs {
    #[command(subcommand)]
    command: CiCommands,
}

#[derive(Subcommand)]
enum CiCommands {
    /// Release inside ci — deliberately shares a bare name with the
    /// top-level "release" above.
    Release(ReleaseArgs),
}

fn main() {
    let _cli = Cli::parse();
}
