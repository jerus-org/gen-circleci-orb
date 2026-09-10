//! Test fixture only: a small clap CLI whose `--help` output exercises the
//! generator's known tricky parsing/rendering branches (log_level merge,
//! positional-after-flags, a restricted `name` param, an enum param,
//! config-driven interactivity, a lone repeatable flag). Never shipped —
//! gated behind the `test-fixtures` feature so it isn't built by a normal
//! `cargo build`/`cargo install`. Its own runtime behavior is irrelevant;
//! it is only ever introspected via `--help`.

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "fixture-cli", about = "Generator test fixture CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build a target, with a name, format and verbosity controls
    Build(BuildArgs),
    /// Push one or more tags
    Push(PushArgs),
    /// Configure the tool (interactivity is asserted via gen-circleci-orb.toml, not this --help)
    Configure(ConfigureArgs),
}

#[derive(clap::Args)]
struct BuildArgs {
    /// Human-readable name for this build
    #[arg(long)]
    name: String,

    /// Output format
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,

    /// Increase logging verbosity (repeatable)
    #[arg(short = 'v', long, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Decrease logging verbosity (repeatable)
    #[arg(short = 'q', long, action = clap::ArgAction::Count)]
    quiet: u8,

    /// Build target
    target: String,
}

#[derive(ValueEnum, Clone, Default)]
enum Format {
    #[default]
    Text,
    Json,
}

#[derive(clap::Args)]
struct PushArgs {
    /// Extra tag to attach (repeatable)
    #[arg(long)]
    tag: Vec<String>,
}

#[derive(clap::Args)]
struct ConfigureArgs {
    /// Config file path
    #[arg(long)]
    config_path: Option<String>,
}

fn main() {
    let _cli = Cli::parse();
}
