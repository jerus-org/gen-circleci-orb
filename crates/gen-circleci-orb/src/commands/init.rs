use anyhow::Result;
use indexmap::IndexMap;
use std::path::PathBuf;

use crate::{
    ci_patcher,
    commands::generate::Generate,
    help_parser::types::CliDefinition,
    orb_config::{CiSection, OrbConfig, OrbSection, RecordConfig, SubcommandConfig},
};

pub const DEFAULT_DOCKER_ORB_VERSION: &str = "3.0.1";
pub const DEFAULT_DOCKER_CONTEXT: &str = "docker-credentials";
pub const DEFAULT_ORB_CONTEXT: &str = "orb-publishing";
pub const DEFAULT_MCP_CONTEXT: &str = "pcu-app";
pub const DEFAULT_MCP_EARLIEST_VERSION: &str = "0.0.1";
/// Default jerus-org/gen-orb-mcp orb version pinned for the build_mcp_server job
/// (Mechanism A). Generator-owned (like the gen-circleci-orb pin) so the `update`
/// gate stays authoritative; Renovate keeps this default current.
pub const DEFAULT_GEN_ORB_MCP_ORB_VERSION: &str = "0.1.48";

/// The values `init` cannot do anything without: the binary it introspects and
/// the five pieces of pipeline wiring it patches into `.circleci/config.yml`.
///
/// Each is resolved from the CLI flag, then the existing config, then the
/// dialogue — so a re-run never asks for what the config already records, and a
/// first run never has to be told six flags up front (#226).
#[derive(Debug)]
pub(crate) struct GatheredCore {
    pub binary: String,
    pub build_workflow: String,
    pub release_workflow: String,
    pub crate_tag_prefix: String,
    pub release_after_job: String,
    pub docker_namespace: String,
}

/// Values resolved by the interactive dialogue (or non-interactive fallback).
/// These are used by both `PatchOpts` and the bootstrap config.
pub(crate) struct GatheredExtras {
    pub home_url: Option<String>,
    pub source_url: Option<String>,
    pub git_push_subcommands: Vec<String>,
    pub docker_context: String,
    pub orb_context: String,
    pub mcp_context: Vec<String>,
    pub mcp_earliest_version: String,
    pub record: Option<RecordConfig>,
}

/// True when there is nobody to ask: no terminal, or a CI run.
///
/// `--dry-run` is deliberately NOT part of this. A dry run still has to gather
/// the values before it has anything to preview; suppressing the dialogue made
/// `init --dry-run` demand six flags it could simply have asked for (#226).
fn is_non_interactive() -> bool {
    std::env::var("CI").is_ok() || !console::Term::stderr().is_term()
}

/// Assemble the `[record]` config from explicit env-var names. Returns `Ok(None)`
/// when auto-record is not enabled. When enabled, every name must be present and
/// non-empty — there are no defaults, so the tool never imposes an env-var
/// convention on the consumer. Errors naming the first missing flag otherwise.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_record_config(
    enabled: bool,
    gpg_key_env: Option<&str>,
    gpg_trust_env: Option<&str>,
    user_name_env: Option<&str>,
    user_email_env: Option<&str>,
    signing_key_env: Option<&str>,
    push_ssh_fingerprint: Option<&str>,
    contexts: &[String],
) -> Result<Option<RecordConfig>> {
    if !enabled {
        return Ok(None);
    }
    let req = |v: Option<&str>, flag: &str| -> Result<String> {
        v.map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "auto-record is enabled but {flag} was not provided \
                     (no default — supply the env-var name)"
                )
            })
    };
    let contexts: Vec<String> = contexts
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if contexts.is_empty() {
        anyhow::bail!(
            "auto-record is enabled but no --record-context was provided \
             (the record job needs the CircleCI context(s) that supply the GPG \
             signing material)"
        );
    }
    Ok(Some(RecordConfig {
        enabled: true,
        gpg_key_env: req(gpg_key_env, "--record-gpg-key-env")?,
        gpg_trust_env: req(gpg_trust_env, "--record-gpg-trust-env")?,
        user_name_env: req(user_name_env, "--record-user-name-env")?,
        user_email_env: req(user_email_env, "--record-user-email-env")?,
        signing_key_env: req(signing_key_env, "--record-signing-key-env")?,
        // Optional: empty means the push falls back to ambient credentials.
        push_ssh_fingerprint: push_ssh_fingerprint
            .map(str::trim)
            .unwrap_or("")
            .to_string(),
        contexts,
    }))
}

/// Detect leaf subcommands that have a required `orb_path` parameter.
/// These should receive `default = "src/@orb.yml"` in the config so
/// orb consumers don't have to supply the path on every invocation.
pub(crate) fn detect_orb_path_subcommands(cli: &CliDefinition) -> Vec<String> {
    cli.subcommands
        .iter()
        .filter(|sub| {
            sub.is_leaf
                && sub
                    .parameters
                    .iter()
                    .any(|p| p.long_name == "orb_path" && p.required)
        })
        .map(|sub| sub.name.clone())
        .collect()
}

/// Add `[subcommand.<name>.param.orb_path] default = "src/@orb.yml"` for each
/// detected subcommand.  Existing entries (e.g. help suppression) are preserved.
pub(crate) fn populate_orb_path_defaults(
    config: &mut crate::orb_config::OrbConfig,
    subcommands: &[String],
) {
    use crate::orb_config::ParamOverride;
    if subcommands.is_empty() {
        return;
    }
    let sc_map = config
        .subcommand
        .get_or_insert_with(indexmap::IndexMap::new);
    for name in subcommands {
        let sc = sc_map.entry(name.clone()).or_default();
        let params = sc.param.get_or_insert_with(indexmap::IndexMap::new);
        params
            .entry("orb_path".to_string())
            .or_insert(ParamOverride {
                default: Some("src/@orb.yml".to_string()),
            });
    }
}

/// Detect leaf subcommands that are likely to push to git, based on whether
/// they have a `--push`, `--no-push`, or `--sign` parameter.
pub(crate) fn detect_git_push_subcommands(cli: &CliDefinition) -> Vec<String> {
    cli.subcommands
        .iter()
        .filter(|sub| {
            sub.is_leaf
                && sub
                    .parameters
                    .iter()
                    .any(|p| matches!(p.long_name.as_str(), "push" | "no_push" | "sign"))
        })
        .map(|sub| sub.name.clone())
        .collect()
}

/// Wire orb generation into an existing repo's CI configuration.
#[derive(Debug, clap::Args)]
pub struct Init {
    /// Name of the binary to introspect (must be on PATH).
    /// Falls back to `[orb] binary` in the config; prompted for if neither is set.
    #[arg(long)]
    pub binary: Option<String>,

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
    /// Falls back to `[ci] build_workflow` in the config; prompted for if neither is set.
    #[arg(long)]
    pub build_workflow: Option<String>,

    /// Name of the release workflow to patch.
    /// Falls back to `[ci] release_workflow` in the config; prompted for if neither is set.
    #[arg(long)]
    pub release_workflow: Option<String>,

    /// Job in the build workflow that regenerate-orb should require.
    #[arg(long)]
    pub requires_job: Option<String>,

    /// Tag prefix used by `toolkit/release_crate` for the crate (e.g. `gen-orb-mcp-v`).
    /// Used to filter the `orb-release:` workflow trigger in config.yml and to normalise
    /// `CIRCLE_TAG` for `orb-tools/publish`.
    /// Falls back to `[ci] crate_tag_prefix` in the config; prompted for if neither is set.
    #[arg(long)]
    pub crate_tag_prefix: Option<String>,

    /// Job in the release workflow after which the generated release jobs
    /// (build-binary-release, pack-orb-release, build-container, ensure-orb-registered)
    /// should be gated. This is the sole mechanism for specifying where the generated
    /// jobs plug into the existing pipeline topology.
    /// Falls back to `[ci] release_after_job` in the config; prompted for if neither is set.
    #[arg(long)]
    pub release_after_job: Option<String>,

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
    /// Falls back to `[ci] docker_namespace` in the config; prompted for if neither is set.
    #[arg(long)]
    pub docker_namespace: Option<String>,

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

    /// CircleCI context name(s) for MCP server build + publish + save steps (repeatable or comma-separated).
    /// Needs: GITHUB_TOKEN (GitHub App token, contents:write + bypass branch protection),
    /// BOT_GPG_KEY, BOT_TRUST, BOT_USER_NAME, BOT_USER_EMAIL, BOT_SIGN_KEY.
    /// Only used when --mcp is enabled. Prompted interactively if not supplied.
    #[arg(long = "mcp-context", value_delimiter = ',')]
    pub mcp_context: Vec<String>,

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

    /// Enable auto-record: after `generate`, the regenerate-orb CI job commits the
    /// regenerated orb source back (GPG-signed) and pushes it, so the published orb
    /// always reflects the CLI. When set, the `--record-*-env` flags name the
    /// environment variables that hold the GPG signing material at runtime (no
    /// defaults — they must be supplied). Prompted interactively if not set.
    #[arg(long)]
    pub record: bool,

    /// Name of the env var holding the base64-encoded GPG private key (auto-record).
    #[arg(long)]
    pub record_gpg_key_env: Option<String>,

    /// Name of the env var holding the GPG ownertrust export (auto-record).
    #[arg(long)]
    pub record_gpg_trust_env: Option<String>,

    /// Name of the env var holding the committer name (auto-record).
    #[arg(long)]
    pub record_user_name_env: Option<String>,

    /// Name of the env var holding the committer email (auto-record).
    #[arg(long)]
    pub record_user_email_env: Option<String>,

    /// Name of the env var holding the GPG signing key id (auto-record).
    #[arg(long)]
    pub record_signing_key_env: Option<String>,

    /// SSH key fingerprint (a public-key hash, not a secret) for the
    /// end-of-workflow push job (auto-record). Optional: when set, the push job
    /// loads this write key and drops the read-only checkout key; empty falls back
    /// to ambient credentials. A value, not an env-var name — add_ssh_keys resolves
    /// fingerprints at config-compile time and cannot read env vars.
    #[arg(long)]
    pub record_push_ssh_fingerprint: Option<String>,

    /// CircleCI context(s) that supply the auto-record env-var values
    /// (GPG signing material), repeatable or comma-separated.
    /// The record CI job attaches these.
    #[arg(long = "record-context", value_delimiter = ',')]
    pub record_contexts: Vec<String>,

    /// Show planned changes without modifying any files.
    #[arg(long)]
    pub dry_run: bool,
}

/// Subcommands present in the target binary that are interactive by default
/// ([`DEFAULT_INTERACTIVE`]) — the ones `init` prompts about and scaffolds.
pub(crate) fn present_default_interactive(cli: &CliDefinition) -> Vec<String> {
    cli.subcommands
        .iter()
        .filter(|s| crate::orb_generator::render::DEFAULT_INTERACTIVE.contains(&s.name.as_str()))
        .map(|s| s.name.clone())
        .collect()
}

pub(crate) fn build_bootstrap_config(
    binary: &str,
    namespaces: &[String],
    orb_dir: &str,
    home_url: Option<&str>,
    source_url: Option<&str>,
    git_push_subcommands: &[String],
    interactive: &[(String, bool)],
) -> OrbConfig {
    // `help` is reserved at the `--help` parser, so it needs no entry. Interactive
    // (CLI-only) subcommands — `init`/`config` by default, as confirmed at init
    // time — are fully excluded from the orb (job + command + script); a parent
    // (`config`) cascades to its whole subtree, so no per-child entries are needed.
    let mut subcommands = IndexMap::new();
    for (name, is_interactive) in interactive {
        subcommands.insert(
            name.clone(),
            SubcommandConfig {
                interactive: Some(*is_interactive),
                ..SubcommandConfig::default()
            },
        );
    }
    let subcommand = if subcommands.is_empty() {
        None
    } else {
        Some(subcommands)
    };
    OrbConfig {
        orb: Some(OrbSection {
            binary: Some(binary.to_string()),
            namespaces: Some(namespaces.to_vec()),
            orb_dir: Some(orb_dir.to_string()),
            base_image: None,
            builder_image: None,
            circleci_cli_version: None,
            install_method: None,
            apt_packages: None,
            cargo_tools: None,
            home_url: home_url.map(str::to_string),
            source_url: source_url.map(str::to_string),
            git_push_subcommands: if git_push_subcommands.is_empty() {
                None
            } else {
                Some(git_push_subcommands.to_vec())
            },
            custom_files: None,
        }),
        ci: None, // populated by run() after gathering extras
        orbs: None,
        subcommand,
        job_group: None,
        extra_job: None,
        record: None, // populated by run() after gathering extras
    }
}

impl Init {
    /// Gather the `[record]` config. Name resolution: CLI flag > existing config.
    /// Non-interactive mode assembles from those sources (erroring if enabled but a
    /// name is missing); interactive mode confirms the need then prompts for each
    /// env-var name (no defaults beyond the user's own prior config).
    fn gather_record(
        &self,
        existing: &OrbConfig,
        interactive: bool,
    ) -> Result<Option<RecordConfig>> {
        let ex = existing.record.as_ref();
        let resolve = |cli: Option<&String>, prev: Option<&str>| -> Option<String> {
            cli.filter(|s| !s.is_empty())
                .cloned()
                .or_else(|| prev.map(str::to_string))
        };
        let gpg_key = resolve(
            self.record_gpg_key_env.as_ref(),
            ex.map(|r| r.gpg_key_env.as_str()),
        );
        let gpg_trust = resolve(
            self.record_gpg_trust_env.as_ref(),
            ex.map(|r| r.gpg_trust_env.as_str()),
        );
        let user_name = resolve(
            self.record_user_name_env.as_ref(),
            ex.map(|r| r.user_name_env.as_str()),
        );
        let user_email = resolve(
            self.record_user_email_env.as_ref(),
            ex.map(|r| r.user_email_env.as_str()),
        );
        let sign_key = resolve(
            self.record_signing_key_env.as_ref(),
            ex.map(|r| r.signing_key_env.as_str()),
        );
        let push_fingerprint = resolve(
            self.record_push_ssh_fingerprint.as_ref(),
            ex.map(|r| r.push_ssh_fingerprint.as_str()),
        );
        let contexts: Vec<String> = if !self.record_contexts.is_empty() {
            self.record_contexts.clone()
        } else {
            ex.map(|r| r.contexts.clone()).unwrap_or_default()
        };

        if !interactive {
            let enabled = self.record || ex.map(|r| r.enabled).unwrap_or(false);
            return build_record_config(
                enabled,
                gpg_key.as_deref(),
                gpg_trust.as_deref(),
                user_name.as_deref(),
                user_email.as_deref(),
                sign_key.as_deref(),
                push_fingerprint.as_deref(),
                &contexts,
            );
        }

        let enabled = if self.record {
            true
        } else {
            dialoguer::Confirm::new()
                .with_prompt("Enable auto-record (CI signs + pushes the regenerated orb)?")
                .default(ex.map(|r| r.enabled).unwrap_or(false))
                .interact()?
        };
        if !enabled {
            return Ok(None);
        }

        use dialoguer::Input;
        let prompt_name = |label: &str, current: Option<String>| -> Result<String> {
            let mut input = Input::<String>::new().with_prompt(label);
            if let Some(c) = current.filter(|s| !s.is_empty()) {
                input = input.default(c);
            }
            Ok(input.interact_text()?)
        };
        let gpg_key = prompt_name("Env var name — base64 GPG private key", gpg_key)?;
        let gpg_trust = prompt_name("Env var name — GPG ownertrust export", gpg_trust)?;
        let user_name = prompt_name("Env var name — committer name", user_name)?;
        let user_email = prompt_name("Env var name — committer email", user_email)?;
        let sign_key = prompt_name("Env var name — GPG signing key id", sign_key)?;
        // Optional: a fingerprint VALUE (public-key hash), or empty for ambient.
        let push_fingerprint = prompt_name(
            "SSH key fingerprint for the push job (optional; empty = ambient credentials)",
            push_fingerprint,
        )?;
        let contexts_default = if contexts.is_empty() {
            None
        } else {
            Some(contexts.join(","))
        };
        let contexts_raw = prompt_name(
            "CircleCI context(s) supplying the GPG signing material, comma-separated",
            contexts_default,
        )?;
        let contexts: Vec<String> = contexts_raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        build_record_config(
            true,
            Some(&gpg_key),
            Some(&gpg_trust),
            Some(&user_name),
            Some(&user_email),
            Some(&sign_key),
            Some(&push_fingerprint),
            &contexts,
        )
    }

    /// Resolve the six values `init` cannot proceed without.
    ///
    /// Interactive: prompt for each one the CLI and config did not supply,
    /// offering the config value (or a sensible guess) as the default.
    /// Non-interactive: fail naming **every** value still missing, so one run
    /// tells the caller the whole set rather than one flag at a time.
    pub(crate) fn gather_core(
        &self,
        existing: &OrbConfig,
        interactive: bool,
    ) -> Result<GatheredCore> {
        let ci = existing.ci.as_ref();
        let orb = existing.orb.as_ref();

        let binary = self
            .binary
            .clone()
            .or_else(|| orb.and_then(|o| o.binary.clone()));
        let build_workflow = self
            .build_workflow
            .clone()
            .or_else(|| ci.and_then(|c| c.build_workflow.clone()));
        let release_workflow = self
            .release_workflow
            .clone()
            .or_else(|| ci.and_then(|c| c.release_workflow.clone()));
        let crate_tag_prefix = self
            .crate_tag_prefix
            .clone()
            .or_else(|| ci.and_then(|c| c.crate_tag_prefix.clone()));
        let release_after_job = self
            .release_after_job
            .clone()
            .or_else(|| ci.and_then(|c| c.release_after_job.clone()));
        let docker_namespace = self
            .docker_namespace
            .clone()
            .or_else(|| ci.and_then(|c| c.docker_namespace.clone()));

        if !interactive {
            let missing: Vec<&str> = [
                (binary.is_none(), "--binary"),
                (build_workflow.is_none(), "--build-workflow"),
                (release_workflow.is_none(), "--release-workflow"),
                (crate_tag_prefix.is_none(), "--crate-tag-prefix"),
                (release_after_job.is_none(), "--release-after-job"),
                (docker_namespace.is_none(), "--docker-namespace"),
            ]
            .into_iter()
            .filter_map(|(missing, flag)| missing.then_some(flag))
            .collect();
            if !missing.is_empty() {
                anyhow::bail!(
                    "init needs these values and there is no terminal to ask \
                     (running under CI, or output redirected): {}. \
                     Supply them as flags, or record them in gen-circleci-orb.toml.",
                    missing.join(", ")
                );
            }
            return Ok(GatheredCore {
                binary: binary.unwrap_or_default(),
                build_workflow: build_workflow.unwrap_or_default(),
                release_workflow: release_workflow.unwrap_or_default(),
                crate_tag_prefix: crate_tag_prefix.unwrap_or_default(),
                release_after_job: release_after_job.unwrap_or_default(),
                docker_namespace: docker_namespace.unwrap_or_default(),
            });
        }

        use dialoguer::Input;
        let ask = |label: &str, current: Option<String>| -> Result<String> {
            let mut input = Input::<String>::new().with_prompt(label);
            if let Some(default) = current.filter(|s| !s.is_empty()) {
                input = input.default(default);
            }
            Ok(input.interact_text()?.trim().to_string())
        };
        let resolve = |label: &str, value: Option<String>| -> Result<String> {
            match value.filter(|s| !s.is_empty()) {
                Some(v) => Ok(v),
                None => ask(label, None),
            }
        };

        let binary = resolve(
            "Binary to introspect (must be on PATH — its --help drives every generated job)",
            binary,
        )?;
        let build_workflow = resolve(
            "Build/validation workflow to patch (a workflow name in .circleci/config.yml)",
            build_workflow,
        )?;
        let release_workflow = resolve(
            "Release workflow to patch (where the orb is published)",
            release_workflow,
        )?;
        let crate_tag_prefix = match crate_tag_prefix.filter(|s| !s.is_empty()) {
            Some(v) => v,
            // The convention is `<crate>-v`, and the binary name is the crate
            // name in every workspace this targets — so offer it, don't impose it.
            None => ask(
                "Crate tag prefix used by the release (e.g. my-crate-v)",
                Some(format!("{binary}-v")),
            )?,
        };
        let release_after_job = resolve(
            "Job in the release workflow the generated release jobs should follow",
            release_after_job,
        )?;
        let docker_namespace = resolve(
            "Docker registry namespace for the orb's executor image (e.g. myorg)",
            docker_namespace,
        )?;

        Ok(GatheredCore {
            binary,
            build_workflow,
            release_workflow,
            crate_tag_prefix,
            release_after_job,
            docker_namespace,
        })
    }

    pub(crate) fn gather_extras(
        &self,
        detected: &[String],
        existing: &OrbConfig,
        interactive: bool,
    ) -> Result<GatheredExtras> {
        // Resolution order: CLI flag > existing config > auto-detected / hardcoded default.
        let existing_ci = existing.ci.as_ref();
        let existing_orb = existing.orb.as_ref();
        let record = self.gather_record(existing, interactive)?;

        let effective_push = if !self.git_push_subcommands.is_empty() {
            self.git_push_subcommands.clone()
        } else {
            existing_orb
                .and_then(|o| o.git_push_subcommands.clone())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| detected.to_vec())
        };

        if !interactive {
            return Ok(GatheredExtras {
                home_url: self
                    .home_url
                    .clone()
                    .or_else(|| existing_orb.and_then(|o| o.home_url.clone())),
                source_url: self
                    .source_url
                    .clone()
                    .or_else(|| existing_orb.and_then(|o| o.source_url.clone())),
                git_push_subcommands: effective_push,
                docker_context: self
                    .docker_context
                    .clone()
                    .or_else(|| existing_ci.and_then(|ci| ci.docker_context.clone()))
                    .unwrap_or_else(|| DEFAULT_DOCKER_CONTEXT.to_string()),
                orb_context: self
                    .orb_context
                    .clone()
                    .or_else(|| existing_ci.and_then(|ci| ci.orb_context.clone()))
                    .unwrap_or_else(|| DEFAULT_ORB_CONTEXT.to_string()),
                mcp_context: if !self.mcp_context.is_empty() {
                    self.mcp_context.clone()
                } else {
                    existing_ci
                        .and_then(|ci| ci.mcp_context.clone())
                        .filter(|v| !v.is_empty())
                        .unwrap_or_else(|| vec![DEFAULT_MCP_CONTEXT.to_string()])
                },
                mcp_earliest_version: self
                    .mcp_earliest_version
                    .clone()
                    .or_else(|| existing_ci.and_then(|ci| ci.mcp_earliest_version.clone()))
                    .unwrap_or_else(|| DEFAULT_MCP_EARLIEST_VERSION.to_string()),
                record,
            });
        }

        // Interactive mode — prompt only for fields not already provided via CLI flag.
        // For un-set fields, the existing config value becomes the prompt default.
        use dialoguer::Input;

        let home_url = if let Some(v) = self.home_url.clone() {
            Some(v).filter(|s| !s.is_empty())
        } else {
            let default = existing_orb
                .and_then(|o| o.home_url.clone())
                .unwrap_or_default();
            let val = Input::<String>::new()
                .with_prompt("Home URL for orb registry (Enter to skip)")
                .default(default)
                .allow_empty(true)
                .interact_text()?;
            if val.is_empty() {
                None
            } else {
                Some(val)
            }
        };

        let source_url = if let Some(v) = self.source_url.clone() {
            Some(v).filter(|s| !s.is_empty())
        } else {
            let default = existing_orb
                .and_then(|o| o.source_url.clone())
                .unwrap_or_default();
            let val = Input::<String>::new()
                .with_prompt("Source URL for orb registry (Enter to skip)")
                .default(default)
                .allow_empty(true)
                .interact_text()?;
            if val.is_empty() {
                None
            } else {
                Some(val)
            }
        };

        let git_push_subcommands = if !self.git_push_subcommands.is_empty() {
            effective_push
        } else {
            let cfg_push = existing_orb
                .and_then(|o| o.git_push_subcommands.clone())
                .unwrap_or_default();
            let current = if !cfg_push.is_empty() {
                cfg_push.join(",")
            } else {
                detected.join(",")
            };
            let prompt = if !detected.is_empty() && cfg_push.is_empty() {
                format!(
                    "Push-capable subcommands detected: {} — confirm or override (comma-separated)",
                    detected.join(", ")
                )
            } else {
                "Subcommands that push to git, comma-separated (e.g. save)".to_string()
            };
            let val = Input::<String>::new()
                .with_prompt(prompt)
                .default(current)
                .allow_empty(true)
                .interact_text()?;
            val.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        };

        let docker_context = if let Some(v) = self.docker_context.clone() {
            v
        } else {
            let default = existing_ci
                .and_then(|ci| ci.docker_context.clone())
                .unwrap_or_else(|| DEFAULT_DOCKER_CONTEXT.to_string());
            Input::<String>::new()
                .with_prompt("Docker context name (needs: DOCKER_LOGIN, DOCKER_PASSWORD)")
                .default(default)
                .interact_text()?
        };

        let orb_context = if let Some(v) = self.orb_context.clone() {
            v
        } else {
            let default = existing_ci
                .and_then(|ci| ci.orb_context.clone())
                .unwrap_or_else(|| DEFAULT_ORB_CONTEXT.to_string());
            Input::<String>::new()
                .with_prompt("Orb publishing context name (needs: CIRCLECI_CLI_TOKEN)")
                .default(default)
                .interact_text()?
        };

        let mcp_context = if self.mcp {
            if !self.mcp_context.is_empty() {
                self.mcp_context.clone()
            } else {
                let default = existing_ci
                    .and_then(|ci| ci.mcp_context.as_ref())
                    .filter(|v| !v.is_empty())
                    .map(|v| v.join(","))
                    .unwrap_or_else(|| DEFAULT_MCP_CONTEXT.to_string());
                let val = Input::<String>::new()
                    .with_prompt(
                        "MCP context names, comma-separated (needs: GITHUB_TOKEN with contents:write + bypass branch protection, BOT_GPG_KEY, BOT_TRUST, BOT_USER_NAME, BOT_USER_EMAIL, BOT_SIGN_KEY)",
                    )
                    .default(default)
                    .interact_text()?;
                val.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            }
        } else if !self.mcp_context.is_empty() {
            self.mcp_context.clone()
        } else {
            existing_ci
                .and_then(|ci| ci.mcp_context.clone())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| vec![DEFAULT_MCP_CONTEXT.to_string()])
        };

        let mcp_earliest_version = if self.mcp {
            if let Some(v) = self.mcp_earliest_version.clone() {
                v
            } else {
                let default = existing_ci
                    .and_then(|ci| ci.mcp_earliest_version.clone())
                    .unwrap_or_else(|| DEFAULT_MCP_EARLIEST_VERSION.to_string());
                Input::<String>::new()
                    .with_prompt("Earliest orb version to include in MCP snapshots")
                    .default(default)
                    .interact_text()?
            }
        } else {
            self.mcp_earliest_version
                .clone()
                .or_else(|| existing_ci.and_then(|ci| ci.mcp_earliest_version.clone()))
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
            record,
        })
    }

    /// Decide which of the present default-interactive subcommands to reserve as
    /// interactive/CLI-only (fully excluded from the orb). Interactive terminal:
    /// prompt per subcommand, defaulting to reserved so the user confirms/overrides
    /// in the initial scaffold. Non-interactive (CI/dry-run): reserve all (the safe
    /// default), so the scaffold still records the choice explicitly.
    fn resolve_interactive(
        &self,
        present: &[String],
        interactive: bool,
    ) -> Result<Vec<(String, bool)>> {
        if !interactive {
            return Ok(present.iter().map(|n| (n.clone(), true)).collect());
        }
        present
            .iter()
            .map(|name| -> Result<(String, bool)> {
                let reserve = dialoguer::Confirm::new()
                    .with_prompt(format!(
                        "Reserve `{name}` as interactive-only (CLI setup — excluded from the CI orb)?"
                    ))
                    .default(true)
                    .interact()?;
                Ok((name.clone(), reserve))
            })
            .collect()
    }

    pub fn run(&self) -> Result<()> {
        let interactive = !is_non_interactive();

        // The binary is itself one of the gathered values, so the config is read
        // and the dialogue run before there is anything to introspect.
        let config_path = std::path::Path::new("gen-circleci-orb.toml");
        let existing_config = crate::orb_config::load_config(config_path)?;
        let core = self.gather_core(&existing_config, interactive)?;

        // Parse binary early: detect push-capable subcommands (for dialogue default)
        // and subcommands with a required orb_path param (for config defaults).
        let (detected_push, detected_orb_path, present_interactive) =
            match crate::help_parser::parse_binary(&core.binary) {
                Ok(cli) => (
                    detect_git_push_subcommands(&cli),
                    detect_orb_path_subcommands(&cli),
                    present_default_interactive(&cli),
                ),
                Err(_) => (vec![], vec![], vec![]),
            };

        let extras = self.gather_extras(&detected_push, &existing_config, interactive)?;
        let namespaces: Vec<String> = self
            .public_orb_namespaces
            .iter()
            .chain(self.private_orb_namespaces.iter())
            .cloned()
            .collect();

        // Step 1: generate orb source files
        tracing::info!("Generating orb source into ./{}", self.orb_dir);
        let gen = Generate {
            binary: Some(core.binary.clone()),
            namespaces: namespaces.clone(),
            output: PathBuf::from("."),
            orb_dir: Some(self.orb_dir.clone()),
            install_method: None,
            base_image: None,
            home_url: extras.home_url.clone(),
            source_url: extras.source_url.clone(),
            git_push_subcommands: extras.git_push_subcommands.clone(),
            circleci_cli_version: None,
            apt_packages: vec![],
            cargo_tools: vec![],
            dry_run: self.dry_run,
            config: None,
            // init is a local bootstrap, not a CI run — never auto-record/push.
            no_record: true,
            // init writes the orb for real; verify-only check mode is off.
            check: false,
        };
        gen.run()?;

        // Step 2: patch CI configs
        let opts = ci_patcher::PatchOpts {
            binary: core.binary.clone(),
            // Advanced knob — not gathered at init; set `[orb] rust_image` in the
            // toml when the workspace needs a clang-equipped build image.
            rust_image: String::new(),
            namespaces,
            docker_namespace: core.docker_namespace.clone(),
            orb_dir: self.orb_dir.clone(),
            build_workflow: core.build_workflow.clone(),
            release_workflow: core.release_workflow.clone(),
            requires_job: self.requires_job.clone(),
            crate_tag_prefix: core.crate_tag_prefix.clone(),
            release_after_job: core.release_after_job.clone(),
            orb_tools_version: self.orb_tools_version.clone(),
            docker_orb_version: self.docker_orb_version.clone(),
            docker_context: extras.docker_context.clone(),
            orb_context: extras.orb_context.clone(),
            private_namespaces: self.private_orb_namespaces.clone(),
            gen_circleci_orb_version: self.gen_circleci_orb_version.clone(),
            mcp: self.mcp,
            mcp_earliest_version: extras.mcp_earliest_version.clone(),
            mcp_context: extras.mcp_context.clone(),
            gen_orb_mcp_orb_version: DEFAULT_GEN_ORB_MCP_ORB_VERSION.to_string(),
            record_contexts: extras
                .record
                .as_ref()
                .map(|r| r.contexts.clone())
                .unwrap_or_default(),
            record_push_ssh_fingerprint: extras
                .record
                .as_ref()
                .map(|r| r.push_ssh_fingerprint.clone())
                .unwrap_or_default(),
        };

        let summary = ci_patcher::apply_patches(&self.ci_dir, &opts, self.dry_run)?;
        for line in &summary {
            println!("{line}");
        }

        // Step 3: write bootstrap gen-circleci-orb.toml
        let config_path = std::path::Path::new("gen-circleci-orb.toml");
        let reserved = self.resolve_interactive(&present_interactive, interactive)?;
        let mut bootstrap = build_bootstrap_config(
            &core.binary,
            opts.namespaces.as_slice(),
            &self.orb_dir,
            extras.home_url.as_deref(),
            extras.source_url.as_deref(),
            &extras.git_push_subcommands,
            &reserved,
        );
        populate_orb_path_defaults(&mut bootstrap, &detected_orb_path);
        bootstrap.ci = Some(CiSection {
            build_workflow: Some(core.build_workflow.clone()),
            release_workflow: Some(core.release_workflow.clone()),
            requires_job: self.requires_job.clone(),
            release_after_job: Some(core.release_after_job.clone()),
            crate_tag_prefix: Some(core.crate_tag_prefix.clone()),
            docker_namespace: Some(core.docker_namespace.clone()),
            docker_context: Some(extras.docker_context.clone()),
            orb_context: Some(extras.orb_context.clone()),
            mcp: Some(self.mcp),
            mcp_context: Some(extras.mcp_context.clone()),
            mcp_earliest_version: Some(extras.mcp_earliest_version.clone()),
            // Left unset so the pin tracks the generator default (like the
            // gen-circleci-orb pin); set it in the toml only to override.
            gen_orb_mcp_orb_version: None,
            // Advanced knob — not gathered at init; set `[ci] rust_image` in the
            // toml when the workspace needs a clang-equipped build image.
            rust_image: None,
        });
        bootstrap.record = extras.record.clone();
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
        let config = build_bootstrap_config(
            "mytool",
            &["my-org".to_string()],
            "orb",
            None,
            None,
            &[],
            &[],
        );
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
            &[],
        );
        assert_eq!(
            config.orb.as_ref().unwrap().namespaces.as_deref(),
            Some(&["ns1".to_string(), "ns2".to_string()][..])
        );
    }

    #[test]
    fn bootstrap_config_scaffolds_interactive_decisions() {
        // The bootstrap scaffolds the init-time interactive decisions explicitly
        // (init reserved, config opted in) and does NOT emit a help entry — help is
        // reserved at the --help parser.
        let config = build_bootstrap_config(
            "mytool",
            &["my-org".to_string()],
            "orb",
            None,
            None,
            &[],
            &[("init".to_string(), true), ("config".to_string(), false)],
        );
        let subcommands = config
            .subcommand
            .as_ref()
            .expect("subcommand section missing");
        assert_eq!(subcommands.get("init").unwrap().interactive, Some(true));
        assert_eq!(subcommands.get("config").unwrap().interactive, Some(false));
        assert!(
            !subcommands.contains_key("help"),
            "help is parser-reserved, not scaffolded into the toml"
        );
    }

    #[test]
    fn bootstrap_config_has_no_subcommand_section_without_interactive() {
        let config = build_bootstrap_config(
            "mytool",
            &["my-org".to_string()],
            "orb",
            None,
            None,
            &[],
            &[],
        );
        assert!(
            config.subcommand.is_none(),
            "no interactive decisions → no [subcommand] section"
        );
    }

    #[test]
    fn present_default_interactive_returns_only_present_defaults() {
        use crate::help_parser::types::{CliDefinition, SubCommand};
        let sub = |name: &str| SubCommand {
            name: name.to_string(),
            description: String::new(),
            is_leaf: true,
            parameters: vec![],
            subcommands: vec![],
        };
        // `init` is a default-interactive name present here; `run` is not; `config`
        // is absent → only `init` is returned (the prompt fires only for present names).
        let cli = CliDefinition {
            binary_name: "mytool".to_string(),
            description: String::new(),
            subcommands: vec![sub("init"), sub("run")],
        };
        assert_eq!(present_default_interactive(&cli), vec!["init".to_string()]);
    }

    #[test]
    fn init_has_git_push_subcommands_field() {
        // Init must expose --git-push-subcommands so the caller can name subcommands
        // (e.g. "save") that need a set_https_remote step in their generated job.
        let init = Init {
            binary: Some("mytool".to_string()),
            public_orb_namespaces: vec!["my-org".to_string()],
            private_orb_namespaces: vec![],
            build_workflow: Some("validation".to_string()),
            release_workflow: Some("orb-release".to_string()),
            requires_job: None,
            crate_tag_prefix: Some("mytool-v".to_string()),
            release_after_job: Some("publish-orb".to_string()),
            orb_dir: "orb".to_string(),
            ci_dir: std::path::PathBuf::from(".circleci"),
            orb_tools_version: "12.3.3".to_string(),
            docker_orb_version: "3.0.1".to_string(),
            docker_namespace: Some("my-docker-ns".to_string()),
            docker_context: None,
            orb_context: None,
            gen_circleci_orb_version: "0.0.1".to_string(),
            mcp: false,
            mcp_earliest_version: None,
            mcp_context: vec![],
            dry_run: false,
            git_push_subcommands: vec!["save".to_string()],
            home_url: None,
            source_url: None,
            record: false,
            record_gpg_key_env: None,
            record_gpg_trust_env: None,
            record_user_name_env: None,
            record_user_email_env: None,
            record_signing_key_env: None,
            record_push_ssh_fingerprint: None,
            record_contexts: vec![],
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
            &[],
        );
        assert_eq!(
            config.orb.as_ref().unwrap().git_push_subcommands.as_deref(),
            Some(&["save".to_string()][..])
        );
    }

    #[test]
    fn bootstrap_config_git_push_subcommands_none_when_empty() {
        let config = build_bootstrap_config(
            "mytool",
            &["my-org".to_string()],
            "orb",
            None,
            None,
            &[],
            &[],
        );
        assert_eq!(
            config.orb.as_ref().unwrap().git_push_subcommands,
            None,
            "empty slice must produce None (not an empty list) to keep the TOML clean"
        );
    }

    #[test]
    fn init_run_writes_ci_section_to_config() {
        let init = make_init(true);
        let extras = init
            .gather_extras(&[], &OrbConfig::default(), false)
            .unwrap();
        let core = init.gather_core(&OrbConfig::default(), false).unwrap();
        let ci = CiSection {
            build_workflow: Some(core.build_workflow.clone()),
            release_workflow: Some(core.release_workflow.clone()),
            requires_job: init.requires_job.clone(),
            release_after_job: Some(core.release_after_job.clone()),
            crate_tag_prefix: Some(core.crate_tag_prefix.clone()),
            docker_namespace: Some(core.docker_namespace.clone()),
            docker_context: Some(extras.docker_context.clone()),
            orb_context: Some(extras.orb_context.clone()),
            mcp: Some(init.mcp),
            mcp_context: Some(extras.mcp_context.clone()),
            mcp_earliest_version: Some(extras.mcp_earliest_version.clone()),
            gen_orb_mcp_orb_version: None,
            rust_image: None,
        };
        assert_eq!(ci.build_workflow.as_deref(), Some("validation"));
        assert_eq!(ci.docker_context.as_deref(), Some(DEFAULT_DOCKER_CONTEXT));
        assert_eq!(ci.mcp, Some(false));
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

    // ── detect_orb_path_subcommands + populate_orb_path_defaults ───────────

    fn make_cli_with_orb_path(
        sub_name: &str,
        required: bool,
    ) -> crate::help_parser::types::CliDefinition {
        use crate::help_parser::types::{CliDefinition, ParamType, Parameter, SubCommand};
        let p = Parameter {
            long_name: "orb_path".to_string(),
            short: Some('p'),
            param_type: ParamType::String,
            default: None,
            required,
            description: "Path to orb YAML".to_string(),
        };
        let sub = SubCommand {
            name: sub_name.to_string(),
            description: String::new(),
            is_leaf: true,
            parameters: vec![p],
            subcommands: vec![],
        };
        CliDefinition {
            binary_name: "mytool".to_string(),
            description: "My tool".to_string(),
            subcommands: vec![sub],
        }
    }

    #[test]
    fn detect_required_orb_path_subcommand() {
        let cli = make_cli_with_orb_path("generate", true);
        let detected = detect_orb_path_subcommands(&cli);
        assert_eq!(detected, vec!["generate".to_string()]);
    }

    #[test]
    fn optional_orb_path_not_detected() {
        let cli = make_cli_with_orb_path("generate", false);
        let detected = detect_orb_path_subcommands(&cli);
        assert!(
            detected.is_empty(),
            "optional orb_path must not trigger default injection"
        );
    }

    #[test]
    fn populate_orb_path_defaults_adds_subcommand_entries() {
        let mut config = build_bootstrap_config(
            "mytool",
            &["my-org".to_string()],
            "orb",
            None,
            None,
            &[],
            &[],
        );
        populate_orb_path_defaults(
            &mut config,
            &["generate".to_string(), "validate".to_string()],
        );
        let subcommands = config.subcommand.as_ref().unwrap();
        let gen_params = subcommands.get("generate").unwrap().param.as_ref().unwrap();
        assert_eq!(
            gen_params.get("orb_path").unwrap().default.as_deref(),
            Some("src/@orb.yml")
        );
        let val_params = subcommands.get("validate").unwrap().param.as_ref().unwrap();
        assert_eq!(
            val_params.get("orb_path").unwrap().default.as_deref(),
            Some("src/@orb.yml")
        );
    }

    #[test]
    fn populate_orb_path_defaults_preserves_existing_subcommand_entries() {
        // An existing interactive entry (init reserved) must not be disturbed when
        // populate adds orb_path param defaults for another subcommand.
        let mut config = build_bootstrap_config(
            "mytool",
            &["my-org".to_string()],
            "orb",
            None,
            None,
            &[],
            &[("init".to_string(), true)],
        );
        populate_orb_path_defaults(&mut config, &["generate".to_string()]);
        let subcommands = config.subcommand.as_ref().unwrap();
        assert_eq!(subcommands.get("init").unwrap().interactive, Some(true));
    }

    #[test]
    fn populate_orb_path_defaults_noop_when_empty() {
        let mut config = build_bootstrap_config(
            "mytool",
            &["my-org".to_string()],
            "orb",
            None,
            None,
            &[],
            &[],
        );
        let before = config.subcommand.clone();
        populate_orb_path_defaults(&mut config, &[]);
        assert_eq!(
            config.subcommand, before,
            "no change when no subcommands detected"
        );
    }

    // ── detect_git_push_subcommands ─────────────────────────────────────────

    #[test]
    fn detect_push_subcommand_with_push_param() {
        use crate::help_parser::types::{CliDefinition, ParamType, Parameter, SubCommand};
        let push_param = Parameter {
            long_name: "push".to_string(),
            short: None,
            param_type: ParamType::Enum(vec!["true".to_string(), "false".to_string()]),
            default: Some("true".to_string()),
            required: false,
            description: "Push after committing".to_string(),
        };
        let sub = SubCommand {
            name: "save".to_string(),
            description: "Save artifacts".to_string(),
            is_leaf: true,
            parameters: vec![push_param],
            subcommands: vec![],
        };
        let cli = CliDefinition {
            binary_name: "mytool".to_string(),
            description: "My tool".to_string(),
            subcommands: vec![sub],
        };
        let detected = detect_git_push_subcommands(&cli);
        assert_eq!(detected, vec!["save".to_string()]);
    }

    #[test]
    fn detect_push_subcommand_with_sign_param() {
        use crate::help_parser::types::{CliDefinition, ParamType, Parameter, SubCommand};
        let sign_param = Parameter {
            long_name: "sign".to_string(),
            short: None,
            param_type: ParamType::Boolean,
            default: None,
            required: false,
            description: "GPG sign".to_string(),
        };
        let sub = SubCommand {
            name: "commit".to_string(),
            description: "Commit".to_string(),
            is_leaf: true,
            parameters: vec![sign_param],
            subcommands: vec![],
        };
        let cli = CliDefinition {
            binary_name: "mytool".to_string(),
            description: "My tool".to_string(),
            subcommands: vec![sub],
        };
        let detected = detect_git_push_subcommands(&cli);
        assert_eq!(detected, vec!["commit".to_string()]);
    }

    #[test]
    fn non_push_subcommand_not_detected() {
        use crate::help_parser::types::{CliDefinition, ParamType, Parameter, SubCommand};
        let other_param = Parameter {
            long_name: "output".to_string(),
            short: None,
            param_type: ParamType::String,
            default: Some("./dist".to_string()),
            required: false,
            description: "Output dir".to_string(),
        };
        let sub = SubCommand {
            name: "generate".to_string(),
            description: "Generate".to_string(),
            is_leaf: true,
            parameters: vec![other_param],
            subcommands: vec![],
        };
        let cli = CliDefinition {
            binary_name: "mytool".to_string(),
            description: "My tool".to_string(),
            subcommands: vec![sub],
        };
        let detected = detect_git_push_subcommands(&cli);
        assert!(detected.is_empty());
    }

    #[test]
    fn gather_extras_uses_detected_when_cli_empty() {
        let init = make_init(true); // dry_run = true → non-interactive
        let extras = init
            .gather_extras(&["save".to_string()], &OrbConfig::default(), false)
            .unwrap();
        assert_eq!(
            extras.git_push_subcommands,
            vec!["save".to_string()],
            "detected candidates must be used when --git-push-subcommands not set"
        );
    }

    #[test]
    fn gather_extras_cli_overrides_detected() {
        let init = Init {
            git_push_subcommands: vec!["custom".to_string()],
            dry_run: true,
            ..make_init(true)
        };
        let extras = init
            .gather_extras(&["save".to_string()], &OrbConfig::default(), false)
            .unwrap();
        assert_eq!(
            extras.git_push_subcommands,
            vec!["custom".to_string()],
            "explicit CLI value must override detected candidates"
        );
    }

    // ── gather_extras / dialogue ────────────────────────────────────────────

    fn make_init(dry_run: bool) -> Init {
        Init {
            binary: Some("mytool".to_string()),
            public_orb_namespaces: vec!["my-org".to_string()],
            private_orb_namespaces: vec![],
            build_workflow: Some("validation".to_string()),
            release_workflow: Some("orb-release".to_string()),
            requires_job: None,
            crate_tag_prefix: Some("mytool-v".to_string()),
            release_after_job: Some("publish-orb".to_string()),
            orb_dir: "orb".to_string(),
            ci_dir: std::path::PathBuf::from(".circleci"),
            orb_tools_version: "12.3.3".to_string(),
            docker_orb_version: "3.0.1".to_string(),
            docker_namespace: Some("my-docker-ns".to_string()),
            docker_context: None,
            orb_context: None,
            gen_circleci_orb_version: "0.0.1".to_string(),
            mcp: false,
            mcp_earliest_version: None,
            mcp_context: vec![],
            dry_run,
            git_push_subcommands: vec![],
            home_url: None,
            source_url: None,
            record: false,
            record_gpg_key_env: None,
            record_gpg_trust_env: None,
            record_user_name_env: None,
            record_user_email_env: None,
            record_signing_key_env: None,
            record_push_ssh_fingerprint: None,
            record_contexts: vec![],
        }
    }

    // ── gather_core: the values init cannot run without (#226) ─────────────

    #[test]
    fn gather_core_uses_cli_values() {
        let init = make_init(false);
        let core = init.gather_core(&OrbConfig::default(), false).unwrap();
        assert_eq!(core.binary, "mytool");
        assert_eq!(core.build_workflow, "validation");
        assert_eq!(core.release_workflow, "orb-release");
        assert_eq!(core.crate_tag_prefix, "mytool-v");
        assert_eq!(core.release_after_job, "publish-orb");
        assert_eq!(core.docker_namespace, "my-docker-ns");
    }

    /// Re-running `init` must not mean re-typing what the config already holds.
    #[test]
    fn gather_core_falls_back_to_existing_config() {
        let existing = OrbConfig {
            orb: Some(OrbSection {
                binary: Some("configured-tool".to_string()),
                ..OrbSection::default()
            }),
            ci: Some(CiSection {
                build_workflow: Some("cfg-build".to_string()),
                release_workflow: Some("cfg-release".to_string()),
                crate_tag_prefix: Some("cfg-v".to_string()),
                release_after_job: Some("cfg-after".to_string()),
                docker_namespace: Some("cfg-ns".to_string()),
                ..CiSection::default()
            }),
            ..OrbConfig::default()
        };
        let init = Init {
            binary: None,
            build_workflow: None,
            release_workflow: None,
            crate_tag_prefix: None,
            release_after_job: None,
            docker_namespace: None,
            ..make_init(false)
        };
        let core = init.gather_core(&existing, false).unwrap();
        assert_eq!(core.binary, "configured-tool");
        assert_eq!(core.build_workflow, "cfg-build");
        assert_eq!(core.release_workflow, "cfg-release");
        assert_eq!(core.crate_tag_prefix, "cfg-v");
        assert_eq!(core.release_after_job, "cfg-after");
        assert_eq!(core.docker_namespace, "cfg-ns");
    }

    #[test]
    fn gather_core_cli_wins_over_config() {
        let existing = OrbConfig {
            orb: Some(OrbSection {
                binary: Some("configured-tool".to_string()),
                ..OrbSection::default()
            }),
            ..OrbConfig::default()
        };
        let core = make_init(false).gather_core(&existing, false).unwrap();
        assert_eq!(core.binary, "mytool");
    }

    /// With nothing to prompt with, the error names EVERY missing value — not
    /// clap's usage dump, and not just the first one.
    #[test]
    fn gather_core_non_interactive_lists_every_missing_value() {
        let init = Init {
            binary: None,
            build_workflow: None,
            release_workflow: None,
            crate_tag_prefix: None,
            release_after_job: None,
            docker_namespace: None,
            ..make_init(false)
        };
        let err = init
            .gather_core(&OrbConfig::default(), false)
            .expect_err("nothing to resolve from, and no terminal to ask");
        let msg = err.to_string();
        for flag in [
            "--binary",
            "--build-workflow",
            "--release-workflow",
            "--crate-tag-prefix",
            "--release-after-job",
            "--docker-namespace",
        ] {
            assert!(msg.contains(flag), "error must name {flag}, got: {msg}");
        }
    }

    /// `--dry-run` is a preview, not a reason to skip the dialogue: the values
    /// have to come from somewhere before there is anything to preview (#226).
    #[test]
    fn dry_run_alone_does_not_suppress_the_dialogue() {
        // is_non_interactive() consults the terminal and $CI only; --dry-run is
        // no longer part of the decision.
        std::env::set_var("CI", "true");
        let non_interactive = is_non_interactive();
        std::env::remove_var("CI");
        assert!(non_interactive, "$CI must still force non-interactive");
    }

    // ── build_record_config ─────────────────────────────────────────────────

    #[test]
    fn build_record_config_disabled_returns_none() {
        let rec = build_record_config(false, None, None, None, None, None, None, &[])
            .expect("disabled is ok");
        assert!(rec.is_none(), "disabled must yield no [record] section");
    }

    #[test]
    fn build_record_config_collects_names_and_contexts() {
        let rec = build_record_config(
            true,
            Some("G_KEY"),
            Some("G_TRUST"),
            Some("G_NAME"),
            Some("G_EMAIL"),
            Some("G_SIGN"),
            None,
            &["release".to_string()],
        )
        .expect("all values present")
        .expect("enabled yields Some");
        assert!(rec.enabled);
        assert_eq!(rec.gpg_key_env, "G_KEY");
        assert_eq!(rec.signing_key_env, "G_SIGN");
        assert_eq!(rec.contexts, vec!["release"]);
    }

    #[test]
    fn build_record_config_errors_when_enabled_without_name() {
        let err = build_record_config(
            true,
            None, // missing gpg key env name
            Some("G_TRUST"),
            Some("G_NAME"),
            Some("G_EMAIL"),
            Some("G_SIGN"),
            None,
            &["release".to_string()],
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("--record-gpg-key-env"), "unexpected: {err}");
    }

    #[test]
    fn build_record_config_errors_when_enabled_without_context() {
        let err = build_record_config(
            true,
            Some("G_KEY"),
            Some("G_TRUST"),
            Some("G_NAME"),
            Some("G_EMAIL"),
            Some("G_SIGN"),
            None,
            &[], // no context supplied
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("--record-context"), "unexpected: {err}");
    }

    #[test]
    fn gather_extras_non_interactive_uses_hardcoded_defaults() {
        let init = make_init(true); // dry_run=true → non-interactive
        let extras = init
            .gather_extras(&[], &OrbConfig::default(), false)
            .unwrap();
        assert_eq!(extras.docker_context, DEFAULT_DOCKER_CONTEXT);
        assert_eq!(extras.orb_context, DEFAULT_ORB_CONTEXT);
        assert_eq!(extras.mcp_context, vec![DEFAULT_MCP_CONTEXT.to_string()]);
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
            mcp_context: vec!["my-mcp-ctx".to_string()],
            mcp_earliest_version: Some("1.2.3".to_string()),
            home_url: Some("https://example.com".to_string()),
            source_url: Some("https://src.example.com".to_string()),
            git_push_subcommands: vec!["save".to_string()],
            dry_run: true,
            ..make_init(true)
        };
        let extras = init
            .gather_extras(&[], &OrbConfig::default(), false)
            .unwrap();
        assert_eq!(extras.docker_context, "my-docker");
        assert_eq!(extras.orb_context, "my-orb-ctx");
        assert_eq!(extras.mcp_context, vec!["my-mcp-ctx".to_string()]);
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
        let extras = init
            .gather_extras(&[], &OrbConfig::default(), false)
            .unwrap();
        std::env::remove_var("CI");
        assert_eq!(extras.docker_context, DEFAULT_DOCKER_CONTEXT);
    }

    // ── gather_extras: skip prompts when field is explicitly set ───────────

    #[test]
    fn gather_extras_skips_docker_context_prompt_when_set() {
        let init = Init {
            docker_context: Some("explicit-docker".to_string()),
            dry_run: true,
            ..make_init(true)
        };
        let extras = init
            .gather_extras(&[], &OrbConfig::default(), false)
            .unwrap();
        assert_eq!(extras.docker_context, "explicit-docker");
    }

    #[test]
    fn gather_extras_skips_orb_context_prompt_when_set() {
        let init = Init {
            orb_context: Some("explicit-orb".to_string()),
            dry_run: true,
            ..make_init(true)
        };
        let extras = init
            .gather_extras(&[], &OrbConfig::default(), false)
            .unwrap();
        assert_eq!(extras.orb_context, "explicit-orb");
    }

    #[test]
    fn gather_extras_skips_mcp_context_prompt_when_set() {
        let init = Init {
            mcp: true,
            mcp_context: vec!["ctx-a".to_string(), "ctx-b".to_string()],
            dry_run: true,
            ..make_init(true)
        };
        let extras = init
            .gather_extras(&[], &OrbConfig::default(), false)
            .unwrap();
        assert_eq!(extras.mcp_context, vec!["ctx-a", "ctx-b"]);
    }

    #[test]
    fn gather_extras_skips_mcp_earliest_version_prompt_when_set() {
        let init = Init {
            mcp: true,
            mcp_earliest_version: Some("3.0.0".to_string()),
            dry_run: true,
            ..make_init(true)
        };
        let extras = init
            .gather_extras(&[], &OrbConfig::default(), false)
            .unwrap();
        assert_eq!(extras.mcp_earliest_version, "3.0.0");
    }

    #[test]
    fn gather_extras_skips_git_push_subcommands_prompt_when_set() {
        let init = Init {
            git_push_subcommands: vec!["deploy".to_string()],
            dry_run: true,
            ..make_init(true)
        };
        // detected list is different — CLI must win without prompting
        let extras = init
            .gather_extras(&["save".to_string()], &OrbConfig::default(), false)
            .unwrap();
        assert_eq!(extras.git_push_subcommands, vec!["deploy"]);
    }

    #[test]
    fn gather_extras_skips_home_url_prompt_when_set() {
        let init = Init {
            home_url: Some("https://example.com/home".to_string()),
            dry_run: true,
            ..make_init(true)
        };
        let extras = init
            .gather_extras(&[], &OrbConfig::default(), false)
            .unwrap();
        assert_eq!(extras.home_url.as_deref(), Some("https://example.com/home"));
    }

    #[test]
    fn gather_extras_skips_source_url_prompt_when_set() {
        let init = Init {
            source_url: Some("https://example.com/src".to_string()),
            dry_run: true,
            ..make_init(true)
        };
        let extras = init
            .gather_extras(&[], &OrbConfig::default(), false)
            .unwrap();
        assert_eq!(
            extras.source_url.as_deref(),
            Some("https://example.com/src")
        );
    }

    // ── gather_extras: existing config as fallback ─────────────────────────

    fn make_existing_config() -> OrbConfig {
        use crate::orb_config::CiSection;
        OrbConfig {
            orb: Some(OrbSection {
                home_url: Some("https://existing-home.example.com".to_string()),
                source_url: Some("https://existing-src.example.com".to_string()),
                git_push_subcommands: Some(vec!["existing-push".to_string()]),
                ..OrbSection::default()
            }),
            ci: Some(CiSection {
                docker_context: Some("existing-docker".to_string()),
                orb_context: Some("existing-orb".to_string()),
                mcp_context: Some(vec!["existing-mcp".to_string()]),
                mcp_earliest_version: Some("9.9.9".to_string()),
                ..CiSection::default()
            }),
            ..OrbConfig::default()
        }
    }

    #[test]
    fn gather_extras_falls_back_to_existing_docker_context() {
        let init = make_init(true); // dry_run → non-interactive
        let extras = init
            .gather_extras(&[], &make_existing_config(), false)
            .unwrap();
        assert_eq!(
            extras.docker_context, "existing-docker",
            "should use [ci].docker_context from existing config when CLI flag not set"
        );
    }

    #[test]
    fn gather_extras_falls_back_to_existing_orb_context() {
        let init = make_init(true);
        let extras = init
            .gather_extras(&[], &make_existing_config(), false)
            .unwrap();
        assert_eq!(extras.orb_context, "existing-orb");
    }

    #[test]
    fn gather_extras_falls_back_to_existing_mcp_context() {
        let init = Init {
            mcp: true,
            dry_run: true,
            ..make_init(true)
        };
        let extras = init
            .gather_extras(&[], &make_existing_config(), false)
            .unwrap();
        assert_eq!(extras.mcp_context, vec!["existing-mcp"]);
    }

    #[test]
    fn gather_extras_falls_back_to_existing_mcp_earliest_version() {
        let init = make_init(true);
        let extras = init
            .gather_extras(&[], &make_existing_config(), false)
            .unwrap();
        assert_eq!(extras.mcp_earliest_version, "9.9.9");
    }

    #[test]
    fn gather_extras_falls_back_to_existing_home_url() {
        let init = make_init(true);
        let extras = init
            .gather_extras(&[], &make_existing_config(), false)
            .unwrap();
        assert_eq!(
            extras.home_url.as_deref(),
            Some("https://existing-home.example.com")
        );
    }

    #[test]
    fn gather_extras_falls_back_to_existing_source_url() {
        let init = make_init(true);
        let extras = init
            .gather_extras(&[], &make_existing_config(), false)
            .unwrap();
        assert_eq!(
            extras.source_url.as_deref(),
            Some("https://existing-src.example.com")
        );
    }

    #[test]
    fn gather_extras_falls_back_to_existing_git_push_subcommands() {
        let init = make_init(true);
        // No CLI flag, no detected — should fall back to existing config
        let extras = init
            .gather_extras(&[], &make_existing_config(), false)
            .unwrap();
        assert_eq!(extras.git_push_subcommands, vec!["existing-push"]);
    }

    #[test]
    fn gather_extras_cli_takes_precedence_over_existing_config() {
        let init = Init {
            docker_context: Some("cli-docker".to_string()),
            orb_context: Some("cli-orb".to_string()),
            dry_run: true,
            ..make_init(true)
        };
        let extras = init
            .gather_extras(&[], &make_existing_config(), false)
            .unwrap();
        assert_eq!(extras.docker_context, "cli-docker");
        assert_eq!(extras.orb_context, "cli-orb");
    }

    #[test]
    fn gather_extras_detected_used_when_neither_cli_nor_config_has_push_subcommands() {
        let init = make_init(true);
        let existing = OrbConfig::default(); // no git_push_subcommands in config
        let extras = init
            .gather_extras(&["detected-push".to_string()], &existing, false)
            .unwrap();
        assert_eq!(extras.git_push_subcommands, vec!["detected-push"]);
    }

    #[test]
    fn is_non_interactive_reflects_tty_state() {
        // Verify that is_non_interactive() correctly responds to the TTY state
        // of the current process. CI environments may allocate a PTY; local
        // subprocess runs (e.g. cargo test piped) do not.
        let ci_was = std::env::var("CI").ok();
        std::env::remove_var("CI");
        let is_tty = console::Term::stderr().is_term();
        let result = is_non_interactive();
        if let Some(val) = ci_was {
            std::env::set_var("CI", val);
        }
        if is_tty {
            assert!(
                !result,
                "is_non_interactive must be false when stderr IS a terminal \
                 and $CI is not set"
            );
        } else {
            assert!(
                result,
                "is_non_interactive must be true when stderr is NOT a terminal"
            );
        }
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
            &[],
        );
        assert_eq!(
            config.orb.as_ref().unwrap().orb_dir.as_deref(),
            Some("custom-orb")
        );
    }
}
