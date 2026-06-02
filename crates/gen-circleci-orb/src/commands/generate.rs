use anyhow::Result;
use clap::ValueEnum;
use std::path::{Path, PathBuf};

use crate::{help_parser, orb_config, orb_generator, output_writer};

pub const DEFAULT_BASE_IMAGE: &str = "debian:12-slim";

#[derive(Debug, Clone, ValueEnum)]
pub enum InstallMethod {
    Binstall,
    Apt,
    /// Binary is pre-built and present in the Docker build context as `./{binary}`.
    /// Generates a single-stage Dockerfile with `COPY {binary}` — no crates.io download.
    /// Use when the release pipeline builds the binary before `docker build`.
    Local,
}

/// Generate orb source files from a CLI binary's --help output.
#[derive(Debug, clap::Args)]
pub struct Generate {
    /// Name of the binary to introspect (must be on PATH).
    /// Falls back to `binary` in the [orb] section of gen-circleci-orb.toml.
    #[arg(long)]
    pub binary: Option<String>,

    /// CircleCI orb namespace(s) to publish the orb under (repeatable).
    /// Falls back to `namespaces` in the [orb] section of gen-circleci-orb.toml.
    #[arg(long = "orb-namespace")]
    pub namespaces: Vec<String>,

    /// Project root directory (orb source is written to <output>/<orb-dir>/).
    #[arg(long, default_value = ".")]
    pub output: PathBuf,

    /// How the binary is installed in the generated Docker image.
    /// Falls back to `install_method` in the [orb] section of gen-circleci-orb.toml, then "binstall".
    #[arg(long, value_enum)]
    pub install_method: Option<InstallMethod>,

    /// Base Docker image for the generated executor.
    /// Falls back to `base_image` in the [orb] section of gen-circleci-orb.toml, then "debian:12-slim".
    #[arg(long)]
    pub base_image: Option<String>,

    /// Home URL for the orb registry display section.
    /// Falls back to `home_url` in the [orb] section of gen-circleci-orb.toml.
    #[arg(long)]
    pub home_url: Option<String>,

    /// Source URL for the orb registry display section.
    /// Falls back to `source_url` in the [orb] section of gen-circleci-orb.toml.
    #[arg(long)]
    pub source_url: Option<String>,

    /// Subdirectory within --output where orb source is written.
    /// Falls back to `orb_dir` in the [orb] section of gen-circleci-orb.toml, then "orb".
    #[arg(long)]
    pub orb_dir: Option<String>,

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

    /// Path to gen-circleci-orb.toml config file.
    /// Defaults to <output>/gen-circleci-orb.toml when not specified.
    #[arg(long)]
    pub config: Option<PathBuf>,
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

/// Resolve the binary name: CLI flag takes precedence, then `[orb].binary` in config.
/// Returns an error with a helpful message when neither is provided.
pub(crate) fn resolve_binary(
    cli: Option<&str>,
    config: &crate::orb_config::OrbConfig,
) -> Result<String> {
    if let Some(b) = cli.filter(|s| !s.is_empty()) {
        return Ok(b.to_string());
    }
    config
        .orb
        .as_ref()
        .and_then(|o| o.binary.clone())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "binary name is required — pass --binary <NAME> or add `binary = \"<name>\"` \
                 to the [orb] section of gen-circleci-orb.toml"
            )
        })
}

/// Resolve namespaces: CLI flags take precedence, then `[orb].namespaces` in config.
/// Returns an error with a helpful message when neither is provided.
pub(crate) fn resolve_namespaces(
    cli: &[String],
    config: &crate::orb_config::OrbConfig,
) -> Result<Vec<String>> {
    if !cli.is_empty() {
        return Ok(cli.to_vec());
    }
    config
        .orb
        .as_ref()
        .and_then(|o| o.namespaces.clone())
        .filter(|ns| !ns.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "at least one namespace is required — pass --orb-namespace <NS> or add \
                 `namespaces = [\"<ns>\"]` to the [orb] section of gen-circleci-orb.toml"
            )
        })
}

/// Resolve orb_dir: CLI value takes precedence, then `[orb].orb_dir` in config, then "orb".
pub(crate) fn resolve_orb_dir(cli: Option<&str>, config: &crate::orb_config::OrbConfig) -> String {
    cli.filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| config.orb.as_ref().and_then(|o| o.orb_dir.clone()))
        .unwrap_or_else(|| "orb".to_string())
}

/// CLI flag takes precedence; falls back to `[orb] git_push_subcommands` in the config.
pub(crate) fn resolve_git_push_subcommands(
    cli: &[String],
    config: &crate::orb_config::OrbConfig,
) -> Vec<String> {
    if !cli.is_empty() {
        return cli.to_vec();
    }
    config
        .orb
        .as_ref()
        .and_then(|o| o.git_push_subcommands.clone())
        .unwrap_or_default()
}

/// CLI flag takes precedence; falls back to `[orb].install_method` in config, then Binstall.
pub(crate) fn resolve_install_method(
    cli: Option<&InstallMethod>,
    config: &crate::orb_config::OrbConfig,
) -> InstallMethod {
    if let Some(m) = cli {
        return m.clone();
    }
    config
        .orb
        .as_ref()
        .and_then(|o| o.install_method.as_deref())
        .and_then(|s| match s {
            "apt" => Some(InstallMethod::Apt),
            "binstall" => Some(InstallMethod::Binstall),
            "local" => Some(InstallMethod::Local),
            _ => None,
        })
        .unwrap_or(InstallMethod::Binstall)
}

/// CLI flag takes precedence; falls back to `[orb].base_image` in config, then DEFAULT_BASE_IMAGE.
pub(crate) fn resolve_base_image(
    cli: Option<&str>,
    config: &crate::orb_config::OrbConfig,
) -> String {
    cli.filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| config.orb.as_ref().and_then(|o| o.base_image.clone()))
        .unwrap_or_else(|| DEFAULT_BASE_IMAGE.to_string())
}

pub(crate) fn resolve_config_path(explicit: Option<&PathBuf>, output: &Path) -> PathBuf {
    explicit
        .cloned()
        .unwrap_or_else(|| output.join("gen-circleci-orb.toml"))
}

impl Generate {
    pub fn run(&self) -> Result<()> {
        let config_path = resolve_config_path(self.config.as_ref(), &self.output);
        let orb_config = orb_config::load_config(&config_path)?;

        // Resolve fields that may come from config when not provided on CLI.
        let binary = resolve_binary(self.binary.as_deref(), &orb_config)?;
        let namespaces = resolve_namespaces(&self.namespaces, &orb_config)?;
        let orb_dir = resolve_orb_dir(self.orb_dir.as_deref(), &orb_config);

        let orb_root = self.output.join(&orb_dir);
        check_orb_dir(&orb_root)?;

        tracing::info!("Parsing {} --help", binary);
        let cli_def = help_parser::parse_binary(&binary)?;

        tracing::info!("Discovered {} subcommand(s)", cli_def.subcommands.len());

        let config_url = orb_config.orb.as_ref();
        let detected_url = self.source_url.is_none().then(detect_source_url).flatten();
        let source_url = self
            .source_url
            .clone()
            .or_else(|| config_url.and_then(|o| o.source_url.clone()))
            .or_else(|| detected_url.clone());
        let home_url = self
            .home_url
            .clone()
            .or_else(|| config_url.and_then(|o| o.home_url.clone()))
            .or_else(|| detected_url.clone());

        let opts = orb_generator::GenerateOpts {
            namespaces,
            install_method: resolve_install_method(self.install_method.as_ref(), &orb_config),
            base_image: resolve_base_image(self.base_image.as_deref(), &orb_config),
            home_url,
            source_url,
            binary_name: cli_def.binary_name.clone(),
            git_push_subcommands: resolve_git_push_subcommands(
                &self.git_push_subcommands,
                &orb_config,
            ),
            circleci_cli_version: self.circleci_cli_version.clone(),
            apt_packages: self.apt_packages.clone(),
        };

        let files = orb_generator::generate(&cli_def, &opts, Some(&orb_config));

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

    // ── Phase 5: config path resolution ────────────────────────────────────

    #[test]
    fn config_path_defaults_to_output_dir() {
        let tmp = TempDir::new().unwrap();
        let output = tmp.path();
        let resolved = resolve_config_path(None, output);
        assert_eq!(resolved, output.join("gen-circleci-orb.toml"));
    }

    #[test]
    fn config_path_uses_explicit_when_provided() {
        let tmp = TempDir::new().unwrap();
        let output = tmp.path();
        let explicit = PathBuf::from("/custom/path/config.toml");
        let resolved = resolve_config_path(Some(&explicit), output);
        assert_eq!(resolved, explicit);
    }

    // ── resolve_orb_opts: config fallbacks for required fields ─────────────

    #[test]
    fn resolve_binary_uses_cli_when_provided() {
        use crate::orb_config::OrbConfig;
        let config = OrbConfig::default();
        let binary = resolve_binary(Some("mytool"), &config).unwrap();
        assert_eq!(binary, "mytool");
    }

    #[test]
    fn resolve_binary_falls_back_to_config() {
        use crate::orb_config::{OrbConfig, OrbSection};
        let config = OrbConfig {
            orb: Some(OrbSection {
                binary: Some("config-tool".to_string()),
                ..OrbSection::default()
            }),
            ..OrbConfig::default()
        };
        let binary = resolve_binary(None, &config).unwrap();
        assert_eq!(binary, "config-tool");
    }

    #[test]
    fn resolve_binary_errors_when_neither_provided() {
        use crate::orb_config::OrbConfig;
        let result = resolve_binary(None, &OrbConfig::default());
        assert!(
            result.is_err(),
            "must error when no binary in CLI or config"
        );
        assert!(result.unwrap_err().to_string().contains("binary"));
    }

    #[test]
    fn resolve_namespaces_uses_cli_when_provided() {
        use crate::orb_config::OrbConfig;
        let ns = resolve_namespaces(&["my-org".to_string()], &OrbConfig::default()).unwrap();
        assert_eq!(ns, vec!["my-org".to_string()]);
    }

    #[test]
    fn resolve_namespaces_falls_back_to_config() {
        use crate::orb_config::{OrbConfig, OrbSection};
        let config = OrbConfig {
            orb: Some(OrbSection {
                namespaces: Some(vec!["cfg-org".to_string()]),
                ..OrbSection::default()
            }),
            ..OrbConfig::default()
        };
        let ns = resolve_namespaces(&[], &config).unwrap();
        assert_eq!(ns, vec!["cfg-org".to_string()]);
    }

    #[test]
    fn resolve_namespaces_errors_when_neither_provided() {
        use crate::orb_config::OrbConfig;
        let result = resolve_namespaces(&[], &OrbConfig::default());
        assert!(
            result.is_err(),
            "must error when no namespaces in CLI or config"
        );
        assert!(result.unwrap_err().to_string().contains("namespace"));
    }

    #[test]
    fn resolve_orb_dir_uses_cli_when_provided() {
        use crate::orb_config::OrbConfig;
        let dir = resolve_orb_dir(Some("custom-orb"), &OrbConfig::default());
        assert_eq!(dir, "custom-orb");
    }

    #[test]
    fn resolve_orb_dir_falls_back_to_config() {
        use crate::orb_config::{OrbConfig, OrbSection};
        let config = OrbConfig {
            orb: Some(OrbSection {
                orb_dir: Some("src/orb".to_string()),
                ..OrbSection::default()
            }),
            ..OrbConfig::default()
        };
        let dir = resolve_orb_dir(None, &config);
        assert_eq!(dir, "src/orb");
    }

    #[test]
    fn resolve_orb_dir_defaults_to_orb_when_nothing_provided() {
        use crate::orb_config::OrbConfig;
        let dir = resolve_orb_dir(None, &OrbConfig::default());
        assert_eq!(dir, "orb");
    }

    // ── resolve_install_method ─────────────────────────────────────────────

    #[test]
    fn resolve_install_method_uses_cli_when_provided() {
        use crate::orb_config::OrbConfig;
        let result = resolve_install_method(Some(&InstallMethod::Apt), &OrbConfig::default());
        assert!(matches!(result, InstallMethod::Apt));
    }

    #[test]
    fn resolve_install_method_falls_back_to_config() {
        use crate::orb_config::{OrbConfig, OrbSection};
        let config = OrbConfig {
            orb: Some(OrbSection {
                install_method: Some("apt".to_string()),
                ..OrbSection::default()
            }),
            ..OrbConfig::default()
        };
        let result = resolve_install_method(None, &config);
        assert!(
            matches!(result, InstallMethod::Apt),
            "expected Apt from config, got {result:?}"
        );
    }

    #[test]
    fn resolve_install_method_cli_overrides_config() {
        use crate::orb_config::{OrbConfig, OrbSection};
        let config = OrbConfig {
            orb: Some(OrbSection {
                install_method: Some("apt".to_string()),
                ..OrbSection::default()
            }),
            ..OrbConfig::default()
        };
        let result = resolve_install_method(Some(&InstallMethod::Binstall), &config);
        assert!(matches!(result, InstallMethod::Binstall));
    }

    #[test]
    fn resolve_install_method_local_from_config() {
        use crate::orb_config::{OrbConfig, OrbSection};
        let config = OrbConfig {
            orb: Some(OrbSection {
                install_method: Some("local".to_string()),
                ..OrbSection::default()
            }),
            ..OrbConfig::default()
        };
        let result = resolve_install_method(None, &config);
        assert!(matches!(result, InstallMethod::Local));
    }

    #[test]
    fn resolve_install_method_defaults_to_binstall() {
        use crate::orb_config::OrbConfig;
        let result = resolve_install_method(None, &OrbConfig::default());
        assert!(matches!(result, InstallMethod::Binstall));
    }

    // ── resolve_base_image ─────────────────────────────────────────────────

    #[test]
    fn resolve_base_image_uses_cli_when_provided() {
        use crate::orb_config::OrbConfig;
        let result = resolve_base_image(Some("ubuntu:22.04"), &OrbConfig::default());
        assert_eq!(result, "ubuntu:22.04");
    }

    #[test]
    fn resolve_base_image_falls_back_to_config() {
        use crate::orb_config::{OrbConfig, OrbSection};
        let config = OrbConfig {
            orb: Some(OrbSection {
                base_image: Some("ubuntu:22.04".to_string()),
                ..OrbSection::default()
            }),
            ..OrbConfig::default()
        };
        let result = resolve_base_image(None, &config);
        assert_eq!(result, "ubuntu:22.04");
    }

    #[test]
    fn resolve_base_image_cli_overrides_config() {
        use crate::orb_config::{OrbConfig, OrbSection};
        let config = OrbConfig {
            orb: Some(OrbSection {
                base_image: Some("ubuntu:22.04".to_string()),
                ..OrbSection::default()
            }),
            ..OrbConfig::default()
        };
        let result = resolve_base_image(Some("alpine:3.19"), &config);
        assert_eq!(result, "alpine:3.19");
    }

    #[test]
    fn resolve_base_image_defaults_to_debian() {
        use crate::orb_config::OrbConfig;
        let result = resolve_base_image(None, &OrbConfig::default());
        assert_eq!(result, DEFAULT_BASE_IMAGE);
    }

    // ── git_push_subcommands: config fallback ───────────────────────────────

    #[test]
    fn git_push_subcommands_falls_back_to_config_when_cli_empty() {
        // When --git-push-subcommand is not passed on the CLI but the config
        // has [orb] git_push_subcommands = ["save"], generate must use the config value
        // so that set_https_remote is generated without requiring the flag every time.
        use crate::orb_config::{OrbConfig, OrbSection};
        let config = OrbConfig {
            orb: Some(OrbSection {
                git_push_subcommands: Some(vec!["save".to_string()]),
                ..OrbSection::default()
            }),
            ..OrbConfig::default()
        };
        let resolved = resolve_git_push_subcommands(&[], &config);
        assert_eq!(resolved, vec!["save".to_string()]);
    }

    #[test]
    fn git_push_subcommands_cli_takes_precedence_over_config() {
        use crate::orb_config::{OrbConfig, OrbSection};
        let config = OrbConfig {
            orb: Some(OrbSection {
                git_push_subcommands: Some(vec!["save".to_string()]),
                ..OrbSection::default()
            }),
            ..OrbConfig::default()
        };
        let resolved = resolve_git_push_subcommands(&["commit".to_string()], &config);
        assert_eq!(resolved, vec!["commit".to_string()]);
    }
}
