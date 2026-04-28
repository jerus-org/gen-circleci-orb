use anyhow::Result;
use clap::ValueEnum;
use std::path::{Path, PathBuf};

use crate::{help_parser, orb_generator, output_writer};

pub const DEFAULT_BASE_IMAGE: &str = "debian:12-slim";

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

    /// Project root directory (orb source is written to <output>/<orb-dir>/).
    #[arg(long, default_value = ".")]
    pub output: PathBuf,

    /// How the binary is installed in the generated Docker image.
    #[arg(long, value_enum, default_value = "binstall")]
    pub install_method: InstallMethod,

    /// Base Docker image for the generated executor.
    #[arg(long, default_value = DEFAULT_BASE_IMAGE)]
    pub base_image: String,

    /// Home URL for the orb registry display section.
    #[arg(long)]
    pub home_url: Option<String>,

    /// Source URL for the orb registry display section.
    #[arg(long)]
    pub source_url: Option<String>,

    /// Subdirectory within --output where orb source is written (default: orb).
    #[arg(long, default_value = "orb")]
    pub orb_dir: String,

    /// Show planned output without writing any files.
    #[arg(long)]
    pub dry_run: bool,
}

/// Guard: refuse to write into a directory that exists but looks like unrelated source code.
///
/// A directory is considered safe if it is absent, empty, or already contains `src/@orb.yml`.
pub(crate) fn check_orb_dir(orb_root: &Path) -> Result<()> {
    if !orb_root.exists() {
        return Ok(());
    }
    if orb_root.join("src/@orb.yml").exists() {
        return Ok(());
    }
    let has_content = std::fs::read_dir(orb_root)?.next().is_some();
    if has_content {
        anyhow::bail!(
            "Directory '{}' already exists but does not appear to contain a CircleCI orb \
             (no src/@orb.yml found). Refusing to write into it to avoid mixing orb source \
             with unrelated code. Use --orb-dir to specify a different subdirectory.",
            orb_root.display()
        );
    }
    Ok(())
}

impl Generate {
    pub fn run(&self) -> Result<()> {
        let orb_root = self.output.join(&self.orb_dir);

        check_orb_dir(&orb_root)?;

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

        let report = output_writer::write_tree(&orb_root, &files, self.dry_run)?;

        println!(
            "Done: {} created, {} updated, {} unchanged",
            report.created, report.updated, report.unchanged
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn check_orb_dir_absent_is_ok() {
        let tmp = TempDir::new().unwrap();
        let orb_root = tmp.path().join("orb");
        assert!(check_orb_dir(&orb_root).is_ok());
    }

    #[test]
    fn check_orb_dir_with_orb_yml_is_ok() {
        let tmp = TempDir::new().unwrap();
        let orb_root = tmp.path().join("orb");
        fs::create_dir_all(orb_root.join("src")).unwrap();
        fs::write(orb_root.join("src/@orb.yml"), "version: 2.1").unwrap();
        assert!(check_orb_dir(&orb_root).is_ok());
    }

    #[test]
    fn check_orb_dir_empty_is_ok() {
        let tmp = TempDir::new().unwrap();
        let orb_root = tmp.path().join("orb");
        fs::create_dir_all(&orb_root).unwrap();
        assert!(check_orb_dir(&orb_root).is_ok());
    }

    #[test]
    fn check_orb_dir_with_unrelated_content_errors() {
        let tmp = TempDir::new().unwrap();
        let orb_root = tmp.path().join("src");
        fs::create_dir_all(&orb_root).unwrap();
        fs::write(orb_root.join("main.rs"), "fn main() {}").unwrap();
        let result = check_orb_dir(&orb_root);
        assert!(
            result.is_err(),
            "should error when unrelated content present"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("does not appear to contain a CircleCI orb"),
            "unexpected error message: {msg}"
        );
    }
}
