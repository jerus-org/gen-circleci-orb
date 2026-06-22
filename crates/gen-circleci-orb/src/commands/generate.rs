use anyhow::Result;
use clap::ValueEnum;
use std::path::{Path, PathBuf};

use crate::{help_parser, orb_config, orb_generator, output_writer};

pub const DEFAULT_BASE_IMAGE: &str = "debian:13-slim";

/// Default image for the Rust `builder` stage (Binstall method) that
/// `cargo install`s the binary. Config-driven (`[orb].builder_image`) so a
/// pinned `…@sha256:…` digest can be kept in gen-circleci-orb.toml and tracked
/// by Renovate instead of being stripped on every regeneration.
pub const DEFAULT_BUILDER_IMAGE: &str = "rust:1-slim-trixie";

/// Base image used when the MCP feature is enabled. `build_mcp_server` compiles
/// the MCP server at runtime via `gen-orb-mcp generate --format binary`, so the
/// executor needs a Rust toolchain (cargo) present in the runtime stage.
pub const MCP_DEFAULT_BASE_IMAGE: &str = "rust:1-slim-trixie";

/// Apt packages the executor image needs to support `build_mcp_server`:
/// `libssl-dev`/`pkg-config` for the cargo compile, `gnupg` for the
/// `gen-orb-mcp save --sign` signed commit-back. Injected automatically when
/// the MCP feature is enabled. (No `openssh-client`: git uses HTTPS via
/// `set_https_remote`.)
pub const MCP_APT_PACKAGES: &[&str] = &["gnupg", "libssl-dev", "pkg-config"];

/// Whether the MCP feature is enabled in `[ci].mcp`.
pub(crate) fn mcp_enabled(config: &crate::orb_config::OrbConfig) -> bool {
    config.ci.as_ref().and_then(|c| c.mcp).unwrap_or(false)
}

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
    /// Falls back to `base_image` in the [orb] section of gen-circleci-orb.toml, then "debian:13-slim".
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

    /// Suppress auto-record for this run, even when `[record].enabled = true` in
    /// gen-circleci-orb.toml. Auto-record commits the regenerated orb source back
    /// to the current branch (GPG-signed) and pushes it, so the published orb
    /// always reflects the CLI. Whether to record, and the names of the env vars
    /// holding the signing material, are config-driven (the `[record]` section).
    /// Use this flag for local, test, and dry runs where no signing material is
    /// available.
    #[arg(long)]
    pub no_record: bool,
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

/// CLI flag takes precedence, then `[orb].base_image`. When neither is set the
/// default depends on the MCP feature: MCP needs a Rust runtime (cargo) for the
/// `--format binary` compile, so it defaults to `MCP_DEFAULT_BASE_IMAGE`;
/// otherwise `DEFAULT_BASE_IMAGE`.
pub(crate) fn resolve_base_image(
    cli: Option<&str>,
    config: &crate::orb_config::OrbConfig,
) -> String {
    cli.filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| config.orb.as_ref().and_then(|o| o.base_image.clone()))
        .unwrap_or_else(|| {
            if mcp_enabled(config) {
                MCP_DEFAULT_BASE_IMAGE.to_string()
            } else {
                DEFAULT_BASE_IMAGE.to_string()
            }
        })
}

/// The Rust `builder` stage image: `[orb].builder_image` in config, else
/// `DEFAULT_BUILDER_IMAGE`. Config-only (no CLI flag) — it's a pinned-digest
/// concern kept in gen-circleci-orb.toml, not something overridden per-run.
pub(crate) fn resolve_builder_image(config: &crate::orb_config::OrbConfig) -> String {
    config
        .orb
        .as_ref()
        .and_then(|o| o.builder_image.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_BUILDER_IMAGE.to_string())
}

/// CLI flags take precedence over `[orb].apt_packages`. When the MCP feature is
/// enabled, the build dependencies it requires (`MCP_APT_PACKAGES`) are unioned
/// in regardless, since `build_mcp_server` cannot run without them.
pub(crate) fn resolve_apt_packages(
    cli: &[String],
    config: &crate::orb_config::OrbConfig,
) -> Vec<String> {
    let mut pkgs = if !cli.is_empty() {
        cli.to_vec()
    } else {
        config
            .orb
            .as_ref()
            .and_then(|o| o.apt_packages.clone())
            .unwrap_or_default()
    };
    if mcp_enabled(config) {
        for pkg in MCP_APT_PACKAGES {
            if !pkgs.iter().any(|p| p == pkg) {
                pkgs.push((*pkg).to_string());
            }
        }
    }
    pkgs
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
            builder_image: resolve_builder_image(&orb_config),
            home_url,
            source_url,
            binary_name: cli_def.binary_name.clone(),
            git_push_subcommands: resolve_git_push_subcommands(
                &self.git_push_subcommands,
                &orb_config,
            ),
            circleci_cli_version: self.circleci_cli_version.clone(),
            apt_packages: resolve_apt_packages(&self.apt_packages, &orb_config),
        };

        let files = orb_generator::generate(&cli_def, &opts, Some(&orb_config));

        tracing::info!("Generated {} file(s)", files.len());

        let custom_files = orb_config
            .orb
            .as_ref()
            .and_then(|o| o.custom_files.clone())
            .unwrap_or_default();
        let report = output_writer::write_tree(&orb_root, &files, &custom_files, self.dry_run)?;

        println!(
            "Done: {} created, {} updated, {} unchanged, {} removed",
            report.created, report.updated, report.unchanged, report.removed
        );

        // Auto-record is config-driven: only when `[record].enabled = true` and
        // not suppressed by --no-record / --dry-run. The branch policy is
        // centralized here so the orb "just works" — we only record on a regular
        // PR branch, never on `main` or a forked-PR build (see
        // should_record_on_branch).
        if !self.no_record && !self.dry_run {
            if let Some(record) = orb_config.record.as_ref().filter(|r| r.enabled) {
                if should_record_on_branch(|k| std::env::var(k).ok()) {
                    record_orb(&orb_root, record)?;
                } else {
                    println!(
                        "Skipping auto-record: not on a recordable PR branch \
                         (main, forked PR, or no CIRCLE_BRANCH)."
                    );
                }
            }
        }
        Ok(())
    }
}

/// Whether the current CI branch is one we should record (commit + push) to.
/// We only record on a regular PR branch:
/// - never on `main` — a push there would need branch-protection bypass we
///   deliberately do not use (the write token has only `contents:write`);
/// - never on a forked-PR build — no write access to the fork, and CircleCI
///   withholds context secrets from fork builds anyway;
/// - never when `CIRCLE_BRANCH` is unset — i.e. local runs, which must not push.
///
/// The injectable getter keeps this unit-testable.
fn should_record_on_branch(get: impl Fn(&str) -> Option<String>) -> bool {
    let branch = get("CIRCLE_BRANCH").unwrap_or_default();
    if branch.is_empty() || branch == "main" {
        return false;
    }
    // CIRCLE_PR_REPONAME is set only on builds originating from a fork.
    if get("CIRCLE_PR_REPONAME")
        .map(|s| !s.is_empty())
        .unwrap_or(false)
    {
        return false;
    }
    true
}

/// Bot identity + GPG material for recording (committing) the regenerated orb.
#[derive(Debug)]
struct RecordEnv {
    gpg_key_b64: String,
    gpg_trust: String,
    user_name: String,
    user_email: String,
    sign_key: String,
}

/// Read the signing material from the environment variables **named** by the
/// `[record]` config, via `get`. The names are the consumer's choice — this
/// function never hardcodes a `BOT_*` convention. Returns an error naming the
/// first missing variable (a real CI misconfiguration when recording is
/// enabled). The injectable getter keeps this unit-testable.
fn read_record_env(
    record: &crate::orb_config::RecordConfig,
    get: impl Fn(&str) -> Option<String>,
) -> Result<RecordEnv> {
    let req = |name: &str| {
        get(name).ok_or_else(|| {
            anyhow::anyhow!(
                "env var `{name}` (named in [record]) is not set, but [record].enabled = true"
            )
        })
    };
    Ok(RecordEnv {
        gpg_key_b64: req(&record.gpg_key_env)?,
        gpg_trust: req(&record.gpg_trust_env)?,
        user_name: req(&record.user_name_env)?,
        user_email: req(&record.user_email_env)?,
        sign_key: req(&record.signing_key_env)?,
    })
}

/// Build the educational message shown when the ambient push is rejected.
///
/// The ambient client (`pcu::Client::new_local()`) pushes with whatever auth
/// `checkout` left in the environment. In a standard CircleCI + GitHub checkout
/// that is the **read-only** deploy key, so the push is rejected ("the key you
/// are authenticating with has been marked as read only"). This is expected, not
/// a tool bug — surface it clearly and point at the recommended setup (a single
/// user-supplied end-of-workflow push job that commits + pushes the regenerated
/// orb with real write authority). `err` is appended for diagnosis.
pub(crate) fn ambient_push_failure_message(err: &impl std::fmt::Display) -> String {
    format!(
        "Could not push the regenerated orb using the ambient CI credentials.\n\
         \n\
         This is expected in a standard CircleCI + GitHub checkout: `checkout` \
         provisions a READ-ONLY deploy key, and auto-record pushes with whatever \
         authorization the environment already holds (it carries no token of its \
         own). A read-only key cannot push.\n\
         \n\
         Recommended setup: do not rely on this ambient push. Configure a single \
         end-of-workflow push job (with real write authority) that commits and \
         pushes the regenerated orb — the producing job persists the changed \
         files and the push happens once, after validation succeeds.\n\
         \n\
         Underlying error: {err}"
    )
}

/// Normalize `orb_root` into a pathspec libgit2's `add_all` will match.
///
/// `Generate::output` defaults to `"."`, so `orb_root` is built as `./orb`. A
/// leading `.` (`Component::CurDir`) makes the libgit2 pathspec match **no**
/// repo entries (entry paths are stored repo-root-relative as `orb/...`, never
/// `./orb/...`), so `stage_paths` would silently stage nothing — the orb source
/// is never recorded even though regeneration changed files on disk (the
/// gen-orb-mcp #207 regression: `0 created, 11 updated` yet "nothing to
/// record"). Dropping the `.` components yields `orb`, which matches.
pub(crate) fn normalize_orb_pathspec(orb_root: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let normalized: std::path::PathBuf = orb_root
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect();
    if normalized.as_os_str().is_empty() {
        std::path::PathBuf::from(".")
    } else {
        normalized
    }
}

/// Commit the regenerated orb source under `orb_root` back to the current (PR)
/// branch (GPG-signed) and push, so the change is reviewable in the PR. No-op if
/// nothing changed. The commit identity and signing key are passed to pcu
/// explicitly (via SignConfig), so this does not depend on git-config visibility
/// in CI.
fn record_orb(orb_root: &std::path::Path, record: &crate::orb_config::RecordConfig) -> Result<()> {
    let env = read_record_env(record, |k| std::env::var(k).ok())?;
    pcu::import_gpg_key(&env.gpg_key_b64, &env.gpg_trust)
        .map_err(|e| anyhow::anyhow!("GPG import failed: {e}"))?;

    // Ambient client: carries no GitHub credentials of its own. Local git ops
    // (stage, signed commit) work without auth; the push uses whatever
    // authorization `checkout` left in the environment (see record's docs).
    let client = pcu::Client::new_local()
        .map_err(|e| anyhow::anyhow!("Failed to create pcu client: {e}"))?;

    // Stage a repo-relative pathspec (no leading `./`) so libgit2 actually
    // matches the regenerated files — see normalize_orb_pathspec.
    let stage_path = normalize_orb_pathspec(orb_root);
    use pcu::GitOps;
    client
        .stage_paths(&[stage_path.as_path()])
        .map_err(|e| anyhow::anyhow!("Failed to stage orb source: {e}"))?;

    // Skip an empty commit when the regenerated orb is identical to HEAD.
    let repo = git2::Repository::discover(".")
        .map_err(|e| anyhow::anyhow!("Not inside a git repository: {e}"))?;
    let mut index = repo.index()?;
    let new_tree = repo.find_tree(index.write_tree()?)?;
    let head_tree = repo
        .head()
        .ok()
        .and_then(|h| h.peel_to_commit().ok())
        .map(|c| c.tree())
        .transpose()?;
    let diff = repo.diff_tree_to_tree(head_tree.as_ref(), Some(&new_tree), None)?;
    if diff.deltas().count() == 0 {
        println!("Orb source unchanged — nothing to record.");
        return Ok(());
    }

    let message = "chore: regenerate orb [skip ci]";
    let sign_config = pcu::SignConfig::new(pcu::Sign::Gpg)
        .with_identity(&env.user_name, &env.user_email)
        .with_signing_key(&env.sign_key);
    client
        .commit_staged(sign_config, message, "", None)
        .map_err(|e| anyhow::anyhow!("Failed to sign and commit regenerated orb: {e}"))?;
    println!("Recorded regenerated orb: {message}");
    client
        .push_commit("", None, false, &env.user_name)
        .map_err(|e| anyhow::anyhow!("{}", ambient_push_failure_message(&e)))?;
    println!("Pushed regenerated orb to remote.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Build a git repo with `orb/src/foo.yml` committed, then return the repo.
    fn repo_with_committed_orb(dir: &TempDir) -> git2::Repository {
        let repo = git2::Repository::init(dir.path()).unwrap();
        fs::create_dir_all(dir.path().join("orb/src")).unwrap();
        fs::write(dir.path().join("orb/src/foo.yml"), "old\n").unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_all(["orb"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_oid = index.write_tree().unwrap();
        {
            let tree = repo.find_tree(tree_oid).unwrap();
            let sig = git2::Signature::now("t", "t@t").unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
                .unwrap();
        }
        repo
    }

    /// Mirror `record_orb`'s stage→write_tree→diff change-detection for a given
    /// `orb_root` path form. Returns the delta count.
    fn detect_deltas_after_staging(repo: &git2::Repository, orb_root: &std::path::Path) -> usize {
        let mut index = repo.index().unwrap();
        index
            .add_all([orb_root].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let new_tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let head_tree = repo
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .tree()
            .unwrap();
        let diff = repo
            .diff_tree_to_tree(Some(&head_tree), Some(&new_tree), None)
            .unwrap();
        diff.deltas().count()
    }

    /// A modification under `orb/` must be detected by `record_orb`'s
    /// stage+diff logic. With `--output "."` the resolved orb_root is `./orb`;
    /// `normalize_orb_pathspec` must turn it into a pathspec libgit2 `add_all`
    /// matches so auto-record does not falsely report "Orb source unchanged —
    /// nothing to record" (the gen-orb-mcp #207 regression: `0 created,
    /// 11 updated` yet "nothing to record").
    #[test]
    fn record_detects_orb_modification_with_dot_slash_output() {
        let dir = TempDir::new().unwrap();
        let repo = repo_with_committed_orb(&dir);
        // Regeneration changes a tracked orb file on disk.
        fs::write(dir.path().join("orb/src/foo.yml"), "new\n").unwrap();

        // orb_root as built by `Generate`: output (default ".") joined with "orb".
        let orb_root = std::path::PathBuf::from(".").join("orb");
        assert_eq!(orb_root, std::path::Path::new("./orb"));

        // The raw `./orb` pathspec stages nothing (proves the root cause)...
        assert_eq!(
            detect_deltas_after_staging(&repo, &orb_root),
            0,
            "expected the unnormalized ./orb pathspec to stage nothing"
        );
        // ...while the normalized pathspec stages the change and is detected.
        let deltas = detect_deltas_after_staging(&repo, &normalize_orb_pathspec(&orb_root));
        assert!(
            deltas > 0,
            "modification under orb/ not detected after normalizing orb_root — \
             auto-record would falsely skip"
        );
    }

    #[test]
    fn normalize_orb_pathspec_strips_cur_dir_components() {
        use std::path::{Path, PathBuf};
        assert_eq!(
            normalize_orb_pathspec(Path::new("./orb")),
            PathBuf::from("orb")
        );
        assert_eq!(
            normalize_orb_pathspec(Path::new("orb")),
            PathBuf::from("orb")
        );
        assert_eq!(
            normalize_orb_pathspec(Path::new("./a/./b")),
            PathBuf::from("a/b")
        );
        // A bare "." must not normalize to empty (which add_all would reject).
        assert_eq!(normalize_orb_pathspec(Path::new(".")), PathBuf::from("."));
    }

    // ── read_record_env ─────────────────────────────────────────────────────

    /// A [record] config whose env-var names are deliberately *not* the BOT_*
    /// convention, proving the names are read from config and not hardcoded.
    fn custom_record_config() -> crate::orb_config::RecordConfig {
        crate::orb_config::RecordConfig {
            enabled: true,
            gpg_key_env: "MY_GPG_KEY".to_string(),
            gpg_trust_env: "MY_GPG_TRUST".to_string(),
            user_name_env: "MY_NAME".to_string(),
            user_email_env: "MY_EMAIL".to_string(),
            signing_key_env: "MY_SIGN_KEY".to_string(),
            contexts: vec!["my-signing-context".to_string()],
        }
    }

    fn full_custom_env(k: &str) -> Option<String> {
        match k {
            "MY_GPG_KEY" => Some("key".to_string()),
            "MY_GPG_TRUST" => Some("trust".to_string()),
            "MY_NAME" => Some("Bot Name".to_string()),
            "MY_EMAIL" => Some("bot@example.com".to_string()),
            "MY_SIGN_KEY" => Some("DEADBEEF".to_string()),
            _ => None,
        }
    }

    #[test]
    fn read_record_env_reads_vars_named_in_config() {
        let env =
            read_record_env(&custom_record_config(), full_custom_env).expect("all vars present");
        assert_eq!(env.user_name, "Bot Name");
        assert_eq!(env.user_email, "bot@example.com");
        assert_eq!(env.sign_key, "DEADBEEF");
        assert_eq!(env.gpg_key_b64, "key");
        assert_eq!(env.gpg_trust, "trust");
    }

    // ── ambient_push_failure_message ────────────────────────────────────────

    /// The push-failure guidance must name the read-only-key cause and point at
    /// the recommended end-of-workflow push job, and append the underlying error
    /// for diagnosis — so a CI operator understands it's expected, not a bug.
    #[test]
    fn ambient_push_failure_message_explains_and_includes_error() {
        let msg = ambient_push_failure_message(&"marked as read only");
        assert!(msg.contains("read-only") || msg.to_lowercase().contains("read only"));
        assert!(
            msg.to_lowercase().contains("end-of-workflow push job"),
            "should point at the recommended setup"
        );
        assert!(
            msg.contains("marked as read only"),
            "should append the underlying error for diagnosis"
        );
    }

    // ── should_record_on_branch ─────────────────────────────────────────────

    #[test]
    fn record_on_regular_pr_branch() {
        let env = |k: &str| match k {
            "CIRCLE_BRANCH" => Some("feat/x".to_string()),
            _ => None,
        };
        assert!(should_record_on_branch(env));
    }

    #[test]
    fn no_record_on_main() {
        let env = |k: &str| match k {
            "CIRCLE_BRANCH" => Some("main".to_string()),
            _ => None,
        };
        assert!(!should_record_on_branch(env));
    }

    #[test]
    fn no_record_on_forked_pr() {
        let env = |k: &str| match k {
            "CIRCLE_BRANCH" => Some("pull/42".to_string()),
            "CIRCLE_PR_REPONAME" => Some("contributor-fork".to_string()),
            _ => None,
        };
        assert!(!should_record_on_branch(env));
    }

    #[test]
    fn no_record_when_branch_unset_locally() {
        assert!(!should_record_on_branch(|_| None));
    }

    #[test]
    fn read_record_env_errors_on_missing_var() {
        let missing_name = |k: &str| {
            if k == "MY_NAME" {
                None
            } else {
                full_custom_env(k)
            }
        };
        let err = read_record_env(&custom_record_config(), missing_name)
            .unwrap_err()
            .to_string();
        assert!(err.contains("MY_NAME"), "unexpected: {err}");
    }

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

    // ── resolve_builder_image ──────────────────────────────────────────────

    #[test]
    fn resolve_builder_image_defaults_to_rust_trixie() {
        use crate::orb_config::OrbConfig;
        assert_eq!(
            resolve_builder_image(&OrbConfig::default()),
            DEFAULT_BUILDER_IMAGE
        );
    }

    #[test]
    fn resolve_builder_image_uses_config_pinned_digest() {
        use crate::orb_config::{OrbConfig, OrbSection};
        let pinned = "rust:1-slim-trixie@sha256:abc123";
        let config = OrbConfig {
            orb: Some(OrbSection {
                builder_image: Some(pinned.to_string()),
                ..OrbSection::default()
            }),
            ..OrbConfig::default()
        };
        assert_eq!(resolve_builder_image(&config), pinned);
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

    // ── resolve_apt_packages ───────────────────────────────────────────────

    #[test]
    fn resolve_apt_packages_uses_cli_when_provided() {
        use crate::orb_config::OrbConfig;
        let cli = vec!["libssl-dev".to_string(), "pkg-config".to_string()];
        let result = resolve_apt_packages(&cli, &OrbConfig::default());
        assert_eq!(result, vec!["libssl-dev", "pkg-config"]);
    }

    #[test]
    fn resolve_apt_packages_falls_back_to_config() {
        use crate::orb_config::{OrbConfig, OrbSection};
        let config = OrbConfig {
            orb: Some(OrbSection {
                apt_packages: Some(vec!["libssl-dev".to_string(), "pkg-config".to_string()]),
                ..OrbSection::default()
            }),
            ..OrbConfig::default()
        };
        let result = resolve_apt_packages(&[], &config);
        assert_eq!(result, vec!["libssl-dev", "pkg-config"]);
    }

    #[test]
    fn resolve_apt_packages_cli_overrides_config() {
        use crate::orb_config::{OrbConfig, OrbSection};
        let config = OrbConfig {
            orb: Some(OrbSection {
                apt_packages: Some(vec!["libssl-dev".to_string()]),
                ..OrbSection::default()
            }),
            ..OrbConfig::default()
        };
        let result = resolve_apt_packages(&["cmake".to_string()], &config);
        assert_eq!(result, vec!["cmake"]);
    }

    #[test]
    fn resolve_apt_packages_defaults_to_empty() {
        use crate::orb_config::OrbConfig;
        let result = resolve_apt_packages(&[], &OrbConfig::default());
        assert!(result.is_empty());
    }

    // ── MCP feature auto-provisions the executor image ─────────────────────

    fn mcp_config() -> crate::orb_config::OrbConfig {
        use crate::orb_config::{CiSection, OrbConfig};
        OrbConfig {
            ci: Some(CiSection {
                mcp: Some(true),
                ..CiSection::default()
            }),
            ..OrbConfig::default()
        }
    }

    #[test]
    fn mcp_enabled_reads_ci_section() {
        use crate::orb_config::OrbConfig;
        assert!(mcp_enabled(&mcp_config()));
        assert!(!mcp_enabled(&OrbConfig::default()));
    }

    #[test]
    fn resolve_base_image_defaults_to_rust_when_mcp_enabled() {
        let result = resolve_base_image(None, &mcp_config());
        assert_eq!(result, MCP_DEFAULT_BASE_IMAGE);
    }

    #[test]
    fn resolve_base_image_explicit_config_overrides_mcp_default() {
        use crate::orb_config::{CiSection, OrbConfig, OrbSection};
        let config = OrbConfig {
            orb: Some(OrbSection {
                base_image: Some("rust:1.85-slim".to_string()),
                ..OrbSection::default()
            }),
            ci: Some(CiSection {
                mcp: Some(true),
                ..CiSection::default()
            }),
            ..OrbConfig::default()
        };
        assert_eq!(resolve_base_image(None, &config), "rust:1.85-slim");
    }

    #[test]
    fn resolve_apt_packages_injects_mcp_deps_when_enabled() {
        let result = resolve_apt_packages(&[], &mcp_config());
        for pkg in MCP_APT_PACKAGES {
            assert!(
                result.iter().any(|p| p == pkg),
                "mcp must inject {pkg}; got {result:?}"
            );
        }
    }

    #[test]
    fn resolve_apt_packages_mcp_deps_union_with_user_packages() {
        use crate::orb_config::{CiSection, OrbConfig, OrbSection};
        let config = OrbConfig {
            orb: Some(OrbSection {
                apt_packages: Some(vec!["cmake".to_string()]),
                ..OrbSection::default()
            }),
            ci: Some(CiSection {
                mcp: Some(true),
                ..CiSection::default()
            }),
            ..OrbConfig::default()
        };
        let result = resolve_apt_packages(&[], &config);
        assert!(
            result.iter().any(|p| p == "cmake"),
            "keep user pkg: {result:?}"
        );
        assert!(
            result.iter().any(|p| p == "gnupg"),
            "inject gnupg: {result:?}"
        );
    }

    #[test]
    fn resolve_apt_packages_no_mcp_deps_when_disabled() {
        use crate::orb_config::OrbConfig;
        let result = resolve_apt_packages(&[], &OrbConfig::default());
        assert!(
            !result.iter().any(|p| p == "gnupg"),
            "no mcp deps when mcp disabled: {result:?}"
        );
    }

    #[test]
    fn resolve_apt_packages_mcp_deps_not_duplicated() {
        use crate::orb_config::{CiSection, OrbConfig, OrbSection};
        let config = OrbConfig {
            orb: Some(OrbSection {
                apt_packages: Some(vec!["gnupg".to_string()]),
                ..OrbSection::default()
            }),
            ci: Some(CiSection {
                mcp: Some(true),
                ..CiSection::default()
            }),
            ..OrbConfig::default()
        };
        let result = resolve_apt_packages(&[], &config);
        assert_eq!(
            result.iter().filter(|p| *p == "gnupg").count(),
            1,
            "gnupg must not be duplicated: {result:?}"
        );
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
