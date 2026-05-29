use anyhow::Result;
use indexmap::IndexMap;
use std::path::PathBuf;

use crate::{
    ci_patcher,
    commands::generate::Generate,
    orb_config::{OrbConfig, OrbSection, SubcommandConfig},
};

pub const DEFAULT_DOCKER_ORB_VERSION: &str = "3.0.1";
pub const DEFAULT_DOCKER_CONTEXT: &str = "docker-credentials";
pub const DEFAULT_ORB_CONTEXT: &str = "orb-publishing";
pub const DEFAULT_MCP_CONTEXT: &str = "pcu-app";
pub const DEFAULT_MCP_EARLIEST_VERSION: &str = "0.0.1";

/// Values resolved by the interactive dialogue (or non-interactive fallback).
/// These are used by both `PatchOpts` and the bootstrap config.
pub(crate) struct GatheredExtras {
    pub home_url: Option<String>,
    pub source_url: Option<String>,
    pub git_push_subcommands: Vec<String>,
    pub docker_context: String,
    pub orb_context: String,
    pub mcp_context: String,
    pub mcp_earliest_version: String,
}

fn is_non_interactive(dry_run: bool) -> bool {
    dry_run || std::env::var("CI").is_ok()
}

/// Wire orb generation into an existing repo's CI configuration.
#[derive(Debug, clap::Args)]
pub struct Init {
    /// Name of the binary to introspect (must be on PATH).
    #[arg(long)]
    pub binary: String,

    /// CircleCI namespace(s) to publish the orb under as a public orb (repeatable).
    /// Must be set correctly on first init — visibility cannot be changed after the orb is created.
    #[arg(long = "public-orb-namespace")]
    pub public_orb_namespaces: Vec<String>,

    /// CircleCI namespace(s) to publish the orb under as a private orb (repeatable).
    /// Each listed namespace gets `--private` in its `circleci orb create` command.
    /// Must be set correctly on first init — visibility cannot be changed after the orb is created.
    #[arg(long = "private-orb-namespace")]
    pub private_orb_namespaces: Vec<String>,

    /// Name of the build/validation workflow to patch.
    #[arg(long)]
    pub build_workflow: String,

    /// Name of the release workflow to patch.
    #[arg(long)]
    pub release_workflow: String,

    /// Job in the build workflow that regenerate-orb should require.
    #[arg(long)]
    pub requires_job: Option<String>,

    /// Tag prefix used by `toolkit/release_crate` for the crate (e.g. `gen-orb-mcp-v`).
    /// Used to filter the `orb-release:` workflow trigger in config.yml and to normalise
    /// `CIRCLE_TAG` for `orb-tools/publish`.
    #[arg(long)]
    pub crate_tag_prefix: String,

    /// Job in the release workflow after which the generated release jobs
    /// (build-binary-release, pack-orb-release, build-container, ensure-orb-registered)
    /// should be gated. This is the sole mechanism for specifying where the generated
    /// jobs plug into the existing pipeline topology.
    #[arg(long)]
    pub release_after_job: String,

    /// Output directory for the generated orb source (relative to repo root).
    #[arg(long, default_value = "orb")]
    pub orb_dir: String,

    /// Path to the .circleci/ directory.
    #[arg(long, default_value = ".circleci")]
    pub ci_dir: PathBuf,

    /// circleci/orb-tools version to pin in generated CI.
    #[arg(long, default_value = "12.3.3")]
    pub orb_tools_version: String,

    /// circleci/docker orb version to pin in generated CI.
    #[arg(long, default_value = DEFAULT_DOCKER_ORB_VERSION)]
    pub docker_orb_version: String,

    /// Docker Hub (or registry) namespace for the built container image.
    #[arg(long)]
    pub docker_namespace: String,

    /// CircleCI context name holding Docker Hub credentials (DOCKER_LOGIN, DOCKER_PASSWORD).
    /// Prompted interactively if not supplied.
    #[arg(long)]
    pub docker_context: Option<String>,

    /// CircleCI context name holding orb publishing credentials (CIRCLECI_CLI_TOKEN).
    /// Prompted interactively if not supplied.
    #[arg(long)]
    pub orb_context: Option<String>,

    /// Version of the jerus-org/gen-circleci-orb orb to pin in generated CI.
    /// Defaults to the version of this binary (orb and crate are released together).
    #[arg(long, default_value = env!("CARGO_PKG_VERSION"))]
    pub gen_circleci_orb_version: String,

    /// Wire in gen-orb-mcp MCP server generation + publish after orb publish.
    #[arg(long)]
    pub mcp: bool,

    /// Earliest orb version to include when priming prior-version snapshots.
    /// Passed to gen-circleci-orb/build_mcp_server as `earliest_version`.
    /// Only used when --mcp is enabled. Prompted interactively if not supplied.
    #[arg(long)]
    pub mcp_earliest_version: Option<String>,

    /// CircleCI context providing push authority for MCP server build + publish + save steps.
    /// Needs: GITHUB_TOKEN (GitHub App token, contents:write + bypass branch protection),
    /// BOT_GPG_KEY, BOT_TRUST, BOT_USER_NAME, BOT_USER_EMAIL, BOT_SIGN_KEY.
    /// Only used when --mcp is enabled. Prompted interactively if not supplied.
    #[arg(long)]
    pub mcp_context: Option<String>,

    /// Subcommand names whose generated jobs should include a `set_https_remote` step
    /// (repeatable). Use for subcommands that push to git (e.g. `save`).
    #[arg(long, value_delimiter = ',')]
    pub git_push_subcommands: Vec<String>,

    /// Home URL for the orb (shown in the CircleCI registry).
    #[arg(long)]
    pub home_url: Option<String>,

    /// Source URL for the orb (shown in the CircleCI registry).
    #[arg(long)]
    pub source_url: Option<String>,

    /// Show planned changes without modifying any files.
    #[arg(long)]
    pub dry_run: bool,
}

pub(crate) fn build_bootstrap_config(
    binary: &str,
    namespaces: &[String],
    orb_dir: &str,
    home_url: Option<&str>,
    source_url: Option<&str>,
    git_push_subcommands: &[String],
) -> OrbConfig {
    let mut subcommands = IndexMap::new();
    subcommands.insert(
        "help".to_string(),
        SubcommandConfig {
            generate_job: Some(false),
            param: None,
        },
    );
    OrbConfig {
        orb: Some(OrbSection {
            binary: Some(binary.to_string()),
            namespaces: Some(namespaces.to_vec()),
            orb_dir: Some(orb_dir.to_string()),
            base_image: None,
            install_method: None,
            home_url: home_url.map(str::to_string),
            source_url: source_url.map(str::to_string),
            git_push_subcommands: if git_push_subcommands.is_empty() {
                None
            } else {
                Some(git_push_subcommands.to_vec())
            },
        }),
        orbs: None,
        subcommand: Some(subcommands),
        job_group: None,
        extra_job: None,
    }
}

impl Init {
    pub(crate) fn gather_extras(&self) -> Result<GatheredExtras> {
        if is_non_interactive(self.dry_run) {
            return Ok(GatheredExtras {
                home_url: self.home_url.clone(),
                source_url: self.source_url.clone(),
                git_push_subcommands: self.git_push_subcommands.clone(),
                docker_context: self
                    .docker_context
                    .clone()
                    .unwrap_or_else(|| DEFAULT_DOCKER_CONTEXT.to_string()),
                orb_context: self
                    .orb_context
                    .clone()
                    .unwrap_or_else(|| DEFAULT_ORB_CONTEXT.to_string()),
                mcp_context: self
                    .mcp_context
                    .clone()
                    .unwrap_or_else(|| DEFAULT_MCP_CONTEXT.to_string()),
                mcp_earliest_version: self
                    .mcp_earliest_version
                    .clone()
                    .unwrap_or_else(|| DEFAULT_MCP_EARLIEST_VERSION.to_string()),
            });
        }

        // Interactive mode — prompt for each field not already provided.
        use dialoguer::Input;

        let home_url = {
            let val = Input::<String>::new()
                .with_prompt("Home URL for orb registry (Enter to skip)")
                .default(self.home_url.clone().unwrap_or_default())
                .allow_empty(true)
                .interact_text()?;
            if val.is_empty() {
                None
            } else {
                Some(val)
            }
        };

        let source_url = {
            let val = Input::<String>::new()
                .with_prompt("Source URL for orb registry (Enter to skip)")
                .default(self.source_url.clone().unwrap_or_default())
                .allow_empty(true)
                .interact_text()?;
            if val.is_empty() {
                None
            } else {
                Some(val)
            }
        };

        let git_push_subcommands = {
            let current = self.git_push_subcommands.join(",");
            let val = Input::<String>::new()
                .with_prompt("Subcommands that push to git, comma-separated (e.g. save)")
                .default(current)
                .allow_empty(true)
                .interact_text()?;
            val.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        };

        let docker_context = Input::<String>::new()
            .with_prompt("Docker context name (needs: DOCKER_LOGIN, DOCKER_PASSWORD)")
            .default(
                self.docker_context
                    .clone()
                    .unwrap_or_else(|| DEFAULT_DOCKER_CONTEXT.to_string()),
            )
            .interact_text()?;

        let orb_context = Input::<String>::new()
            .with_prompt("Orb publishing context name (needs: CIRCLECI_CLI_TOKEN)")
            .default(
                self.orb_context
                    .clone()
                    .unwrap_or_else(|| DEFAULT_ORB_CONTEXT.to_string()),
            )
            .interact_text()?;

        let mcp_context = if self.mcp {
            Input::<String>::new()
                .with_prompt(
                    "MCP context name (needs: GITHUB_TOKEN with contents:write + bypass branch protection, BOT_GPG_KEY, BOT_TRUST, BOT_USER_NAME, BOT_USER_EMAIL, BOT_SIGN_KEY)",
                )
                .default(
                    self.mcp_context
                        .clone()
                        .unwrap_or_else(|| DEFAULT_MCP_CONTEXT.to_string()),
                )
                .interact_text()?
        } else {
            self.mcp_context
                .clone()
                .unwrap_or_else(|| DEFAULT_MCP_CONTEXT.to_string())
        };

        let mcp_earliest_version = if self.mcp {
            Input::<String>::new()
                .with_prompt("Earliest orb version to include in MCP snapshots")
                .default(
                    self.mcp_earliest_version
                        .clone()
                        .unwrap_or_else(|| DEFAULT_MCP_EARLIEST_VERSION.to_string()),
                )
                .interact_text()?
        } else {
            self.mcp_earliest_version
                .clone()
                .unwrap_or_else(|| DEFAULT_MCP_EARLIEST_VERSION.to_string())
        };

        Ok(GatheredExtras {
            home_url,
            source_url,
            git_push_subcommands,
            docker_context,
            orb_context,
            mcp_context,
            mcp_earliest_version,
        })
    }

    pub fn run(&self) -> Result<()> {
        let extras = self.gather_extras()?;
        let namespaces: Vec<String> = self
            .public_orb_namespaces
            .iter()
            .chain(self.private_orb_namespaces.iter())
            .cloned()
            .collect();

        // Step 1: generate orb source files
        tracing::info!("Generating orb source into ./{}", self.orb_dir);
        let gen = Generate {
            binary: self.binary.clone(),
            namespaces: namespaces.clone(),
            output: PathBuf::from("."),
            orb_dir: self.orb_dir.clone(),
            install_method: crate::commands::generate::InstallMethod::Binstall,
            base_image: crate::commands::generate::DEFAULT_BASE_IMAGE.to_string(),
            home_url: extras.home_url.clone(),
            source_url: extras.source_url.clone(),
            git_push_subcommands: extras.git_push_subcommands.clone(),
            circleci_cli_version: None,
            apt_packages: vec![],
            dry_run: self.dry_run,
            config: None,
        };
        gen.run()?;

        // Step 2: patch CI configs
        let opts = ci_patcher::PatchOpts {
            binary: self.binary.clone(),
            namespaces,
            docker_namespace: self.docker_namespace.clone(),
            orb_dir: self.orb_dir.clone(),
            build_workflow: self.build_workflow.clone(),
            release_workflow: self.release_workflow.clone(),
            requires_job: self.requires_job.clone(),
            crate_tag_prefix: self.crate_tag_prefix.clone(),
            release_after_job: self.release_after_job.clone(),
            orb_tools_version: self.orb_tools_version.clone(),
            docker_orb_version: self.docker_orb_version.clone(),
            docker_context: extras.docker_context.clone(),
            orb_context: extras.orb_context.clone(),
            private_namespaces: self.private_orb_namespaces.clone(),
            gen_circleci_orb_version: self.gen_circleci_orb_version.clone(),
            mcp: self.mcp,
            mcp_earliest_version: extras.mcp_earliest_version.clone(),
            mcp_context: extras.mcp_context.clone(),
        };

        let summary = ci_patcher::apply_patches(&self.ci_dir, &opts, self.dry_run)?;
        for line in &summary {
            println!("{line}");
        }

        // Step 3: write bootstrap gen-circleci-orb.toml
        let config_path = std::path::Path::new("gen-circleci-orb.toml");
        let bootstrap = build_bootstrap_config(
            &self.binary,
            opts.namespaces.as_slice(),
            &self.orb_dir,
            extras.home_url.as_deref(),
            extras.source_url.as_deref(),
            &extras.git_push_subcommands,
        );
        if self.dry_run {
            let content = toml::to_string_pretty(&bootstrap)?;
            println!("(dry-run) Would write {}", config_path.display());
            println!("{content}");
            println!("(dry-run: no files written)");
        } else {
            crate::orb_config::save_config(config_path, &bootstrap)?;
            println!("Wrote {}", config_path.display());
            println!("Done.");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_docker_orb_version_matches_registry() {
        // The CircleCI registry has circleci/docker@3.0.1 as latest.
        // 3.2.0 does not exist and causes "Cannot find circleci/docker@3.2.0" errors.
        assert_eq!(
            DEFAULT_DOCKER_ORB_VERSION, "3.0.1",
            "DEFAULT_DOCKER_ORB_VERSION must be the registry-available version"
        );
    }

    // ── Phase 6: bootstrap config written by init ───────────────────────────

    #[test]
    fn bootstrap_config_has_orb_section_with_binary() {
        let config =
            build_bootstrap_config("mytool", &["my-org".to_string()], "orb", None, None, &[]);
        assert!(
            config.orb.is_some(),
            "bootstrap config must have [orb] section"
        );
        assert_eq!(
            config.orb.as_ref().unwrap().binary.as_deref(),
            Some("mytool")
        );
    }

    #[test]
    fn bootstrap_config_has_namespaces() {
        let config = build_bootstrap_config(
            "mytool",
            &["ns1".to_string(), "ns2".to_string()],
            "orb",
            None,
            None,
            &[],
        );
        assert_eq!(
            config.orb.as_ref().unwrap().namespaces.as_deref(),
            Some(&["ns1".to_string(), "ns2".to_string()][..])
        );
    }

    #[test]
    fn bootstrap_config_suppresses_help_subcommand() {
        let config =
            build_bootstrap_config("mytool", &["my-org".to_string()], "orb", None, None, &[]);
        let subcommands = config
            .subcommand
            .as_ref()
            .expect("subcommand section missing");
        let help = subcommands.get("help").expect("help entry missing");
        assert_eq!(
            help.generate_job,
            Some(false),
            "help subcommand must be suppressed in bootstrap config"
        );
    }

    #[test]
    fn init_has_git_push_subcommands_field() {
        // Init must expose --git-push-subcommands so the caller can name subcommands
        // (e.g. "save") that need a set_https_remote step in their generated job.
        let init = Init {
            binary: "mytool".to_string(),
            public_orb_namespaces: vec!["my-org".to_string()],
            private_orb_namespaces: vec![],
            build_workflow: "validation".to_string(),
            release_workflow: "orb-release".to_string(),
            requires_job: None,
            crate_tag_prefix: "mytool-v".to_string(),
            release_after_job: "publish-orb".to_string(),
            orb_dir: "orb".to_string(),
            ci_dir: std::path::PathBuf::from(".circleci"),
            orb_tools_version: "12.3.3".to_string(),
            docker_orb_version: "3.0.1".to_string(),
            docker_namespace: "my-docker-ns".to_string(),
            docker_context: None,
            orb_context: None,
            gen_circleci_orb_version: "0.0.1".to_string(),
            mcp: false,
            mcp_earliest_version: None,
            mcp_context: None,
            dry_run: false,
            git_push_subcommands: vec!["save".to_string()],
            home_url: None,
            source_url: None,
        };
        assert_eq!(
            init.git_push_subcommands,
            vec!["save".to_string()],
            "Init must hold git_push_subcommands and pass it through to Generate"
        );
    }

    #[test]
    fn bootstrap_config_includes_git_push_subcommands() {
        let config = build_bootstrap_config(
            "mytool",
            &["my-org".to_string()],
            "orb",
            None,
            None,
            &["save".to_string()],
        );
        assert_eq!(
            config.orb.as_ref().unwrap().git_push_subcommands.as_deref(),
            Some(&["save".to_string()][..])
        );
    }

    #[test]
    fn bootstrap_config_git_push_subcommands_none_when_empty() {
        let config =
            build_bootstrap_config("mytool", &["my-org".to_string()], "orb", None, None, &[]);
        assert_eq!(
            config.orb.as_ref().unwrap().git_push_subcommands,
            None,
            "empty slice must produce None (not an empty list) to keep the TOML clean"
        );
    }

    #[test]
    fn bootstrap_config_includes_home_and_source_url() {
        let config = build_bootstrap_config(
            "mytool",
            &["my-org".to_string()],
            "orb",
            Some("https://example.com/home"),
            Some("https://example.com/source"),
            &[],
        );
        assert_eq!(
            config.orb.as_ref().unwrap().home_url.as_deref(),
            Some("https://example.com/home")
        );
        assert_eq!(
            config.orb.as_ref().unwrap().source_url.as_deref(),
            Some("https://example.com/source")
        );
    }

    // ── gather_extras / dialogue ────────────────────────────────────────────

    fn make_init(dry_run: bool) -> Init {
        Init {
            binary: "mytool".to_string(),
            public_orb_namespaces: vec!["my-org".to_string()],
            private_orb_namespaces: vec![],
            build_workflow: "validation".to_string(),
            release_workflow: "orb-release".to_string(),
            requires_job: None,
            crate_tag_prefix: "mytool-v".to_string(),
            release_after_job: "publish-orb".to_string(),
            orb_dir: "orb".to_string(),
            ci_dir: std::path::PathBuf::from(".circleci"),
            orb_tools_version: "12.3.3".to_string(),
            docker_orb_version: "3.0.1".to_string(),
            docker_namespace: "my-docker-ns".to_string(),
            docker_context: None,
            orb_context: None,
            gen_circleci_orb_version: "0.0.1".to_string(),
            mcp: false,
            mcp_earliest_version: None,
            mcp_context: None,
            dry_run,
            git_push_subcommands: vec![],
            home_url: None,
            source_url: None,
        }
    }

    #[test]
    fn gather_extras_non_interactive_uses_hardcoded_defaults() {
        let init = make_init(true); // dry_run=true → non-interactive
        let extras = init.gather_extras().unwrap();
        assert_eq!(extras.docker_context, DEFAULT_DOCKER_CONTEXT);
        assert_eq!(extras.orb_context, DEFAULT_ORB_CONTEXT);
        assert_eq!(extras.mcp_context, DEFAULT_MCP_CONTEXT);
        assert_eq!(extras.mcp_earliest_version, DEFAULT_MCP_EARLIEST_VERSION);
        assert_eq!(extras.home_url, None);
        assert_eq!(extras.source_url, None);
        assert!(extras.git_push_subcommands.is_empty());
    }

    #[test]
    fn gather_extras_cli_values_take_precedence_over_defaults() {
        let init = Init {
            docker_context: Some("my-docker".to_string()),
            orb_context: Some("my-orb-ctx".to_string()),
            mcp_context: Some("my-mcp-ctx".to_string()),
            mcp_earliest_version: Some("1.2.3".to_string()),
            home_url: Some("https://example.com".to_string()),
            source_url: Some("https://src.example.com".to_string()),
            git_push_subcommands: vec!["save".to_string()],
            dry_run: true,
            ..make_init(true)
        };
        let extras = init.gather_extras().unwrap();
        assert_eq!(extras.docker_context, "my-docker");
        assert_eq!(extras.orb_context, "my-orb-ctx");
        assert_eq!(extras.mcp_context, "my-mcp-ctx");
        assert_eq!(extras.mcp_earliest_version, "1.2.3");
        assert_eq!(extras.home_url.as_deref(), Some("https://example.com"));
        assert_eq!(
            extras.source_url.as_deref(),
            Some("https://src.example.com")
        );
        assert_eq!(extras.git_push_subcommands, vec!["save"]);
    }

    #[test]
    fn gather_extras_ci_env_var_is_non_interactive() {
        // When $CI is set the dialogue must be skipped even without --dry-run
        std::env::set_var("CI", "true");
        let init = make_init(false);
        let extras = init.gather_extras().unwrap();
        std::env::remove_var("CI");
        assert_eq!(extras.docker_context, DEFAULT_DOCKER_CONTEXT);
    }

    #[test]
    fn init_docker_context_field_is_option() {
        // Compile-time guard: field must be Option<String> so we can distinguish
        // "explicitly set" from "not set (will prompt or use default)".
        let init = make_init(true);
        let _: Option<String> = init.docker_context;
    }

    #[test]
    fn bootstrap_config_has_orb_dir() {
        let config = build_bootstrap_config(
            "mytool",
            &["my-org".to_string()],
            "custom-orb",
            None,
            None,
            &[],
        );
        assert_eq!(
            config.orb.as_ref().unwrap().orb_dir.as_deref(),
            Some("custom-orb")
        );
    }
}
