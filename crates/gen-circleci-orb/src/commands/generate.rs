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

    /// CircleCI orb namespace(s) to publish the orb under (repeatable).
    #[arg(long = "orb-namespace", required = true)]
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

    /// Subcommand name(s) whose generated jobs should include a set_https_remote step
    /// (repeatable). Use for subcommands that push to git, e.g. --git-push-subcommand save.
    #[arg(long = "git-push-subcommand")]
    pub git_push_subcommands: Vec<String>,

    /// circleci-cli version to install in the generated Docker image executor.
    /// When set, adds a cli-installer builder stage that downloads the release
    /// tarball, verifies its SHA-256 checksum, and copies the binary into the
    /// final image.  Required when the wrapped binary calls `circleci` commands
    /// at runtime (e.g. when generating gen-circleci-orb's own orb).
    #[arg(long)]
    pub circleci_cli_version: Option<String>,

    /// Extra apt package(s) to install in the final Docker image stage (repeatable).
    /// Combined with the baseline packages (ca-certificates, git) and sorted
    /// alphanumerically. Example: --apt-packages libssl-dev --apt-packages pkg-config
    #[arg(long = "apt-packages")]
    pub apt_packages: Vec<String>,

    /// Show planned output without writing any files.
    #[arg(long)]
    pub dry_run: bool,
}

/// Convert any git remote URL to a plain HTTPS URL, stripping the `.git` suffix.
///
/// Handles:
/// - `git@github.com:org/repo.git` → `https://github.com/org/repo`
/// - `https://github.com/org/repo.git` → `https://github.com/org/repo`
/// - `https://github.com/org/repo` → unchanged
pub(crate) fn normalize_git_remote_url(url: &str) -> String {
    let url = if let Some(rest) = url.strip_prefix("git@") {
        // git@host:org/repo.git → https://host/org/repo
        let normalized = rest.replacen(':', "/", 1);
        format!("https://{normalized}")
    } else {
        url.to_string()
    };
    url.strip_suffix(".git").unwrap_or(&url).to_string()
}

/// Attempt to detect the repository source URL from the git remote named `origin`.
/// Returns `None` if git is unavailable or no `origin` remote is configured.
pub(crate) fn detect_source_url() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8(output.stdout).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(normalize_git_remote_url(trimmed))
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

        let detected_url = self.source_url.is_none().then(detect_source_url).flatten();
        let source_url = self.source_url.clone().or_else(|| detected_url.clone());
        let home_url = self.home_url.clone().or_else(|| detected_url.clone());

        let opts = orb_generator::GenerateOpts {
            namespaces: self.namespaces.clone(),
            install_method: self.install_method.clone(),
            base_image: self.base_image.clone(),
            home_url,
            source_url,
            binary_name: cli_def.binary_name.clone(),
            git_push_subcommands: self.git_push_subcommands.clone(),
            circleci_cli_version: self.circleci_cli_version.clone(),
            apt_packages: self.apt_packages.clone(),
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

    // ── normalize_git_remote_url ────────────────────────────────────────────

    #[test]
    fn normalize_ssh_remote_to_https() {
        assert_eq!(
            normalize_git_remote_url("git@github.com:jerus-org/gen-circleci-orb.git"),
            "https://github.com/jerus-org/gen-circleci-orb"
        );
    }

    #[test]
    fn normalize_https_with_git_suffix() {
        assert_eq!(
            normalize_git_remote_url("https://github.com/jerus-org/gen-circleci-orb.git"),
            "https://github.com/jerus-org/gen-circleci-orb"
        );
    }

    #[test]
    fn normalize_https_without_git_suffix_unchanged() {
        assert_eq!(
            normalize_git_remote_url("https://github.com/jerus-org/gen-circleci-orb"),
            "https://github.com/jerus-org/gen-circleci-orb"
        );
    }

    #[test]
    fn normalize_ssh_non_github_host() {
        assert_eq!(
            normalize_git_remote_url("git@gitlab.com:myorg/myrepo.git"),
            "https://gitlab.com/myorg/myrepo"
        );
    }

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
