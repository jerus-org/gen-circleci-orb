//! Test fixture only: a small clap CLI that distinguishes a genuinely shared
//! option (a root-level `global = true` flag, available to every subcommand)
//! from two subcommands that each independently declare an option of the same
//! name (gen-circleci-orb#423). Never shipped — gated behind the
//! `test-fixtures` feature, same as fixture-cli.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "fixture-cli-shared-params",
    about = "Generator test fixture CLI: shared vs same-named params (#423)"
)]
struct Cli {
    /// Config file, set once and inherited by every subcommand
    #[arg(long, global = true)]
    config: Option<String>,

    /// The root's own output, NOT global: no subcommand inherits it, even
    /// though each declares an unrelated `--output` of its own
    #[arg(long)]
    output: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate a report
    Generate {
        /// Where generate writes its report
        #[arg(long)]
        output: Option<String>,
    },
    /// Publish a release
    Release {
        /// Where release writes its archive
        #[arg(long)]
        output: Option<String>,
    },
}

fn main() {
    let _cli = Cli::parse();
}
