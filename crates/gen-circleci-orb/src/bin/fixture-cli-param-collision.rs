//! Test fixture only: a small clap CLI whose --help output exercises a real
//! param-key collision (gen-circleci-orb#412) through the actual
//! --help-parsing pipeline, not a hand-built CliDefinition literal. Subcommand
//! `generate` has a restricted `--name` (auto-renamed by gen-circleci-orb to
//! `generate_name`) and an unrelated, genuinely-named `--generate-name` flag
//! whose own normalized name already equals that rename target -- two
//! perfectly valid, individually-unambiguous clap flags; the collision is
//! purely an artifact of gen-circleci-orb's own restricted-param rename, not
//! anything clap or this fixture's author would ever see. Never shipped —
//! gated behind the `test-fixtures` feature, same as fixture-cli.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "fixture-cli-param-collision",
    about = "Generator test fixture CLI: renamed param-key collision (#412)"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate the output
    Generate(GenerateArgs),
}

#[derive(clap::Args)]
struct GenerateArgs {
    /// Name for the output
    #[arg(long)]
    name: Option<String>,
    /// Whether to generate a name
    #[arg(long)]
    generate_name: bool,
}

fn main() {
    let _cli = Cli::parse();
}
