//! Test fixture only: like `fixture-cli-collision` (a root `release` and a
//! nested `ci release` share a bare name) but each also declares a short-only
//! `-n <COUNT>` whose description yields no usable name, so it can only be
//! named via `[subcommand.<name>.short_param]` (gen-circleci-orb#435). Never
//! shipped — gated behind the `test-fixtures` feature.

use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "fixture-cli-short-collision",
    about = "Generator test fixture CLI: colliding leaves with short-only options (#435)"
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

#[derive(Args)]
struct ReleaseArgs {
    /// How many times
    #[arg(short = 'n')]
    count: Option<String>,
}

#[derive(Args)]
struct CiArgs {
    #[command(subcommand)]
    command: CiCommands,
}

#[derive(Subcommand)]
enum CiCommands {
    /// Release inside ci
    Release(ReleaseArgs),
}

fn main() {
    let _cli = Cli::parse();
}
