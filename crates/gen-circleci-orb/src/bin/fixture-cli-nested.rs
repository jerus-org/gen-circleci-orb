//! Test fixture only: a small clap CLI with three levels of subcommand
//! nesting (`a b c`), to exercise the generator's `--help`-parsing and
//! invocation-script rendering at a depth beyond the flat `fixture-cli` and
//! the two-level `fixture-cli-collision`. Never shipped — gated behind the
//! `test-fixtures` feature, same as the other fixtures.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "fixture-cli-nested",
    about = "Generator test fixture CLI: 3-level nesting"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Level-one group
    A(AArgs),
}

#[derive(clap::Args)]
struct AArgs {
    #[command(subcommand)]
    command: ACommands,
}

#[derive(Subcommand)]
enum ACommands {
    /// Level-two group
    B(BArgs),
}

#[derive(clap::Args)]
struct BArgs {
    #[command(subcommand)]
    command: BCommands,
}

#[derive(Subcommand)]
enum BCommands {
    /// Level-three leaf, with a real parameter to prove it was parsed
    /// correctly rather than falling back to a clap error string.
    C(CArgs),
}

#[derive(clap::Args)]
struct CArgs {
    /// A value to pass through
    #[arg(long)]
    value: Option<String>,
}

fn main() {
    let _cli = Cli::parse();
}
