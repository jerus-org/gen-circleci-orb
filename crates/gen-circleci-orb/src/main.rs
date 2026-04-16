use anyhow::Result;
use clap::Parser;
use gen_circleci_orb::Cli;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[allow(unreachable_code, unused_variables)]
fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "gen_circleci_orb=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();
    cli.run()
}
