use anyhow::Result;
use clap::ValueEnum;
use std::path::PathBuf;

use crate::{help_parser, orb_generator, output_writer};

#[derive(Debug, Clone, ValueEnum)]
pub enum InstallMethod {
    Binstall,
    Apt,
}

/// Generate orb source files from a CLI binary's --help output.
#[derive(Debug, clap::Args)]
pub struct Generate {
    /// Name of the binary to introspect (must be on PATH).
    #[arg(long)]
    pub binary: String,

    /// CircleCI namespace(s) to publish the orb under (repeatable).
    #[arg(long = "namespace", required = true)]
    pub namespaces: Vec<String>,

    /// Output directory for generated orb source.
    #[arg(long, default_value = "./out")]
    pub output: PathBuf,

    /// How the binary is installed in the generated Docker image.
    #[arg(long, value_enum, default_value = "binstall")]
    pub install_method: InstallMethod,

    /// Base Docker image for the generated executor.
    #[arg(long, default_value = "ubuntu:24.04")]
    pub base_image: String,

    /// Home URL for the orb registry display section.
    #[arg(long)]
    pub home_url: Option<String>,

    /// Source URL for the orb registry display section.
    #[arg(long)]
    pub source_url: Option<String>,

    /// Show planned output without writing any files.
    #[arg(long)]
    pub dry_run: bool,
}

impl Generate {
    pub fn run(&self) -> Result<()> {
        tracing::info!("Parsing {} --help", self.binary);
        let cli_def = help_parser::parse_binary(&self.binary)?;

        tracing::info!("Discovered {} subcommand(s)", cli_def.subcommands.len());

        let opts = orb_generator::GenerateOpts {
            namespaces: self.namespaces.clone(),
            install_method: self.install_method.clone(),
            base_image: self.base_image.clone(),
            home_url: self.home_url.clone(),
            source_url: self.source_url.clone(),
            binary_name: cli_def.binary_name.clone(),
        };

        let files = orb_generator::generate(&cli_def, &opts);

        tracing::info!("Generated {} file(s)", files.len());

        let report = output_writer::write_tree(&self.output, &files, self.dry_run)?;

        println!(
            "Done: {} created, {} updated, {} unchanged",
            report.created, report.updated, report.unchanged
        );
        Ok(())
    }
}
