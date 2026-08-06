use anyhow::Result;
use std::path::PathBuf;

use crate::{
    ci_patcher,
    commands::generate::Generate,
    help_parser::types::CliDefinition,
    orb_config::{
        non_empty, CiSection, OrbConfig, OrbSection, RecordConfig, DEFAULT_CRATE_WAIT_ATTEMPTS,
        DEFAULT_CRATE_WAIT_SECONDS,
    },
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

/// The `[orb]` settings `init` records so the config states them rather than
/// leaving them to resolve from code the consumer cannot see (#251).
///
/// Each is a *seed*: the generator's recommendation is offered as the prompt
/// default, and what comes back is the project's stated configuration from then
/// on — maintained by the user (Renovate, for the pinned images). A later
/// generator release changing a recommendation is therefore correct and does
/// not reach an existing project.
#[derive(Debug)]
pub(crate) struct GatheredOrb {
    pub orb_dir: String,
    pub install_method: String,
    pub base_image: String,
    pub builder_image: String,
    /// `None` means "not recorded", which for this one setting is meaningful:
    /// a version is also an opt-in to bundling the CLI, so it is recorded only
    /// for a binary that intrinsically needs it. See [`OrbSection`].
    pub circleci_cli_version: Option<String>,
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

/// Resolve one gathered value.
///
/// The shape every field in the dialogue shares: take the flag if it was given,
/// otherwise ask with the recorded value offered as the default, and when there
/// is nobody to ask, take that same fallback. Writing it once is what keeps the
/// interactive and non-interactive paths from drifting apart — they previously
/// disagreed about whether an empty flag counted as a value.
fn resolve_value(
    flag: Option<String>,
    fallback: String,
    prompt: &str,
    interactive: bool,
) -> Result<String> {
    // A flag that was supplied is an answer, even an empty one: a wrapper
    // script passing an unset variable must not start blocking on stdin. Empty
    // is not a usable value here, so it means "take the fallback".
    if let Some(flag) = flag {
        return Ok(non_empty(Some(flag)).unwrap_or(fallback));
    }
    if !interactive {
        return Ok(fallback);
    }
    Ok(dialoguer::Input::<String>::new()
        .with_prompt(prompt)
        .default(fallback)
        .interact_text()?)
}

/// As [`resolve_value`], for a value that may legitimately be absent: an empty
/// answer means "not set" rather than an empty string.
fn resolve_optional(
    flag: Option<String>,
    fallback: Option<String>,
    prompt: &str,
    interactive: bool,
) -> Result<Option<String>> {
    // As above, a supplied flag is an answer — and here an empty one is a
    // meaningful answer: "no value", which is exactly how it read before the
    // resolvers were shared.
    if let Some(flag) = flag {
        return Ok(non_empty(Some(flag)));
    }
    if !interactive {
        return Ok(non_empty(fallback));
    }
    let answer = dialoguer::Input::<String>::new()
        .with_prompt(prompt)
        .default(fallback.unwrap_or_default())
        .allow_empty(true)
        .interact_text()?;
    Ok(non_empty(Some(answer)))
}

/// As [`resolve_value`], for a comma-separated list.
fn resolve_list(
    flag: &[String],
    fallback: Vec<String>,
    prompt: &str,
    interactive: bool,
) -> Result<Vec<String>> {
    // Flag values go through the same normalisation as a typed answer: the
    // generator matches subcommand names exactly, so `--flag "save, push"`
    // leaving a leading space would silently drop the step it asked for.
    if !flag.is_empty() {
        let cleaned = split_list(&flag.join(","));
        return Ok(if cleaned.is_empty() {
            fallback
        } else {
            cleaned
        });
    }
    if !interactive {
        return Ok(fallback);
    }
    let answer = dialoguer::Input::<String>::new()
        .with_prompt(prompt)
        .default(fallback.join(","))
        .allow_empty(true)
        .interact_text()?;
    Ok(split_list(&answer))
}

/// Split a comma-separated answer, discarding blanks.
fn split_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
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
    /// Falls back to `[orb] orb_dir` in the config; prompted for if neither is set.
    #[arg(long)]
    pub orb_dir: Option<String>,

    /// How the generated Dockerfile obtains the binary: `binstall`, `apt` or `local`.
    /// Falls back to `[orb] install_method` in the config; prompted for if neither is set.
    #[arg(long)]
    pub install_method: Option<String>,

    /// Runtime stage image for the generated Dockerfile.
    /// Falls back to `[orb] base_image` in the config; prompted for if neither is set.
    #[arg(long)]
    pub base_image: Option<String>,

    /// Image for the Rust `builder` stage that installs the binary.
    /// Falls back to `[orb] builder_image` in the config; prompted for if neither is set.
    #[arg(long)]
    pub builder_image: Option<String>,

    /// circleci-cli version to bundle into the generated image. Only needed for a
    /// binary that shells out to `circleci`; empty records nothing and bundles no CLI.
    /// Falls back to `[orb] circleci_cli_version` in the config.
    #[arg(long)]
    pub circleci_cli_version: Option<String>,

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

/// Assemble the config `init` writes.
///
/// `existing` is the config being re-initialised (or [`OrbConfig::default`] on
/// a first run). Sections this dialogue does not gather are carried across from
/// it rather than dropped: `init` used to null `[[job_group]]`, `[[extra_job]]`
/// and `[orbs]`, and since the save now removes keys the config no longer has,
/// a re-run deleted the consumer's hand-authored job groups along with their
/// comments (#268). `[ci]` and `[record]` are already carried this way, via the
/// gathered values.
pub(crate) fn build_bootstrap_config(
    binary: &str,
    namespaces: &[String],
    orb: &GatheredOrb,
    extras: &GatheredExtras,
    existing: &OrbConfig,
    interactive: &[(String, bool)],
) -> OrbConfig {
    // `help` is reserved at the `--help` parser, so it needs no entry. Interactive
    // (CLI-only) subcommands — `init`/`config` by default, as confirmed at init
    // time — are fully excluded from the orb (job + command + script); a parent
    // (`config`) cascades to its whole subtree, so no per-child entries are needed.
    //
    // Merged into the recorded entries rather than replacing them (#271): the
    // dialogue owns `interactive` for the subcommands it asked about and nothing
    // else, so a curated `label`, a `short_param` naming, or a `param` override
    // has to survive. Rebuilding the map dropped all three — and losing a
    // `short_param` entry is not merely cosmetic: a short-only option with no
    // derivable name fails generation until it is restored by hand.
    let mut subcommands = existing.subcommand.clone().unwrap_or_default();
    for (name, is_interactive) in interactive {
        subcommands.entry(name.clone()).or_default().interactive = Some(*is_interactive);
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
            orb_dir: orb.orb_dir.clone(),
            install_method: orb.install_method.clone(),
            base_image: orb.base_image.clone(),
            builder_image: orb.builder_image.clone(),
            circleci_cli_version: orb.circleci_cli_version.clone(),
            home_url: extras.home_url.clone(),
            source_url: extras.source_url.clone(),
            git_push_subcommands: if extras.git_push_subcommands.is_empty() {
                None
            } else {
                Some(extras.git_push_subcommands.clone())
            },
            // Settings with no place in the dialogue — advanced knobs, edited in
            // the toml — are carried across for the same reason as the sections
            // below: `init` is a bootstrap, not a reset.
            apt_packages: existing.orb.as_ref().and_then(|o| o.apt_packages.clone()),
            cargo_tools: existing.orb.as_ref().and_then(|o| o.cargo_tools.clone()),
            custom_files: existing.orb.as_ref().and_then(|o| o.custom_files.clone()),
            allow_unparsed_help: existing.orb.as_ref().and_then(|o| o.allow_unparsed_help),
            crate_wait_attempts: existing
                .orb
                .as_ref()
                .map(|o| o.crate_wait_attempts)
                .unwrap_or(DEFAULT_CRATE_WAIT_ATTEMPTS),
            crate_wait_seconds: existing
                .orb
                .as_ref()
                .map(|o| o.crate_wait_seconds)
                .unwrap_or(DEFAULT_CRATE_WAIT_SECONDS),
        }),
        ci: None, // populated by run() after gathering extras
        orbs: existing.orbs.clone(),
        subcommand,
        job_group: existing.job_group.clone(),
        extra_job: existing.extra_job.clone(),
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

        // Filter once, here, so the interactive and non-interactive branches
        // agree on what "missing" means: a blank value is missing in both.
        let pick = |flag: Option<String>, configured: Option<String>| -> Option<String> {
            non_empty(flag).or_else(|| non_empty(configured))
        };
        let binary = pick(self.binary.clone(), orb.and_then(|o| o.binary.clone()));
        let build_workflow = pick(
            self.build_workflow.clone(),
            ci.and_then(|c| c.build_workflow.clone()),
        );
        let release_workflow = pick(
            self.release_workflow.clone(),
            ci.and_then(|c| c.release_workflow.clone()),
        );
        let crate_tag_prefix = pick(
            self.crate_tag_prefix.clone(),
            ci.and_then(|c| c.crate_tag_prefix.clone()),
        );
        let release_after_job = pick(
            self.release_after_job.clone(),
            ci.and_then(|c| c.release_after_job.clone()),
        );
        let docker_namespace = pick(
            self.docker_namespace.clone(),
            ci.and_then(|c| c.docker_namespace.clone()),
        );

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
                     (running under CI, or with stderr redirected): {}. \
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
        // The candidates are already non-empty or None (see `pick` above).
        let resolve = |label: &str, value: Option<String>| -> Result<String> {
            match value {
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
        let crate_tag_prefix = match crate_tag_prefix {
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

    /// Resolve the `[orb]` settings the config should state (#251).
    ///
    /// Same order as everything else in the dialogue: flag > recorded value >
    /// the generator's recommendation, offered as the prompt default. The
    /// recorded value is never "unset" for four of the five — `OrbSection`
    /// carries the recommendation — so a re-run offers back what the project
    /// already chose rather than resetting it.
    ///
    /// `needs_circleci_cli` comes from the binary's own `--help`, so the CLI is
    /// offered where it is actually required and left out otherwise.
    pub(crate) fn gather_orb(
        &self,
        existing: &OrbConfig,
        needs_circleci_cli: bool,
        interactive: bool,
    ) -> Result<GatheredOrb> {
        use crate::orb_config::{
            DEFAULT_BASE_IMAGE, DEFAULT_BUILDER_IMAGE, DEFAULT_INSTALL_METHOD, DEFAULT_ORB_DIR,
            MCP_DEFAULT_BASE_IMAGE,
        };
        let orb = existing.orb.as_ref();
        let recorded = |value: Option<String>, recommended: &str| -> String {
            non_empty(value).unwrap_or_else(|| recommended.to_string())
        };

        let orb_dir = resolve_value(
            self.orb_dir.clone(),
            recorded(orb.map(|o| o.orb_dir.clone()), DEFAULT_ORB_DIR),
            "Directory to generate the orb source into",
            interactive,
        )?;
        let install_method = resolve_value(
            self.install_method.clone(),
            recorded(
                orb.map(|o| o.install_method.clone()),
                DEFAULT_INSTALL_METHOD,
            ),
            "How the executor image installs the binary (binstall, apt, local)",
            interactive,
        )?;
        // Checked here rather than left to the generator's warning: `init` both
        // records the answer and generates from it, so an unrecognised value
        // would be written to the config *and* silently produce an image built
        // by whatever method the old config named — two different wrong answers
        // from one typo.
        if crate::commands::generate::install_method_from_str(&install_method).is_none() {
            anyhow::bail!("install_method \"{install_method}\" is not one of binstall, apt, local");
        }

        // MCP compiles the MCP server in the executor, so it needs cargo there.
        // Recommending the right image up front is what keeps the correction in
        // `resolve_base_image` a safety net rather than the normal path.
        let recommended_base = if self.mcp {
            MCP_DEFAULT_BASE_IMAGE
        } else {
            DEFAULT_BASE_IMAGE
        };
        // A recorded value equal to the plain default is the *unset* case, not a
        // choice: `base_image` is materialised, so any config with an `[orb]`
        // table deserialises it to `DEFAULT_BASE_IMAGE` whether or not the key
        // was ever written. Treating that as a decision made `--mcp` recommend
        // debian for every existing repo — and the safety net in
        // `resolve_base_image` cannot cover for it here, because `[ci] mcp` is
        // not written to the config until after generation.
        let recorded_base = non_empty(orb.map(|o| o.base_image.clone()))
            .filter(|image| !(self.mcp && image == DEFAULT_BASE_IMAGE));
        let base_image = resolve_value(
            self.base_image.clone(),
            recorded_base.unwrap_or_else(|| recommended_base.to_string()),
            "Runtime image for the executor (pin a @sha256 digest to keep it)",
            interactive,
        )?;
        let builder_image = resolve_value(
            self.builder_image.clone(),
            recorded(orb.map(|o| o.builder_image.clone()), DEFAULT_BUILDER_IMAGE),
            "Rust image the binary is built/installed in (pin a @sha256 digest to keep it)",
            interactive,
        )?;

        // Optional, unlike the four above: recording a version also opts the
        // image into carrying the CLI, so an answer of "none" has to remain
        // expressible — and has to be the default for a binary that never
        // invokes `circleci`.
        let recommended_cli = needs_circleci_cli
            .then(|| crate::commands::generate::DEFAULT_CIRCLECI_CLI_VERSION.to_string());
        let circleci_cli_version = resolve_optional(
            self.circleci_cli_version.clone(),
            non_empty(orb.and_then(|o| o.circleci_cli_version.clone())).or(recommended_cli),
            "circleci-cli version to bundle in the image (Enter to skip)",
            interactive,
        )?;

        Ok(GatheredOrb {
            orb_dir,
            install_method,
            base_image,
            builder_image,
            circleci_cli_version,
        })
    }

    pub(crate) fn gather_extras(
        &self,
        detected: &[String],
        detected_url: Option<&str>,
        existing: &OrbConfig,
        interactive: bool,
    ) -> Result<GatheredExtras> {
        // Resolution order, for every field: CLI flag > existing config >
        // auto-detected or hardcoded default. Interactive runs offer that
        // fallback as the prompt default rather than taking it silently.
        let ci = existing.ci.as_ref();
        let orb = existing.orb.as_ref();
        let record = self.gather_record(existing, interactive)?;

        // Seed both from the repo's own remote (#269). `generate` detects it
        // anyway and puts it in the orb, so offering nothing here made Enter
        // record a blank for a value the orb was going to carry regardless —
        // leaving the config unable to say, or change, what the registry links
        // would be. Still optional: an explicit empty answer clears them.
        let detected_url = non_empty(detected_url.map(str::to_string));
        let home_url = resolve_optional(
            self.home_url.clone(),
            non_empty(orb.and_then(|o| o.home_url.clone())).or_else(|| detected_url.clone()),
            "Home URL for orb registry (Enter to skip)",
            interactive,
        )?;
        let source_url = resolve_optional(
            self.source_url.clone(),
            non_empty(orb.and_then(|o| o.source_url.clone())).or(detected_url),
            "Source URL for orb registry (Enter to skip)",
            interactive,
        )?;

        // The recorded list wins over detection; detection seeds the prompt when
        // nothing has been recorded yet.
        let configured_push = orb
            .and_then(|o| o.git_push_subcommands.clone())
            .filter(|v| !v.is_empty());
        let push_prompt = if configured_push.is_none() && !detected.is_empty() {
            format!(
                "Push-capable subcommands detected: {} — confirm or override (comma-separated)",
                detected.join(", ")
            )
        } else {
            "Subcommands that push to git, comma-separated (e.g. save)".to_string()
        };
        let git_push_subcommands = resolve_list(
            &self.git_push_subcommands,
            configured_push.unwrap_or_else(|| detected.to_vec()),
            &push_prompt,
            interactive,
        )?;

        let docker_context = resolve_value(
            self.docker_context.clone(),
            non_empty(ci.and_then(|c| c.docker_context.clone()))
                .unwrap_or_else(|| DEFAULT_DOCKER_CONTEXT.to_string()),
            "Docker context name (needs: DOCKER_LOGIN, DOCKER_PASSWORD)",
            interactive,
        )?;
        let orb_context = resolve_value(
            self.orb_context.clone(),
            non_empty(ci.and_then(|c| c.orb_context.clone()))
                .unwrap_or_else(|| DEFAULT_ORB_CONTEXT.to_string()),
            "Orb publishing context name (needs: CIRCLECI_CLI_TOKEN)",
            interactive,
        )?;

        // The MCP settings only matter when MCP wiring was asked for, so they
        // are never prompted for otherwise — the value is still resolved so the
        // config records something sensible either way.
        let ask_mcp = interactive && self.mcp;
        let mcp_context = resolve_list(
            &self.mcp_context,
            ci.and_then(|c| c.mcp_context.clone())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| vec![DEFAULT_MCP_CONTEXT.to_string()]),
            "MCP context names, comma-separated (needs: GITHUB_TOKEN with contents:write \
             + bypass branch protection, BOT_GPG_KEY, BOT_TRUST, BOT_USER_NAME, \
             BOT_USER_EMAIL, BOT_SIGN_KEY)",
            ask_mcp,
        )?;
        let mcp_earliest_version = resolve_value(
            self.mcp_earliest_version.clone(),
            non_empty(ci.and_then(|c| c.mcp_earliest_version.clone()))
                .unwrap_or_else(|| DEFAULT_MCP_EARLIEST_VERSION.to_string()),
            "Earliest orb version to include in MCP snapshots",
            ask_mcp,
        )?;

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

        // The config is read first: it carries the parser settings (e.g.
        // allow_unparsed_help) AND the fallbacks for the values gathered below —
        // one of which is the binary there is no point introspecting until it is
        // resolved.
        let config_path = std::path::Path::new("gen-circleci-orb.toml");
        let existing_config = crate::orb_config::load_config(config_path)?;
        let core = self.gather_core(&existing_config, interactive)?;

        // Parse binary early: detect push-capable subcommands (for dialogue default),
        // subcommands with a required orb_path param (for config defaults), and
        // whether the binary shells out to `circleci` (for the CLI version prompt).
        let (detected_push, detected_orb_path, present_interactive, needs_circleci_cli) =
            match crate::help_parser::parse_binary(
                &core.binary,
                &crate::commands::generate::parse_options(&existing_config),
            ) {
                Ok(cli) => (
                    detect_git_push_subcommands(&cli),
                    detect_orb_path_subcommands(&cli),
                    present_default_interactive(&cli),
                    crate::commands::generate::binary_uses_circleci_cli(&cli),
                ),
                Err(e) => {
                    // Not fatal here — `generate` re-runs the parse a moment later
                    // and fails properly. But the binary may have just been typed
                    // at a prompt, so say why it could not be read, rather than
                    // letting it surface one step downstream as a generate error.
                    tracing::warn!("could not introspect `{}`: {e:#}", core.binary);
                    (vec![], vec![], vec![], false)
                }
            };

        let orb = self.gather_orb(&existing_config, needs_circleci_cli, interactive)?;
        let extras = self.gather_extras(
            &detected_push,
            crate::commands::generate::detect_source_url().as_deref(),
            &existing_config,
            interactive,
        )?;
        let namespaces: Vec<String> = self
            .public_orb_namespaces
            .iter()
            .chain(self.private_orb_namespaces.iter())
            .cloned()
            .collect();

        // Step 1: generate orb source files
        tracing::info!("Generating orb source into ./{}", orb.orb_dir);
        let gen = Generate {
            binary: Some(core.binary.clone()),
            namespaces: namespaces.clone(),
            output: PathBuf::from("."),
            orb_dir: Some(orb.orb_dir.clone()),
            // Passed explicitly because `generate` runs *before* the config is
            // written in step 3 — it would otherwise resolve from the config as
            // it was, ignoring what the dialogue just gathered.
            install_method: crate::commands::generate::install_method_from_str(&orb.install_method),
            base_image: Some(orb.base_image.clone()),
            builder_image: Some(orb.builder_image.clone()),
            home_url: extras.home_url.clone(),
            source_url: extras.source_url.clone(),
            git_push_subcommands: extras.git_push_subcommands.clone(),
            circleci_cli_version: orb.circleci_cli_version.clone(),
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
            orb_dir: orb.orb_dir.clone(),
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
            &orb,
            &extras,
            &existing_config,
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
    use crate::orb_config::SubcommandConfig;
    use indexmap::IndexMap;

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

    /// The `[orb]` settings as a first `init` would resolve them: every one at
    /// the generator's recommendation.
    fn gathered_orb() -> GatheredOrb {
        GatheredOrb {
            orb_dir: crate::orb_config::DEFAULT_ORB_DIR.to_string(),
            install_method: crate::orb_config::DEFAULT_INSTALL_METHOD.to_string(),
            base_image: crate::orb_config::DEFAULT_BASE_IMAGE.to_string(),
            builder_image: crate::orb_config::DEFAULT_BUILDER_IMAGE.to_string(),
            circleci_cli_version: None,
        }
    }

    fn gathered_extras() -> GatheredExtras {
        GatheredExtras {
            home_url: None,
            source_url: None,
            git_push_subcommands: vec![],
            docker_context: DEFAULT_DOCKER_CONTEXT.to_string(),
            orb_context: DEFAULT_ORB_CONTEXT.to_string(),
            mcp_context: vec![],
            mcp_earliest_version: DEFAULT_MCP_EARLIEST_VERSION.to_string(),
            record: None,
        }
    }

    /// `build_bootstrap_config` against no existing config — a first `init`.
    fn bootstrap_with(
        binary: &str,
        namespaces: &[String],
        orb: GatheredOrb,
        extras: GatheredExtras,
        interactive: &[(String, bool)],
    ) -> OrbConfig {
        build_bootstrap_config(
            binary,
            namespaces,
            &orb,
            &extras,
            &OrbConfig::default(),
            interactive,
        )
    }

    #[test]
    fn bootstrap_config_has_orb_section_with_binary() {
        let config = bootstrap_with(
            "mytool",
            &["my-org".to_string()],
            gathered_orb(),
            gathered_extras(),
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
        let config = bootstrap_with(
            "mytool",
            &["ns1".to_string(), "ns2".to_string()],
            gathered_orb(),
            gathered_extras(),
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
        let config = bootstrap_with(
            "mytool",
            &["my-org".to_string()],
            gathered_orb(),
            gathered_extras(),
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
        let config = bootstrap_with(
            "mytool",
            &["my-org".to_string()],
            gathered_orb(),
            gathered_extras(),
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
            orb_dir: None,
            install_method: None,
            base_image: None,
            builder_image: None,
            circleci_cli_version: None,
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
        let config = bootstrap_with(
            "mytool",
            &["my-org".to_string()],
            gathered_orb(),
            GatheredExtras {
                git_push_subcommands: vec!["save".to_string()],
                ..gathered_extras()
            },
            &[],
        );
        assert_eq!(
            config.orb.as_ref().unwrap().git_push_subcommands.as_deref(),
            Some(&["save".to_string()][..])
        );
    }

    #[test]
    fn bootstrap_config_git_push_subcommands_none_when_empty() {
        let config = bootstrap_with(
            "mytool",
            &["my-org".to_string()],
            gathered_orb(),
            gathered_extras(),
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
            .gather_extras(&[], None, &OrbConfig::default(), false)
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
        let config = bootstrap_with(
            "mytool",
            &["my-org".to_string()],
            gathered_orb(),
            GatheredExtras {
                home_url: Some("https://example.com/home".to_string()),
                source_url: Some("https://example.com/source".to_string()),
                ..gathered_extras()
            },
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
            ..Default::default()
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
        let mut config = bootstrap_with(
            "mytool",
            &["my-org".to_string()],
            gathered_orb(),
            gathered_extras(),
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
        let mut config = bootstrap_with(
            "mytool",
            &["my-org".to_string()],
            gathered_orb(),
            gathered_extras(),
            &[("init".to_string(), true)],
        );
        populate_orb_path_defaults(&mut config, &["generate".to_string()]);
        let subcommands = config.subcommand.as_ref().unwrap();
        assert_eq!(subcommands.get("init").unwrap().interactive, Some(true));
    }

    #[test]
    fn populate_orb_path_defaults_noop_when_empty() {
        let mut config = bootstrap_with(
            "mytool",
            &["my-org".to_string()],
            gathered_orb(),
            gathered_extras(),
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            .gather_extras(&["save".to_string()], None, &OrbConfig::default(), false)
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
            .gather_extras(&["save".to_string()], None, &OrbConfig::default(), false)
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
            orb_dir: None,
            install_method: None,
            base_image: None,
            builder_image: None,
            circleci_cli_version: None,
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

    // ── the shared resolvers ───────────────────────────────────────────────

    /// A flag that was given — even as an empty string — is an answer. Falling
    /// through to a prompt would block a wrapper script passing an unset
    /// variable, which previously just proceeded.
    ///
    /// `interactive = true` here is the assertion: with no terminal attached a
    /// prompt fails, so returning `Ok` proves none was issued.
    #[test]
    fn an_explicitly_empty_flag_is_answered_not_prompted() {
        assert_eq!(
            resolve_optional(Some(String::new()), Some("recorded".into()), "?", true).unwrap(),
            None,
            "an empty optional flag means 'none', without asking"
        );
        assert_eq!(
            resolve_value(Some(String::new()), "fallback".into(), "?", true).unwrap(),
            "fallback",
            "an empty required flag falls back, without asking"
        );
        assert_eq!(
            resolve_list(&[String::new()], vec!["recorded".into()], "?", true).unwrap(),
            vec!["recorded".to_string()],
            "an empty list flag falls back, without asking"
        );
    }

    /// A blank recorded value is not a value: the hardcoded default must still
    /// apply, or `[ci] docker_context = ""` silently yields a job with no
    /// context and a registry push that cannot authenticate.
    #[test]
    fn a_blank_recorded_value_does_not_defeat_the_default() {
        let existing = OrbConfig {
            ci: Some(CiSection {
                docker_context: Some(String::new()),
                orb_context: Some("   ".to_string()),
                ..CiSection::default()
            }),
            ..OrbConfig::default()
        };
        let init = Init {
            docker_context: None,
            orb_context: None,
            ..make_init(false)
        };
        let extras = init.gather_extras(&[], None, &existing, false).unwrap();
        assert_eq!(extras.docker_context, DEFAULT_DOCKER_CONTEXT);
        assert_eq!(extras.orb_context, DEFAULT_ORB_CONTEXT);
    }

    /// Flag values and prompt answers must be normalised the same way — the
    /// generator matches subcommand names exactly, so a stray space silently
    /// drops the step the entry was asking for.
    #[test]
    fn list_flag_values_are_trimmed_like_prompt_answers() {
        let flags = vec!["save".to_string(), " push".to_string(), String::new()];
        assert_eq!(
            resolve_list(&flags, vec![], "?", false).unwrap(),
            vec!["save".to_string(), "push".to_string()],
            "flag values must be trimmed and blanks dropped, as answers are"
        );
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

    /// An empty value is not a value. The interactive path already filtered
    /// them out; the non-interactive guard tested only for absence, so a config
    /// carrying `binary = ""` walked straight through it and init proceeded to
    /// write empty strings into the CI config it patches.
    #[test]
    fn gather_core_treats_empty_values_as_missing() {
        let existing = OrbConfig {
            orb: Some(OrbSection {
                binary: Some(String::new()),
                ..OrbSection::default()
            }),
            ci: Some(CiSection {
                build_workflow: Some(String::new()),
                release_workflow: Some(String::new()),
                crate_tag_prefix: Some(String::new()),
                release_after_job: Some(String::new()),
                docker_namespace: Some(String::new()),
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
        let err = init
            .gather_core(&existing, false)
            .expect_err("empty values must be treated as missing");
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

    /// An empty flag value is no better than an empty config value.
    #[test]
    fn gather_core_treats_empty_cli_values_as_missing() {
        let init = Init {
            binary: Some(String::new()),
            ..make_init(false)
        };
        let err = init
            .gather_core(&OrbConfig::default(), false)
            .expect_err("an empty --binary must be treated as missing");
        assert!(err.to_string().contains("--binary"));
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
            .gather_extras(&[], None, &OrbConfig::default(), false)
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
            .gather_extras(&[], None, &OrbConfig::default(), false)
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

    /// `$CI` is process-global, so the tests that set it cannot run alongside
    /// the ones that read it. Rust runs tests in parallel by default: without
    /// this, one test removing `$CI` can flip another's reading of it.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// A poisoned lock only means some other test panicked while holding it —
    /// the data is `()`, so there is nothing to protect against.
    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn ci_env_var_forces_non_interactive() {
        let _guard = lock_env();
        let was = std::env::var("CI").ok();
        std::env::set_var("CI", "true");
        let result = is_non_interactive();
        match was {
            Some(v) => std::env::set_var("CI", v),
            None => std::env::remove_var("CI"),
        }
        assert!(
            result,
            "$CI set must force non-interactive, whatever the terminal says"
        );
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
            .gather_extras(&[], None, &OrbConfig::default(), false)
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
            .gather_extras(&[], None, &OrbConfig::default(), false)
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
            .gather_extras(&[], None, &OrbConfig::default(), false)
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
            .gather_extras(&[], None, &OrbConfig::default(), false)
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
            .gather_extras(&["save".to_string()], None, &OrbConfig::default(), false)
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
            .gather_extras(&[], None, &OrbConfig::default(), false)
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
            .gather_extras(&[], None, &OrbConfig::default(), false)
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
            .gather_extras(&[], None, &make_existing_config(), false)
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
            .gather_extras(&[], None, &make_existing_config(), false)
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
            .gather_extras(&[], None, &make_existing_config(), false)
            .unwrap();
        assert_eq!(extras.mcp_context, vec!["existing-mcp"]);
    }

    #[test]
    fn gather_extras_falls_back_to_existing_mcp_earliest_version() {
        let init = make_init(true);
        let extras = init
            .gather_extras(&[], None, &make_existing_config(), false)
            .unwrap();
        assert_eq!(extras.mcp_earliest_version, "9.9.9");
    }

    #[test]
    fn gather_extras_falls_back_to_existing_home_url() {
        let init = make_init(true);
        let extras = init
            .gather_extras(&[], None, &make_existing_config(), false)
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
            .gather_extras(&[], None, &make_existing_config(), false)
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
            .gather_extras(&[], None, &make_existing_config(), false)
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
            .gather_extras(&[], None, &make_existing_config(), false)
            .unwrap();
        assert_eq!(extras.docker_context, "cli-docker");
        assert_eq!(extras.orb_context, "cli-orb");
    }

    #[test]
    fn gather_extras_detected_used_when_neither_cli_nor_config_has_push_subcommands() {
        let init = make_init(true);
        let existing = OrbConfig::default(); // no git_push_subcommands in config
        let extras = init
            .gather_extras(&["detected-push".to_string()], None, &existing, false)
            .unwrap();
        assert_eq!(extras.git_push_subcommands, vec!["detected-push"]);
    }

    #[test]
    fn is_non_interactive_reflects_tty_state() {
        // Verify that is_non_interactive() correctly responds to the TTY state
        // of the current process. CI environments may allocate a PTY; local
        // subprocess runs (e.g. cargo test piped) do not.
        let _guard = lock_env();
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
        let config = bootstrap_with(
            "mytool",
            &["my-org".to_string()],
            GatheredOrb {
                orb_dir: "custom-orb".to_string(),
                ..gathered_orb()
            },
            gathered_extras(),
            &[],
        );
        assert_eq!(config.orb.as_ref().unwrap().orb_dir, "custom-orb");
    }

    // ── #251: init records the [orb] settings ──────────────────────────────

    #[test]
    fn init_records_the_orb_settings_at_their_recommendations() {
        let orb = make_init(true)
            .gather_orb(&OrbConfig::default(), false, false)
            .unwrap();
        assert_eq!(orb.orb_dir, crate::orb_config::DEFAULT_ORB_DIR);
        assert_eq!(
            orb.install_method,
            crate::orb_config::DEFAULT_INSTALL_METHOD
        );
        assert_eq!(orb.base_image, crate::orb_config::DEFAULT_BASE_IMAGE);
        assert_eq!(orb.builder_image, crate::orb_config::DEFAULT_BUILDER_IMAGE);
    }

    /// MCP compiles the MCP server inside the executor, so the recommendation
    /// has to be the image that has cargo — recording `debian:13-slim` here
    /// would produce a `build_mcp_server` that fails in CI.
    #[test]
    fn init_recommends_the_mcp_base_image_when_mcp_is_wired_in() {
        let mut init = make_init(true);
        init.mcp = true;
        let orb = init
            .gather_orb(&OrbConfig::default(), false, false)
            .unwrap();
        assert_eq!(orb.base_image, crate::orb_config::MCP_DEFAULT_BASE_IMAGE);
    }

    /// The recommendation has to reach a repo that already has an `[orb]` table.
    /// `base_image` is materialised, so *every* such config deserialises it to
    /// `debian:13-slim` whether the key was written or not — treating that as a
    /// stated choice made `--mcp` recommend an executor with no cargo for every
    /// existing repo, which `resolve_base_image` cannot correct for either,
    /// since `[ci] mcp` is not on disk until after generation.
    #[test]
    fn init_recommends_the_mcp_base_image_over_a_materialised_plain_default() {
        let existing = OrbConfig {
            orb: Some(OrbSection {
                binary: Some("mytool".to_string()),
                ..OrbSection::default()
            }),
            ..OrbConfig::default()
        };
        let mut init = make_init(true);
        init.mcp = true;
        let orb = init.gather_orb(&existing, false, false).unwrap();
        assert_eq!(orb.base_image, crate::orb_config::MCP_DEFAULT_BASE_IMAGE);
    }

    /// …but a genuinely chosen image still wins, MCP or not.
    #[test]
    fn init_keeps_a_chosen_base_image_under_mcp() {
        let existing = OrbConfig {
            orb: Some(OrbSection {
                base_image: "debian:13-slim@sha256:chosen".to_string(),
                ..OrbSection::default()
            }),
            ..OrbConfig::default()
        };
        let mut init = make_init(true);
        init.mcp = true;
        let orb = init.gather_orb(&existing, false, false).unwrap();
        assert_eq!(orb.base_image, "debian:13-slim@sha256:chosen");
    }

    /// `init` records the answer *and* generates from it, so an unrecognised
    /// value would be written to the config and silently build an image by
    /// whatever method the old config named. Refuse it instead.
    #[test]
    fn init_rejects_an_unrecognised_install_method() {
        let mut init = make_init(true);
        init.install_method = Some("Local".to_string());
        let err = init
            .gather_orb(&OrbConfig::default(), false, false)
            .expect_err("an unrecognised install method must not be accepted");
        let message = err.to_string();
        assert!(
            message.contains("Local") && message.contains("binstall"),
            "the error must name the bad value and the valid ones: {message}"
        );
    }

    #[test]
    fn init_accepts_every_valid_install_method() {
        for method in ["binstall", "apt", "local"] {
            let mut init = make_init(true);
            init.install_method = Some(method.to_string());
            let orb = init
                .gather_orb(&OrbConfig::default(), false, false)
                .unwrap();
            assert_eq!(orb.install_method, method);
        }
    }

    /// The property `circleci_cli_version` is kept `Option` for: materialising a
    /// version would bundle the CLI into every executor, turning "installed
    /// because the binary needs it" into "installed because the file says so".
    #[test]
    fn init_records_no_circleci_cli_version_for_a_binary_that_does_not_need_one() {
        let orb = make_init(true)
            .gather_orb(&OrbConfig::default(), false, false)
            .unwrap();
        assert_eq!(orb.circleci_cli_version, None);
    }

    #[test]
    fn init_records_the_default_circleci_cli_version_for_a_binary_that_needs_one() {
        let orb = make_init(true)
            .gather_orb(&OrbConfig::default(), true, false)
            .unwrap();
        assert_eq!(
            orb.circleci_cli_version.as_deref(),
            Some(crate::commands::generate::DEFAULT_CIRCLECI_CLI_VERSION)
        );
    }

    /// A re-run offers back what the project already chose. The pinned digests
    /// in these two keys are the reason: resetting them to the unpinned
    /// recommendation would silently un-pin the executor's images.
    #[test]
    fn init_keeps_the_recorded_orb_settings() {
        let existing = OrbConfig {
            orb: Some(OrbSection {
                orb_dir: "src/orb".to_string(),
                install_method: "local".to_string(),
                base_image: "debian:13-slim@sha256:abc".to_string(),
                builder_image: "rust:1-slim-trixie@sha256:def".to_string(),
                circleci_cli_version: Some("0.1.30000".to_string()),
                ..OrbSection::default()
            }),
            ..OrbConfig::default()
        };
        let orb = make_init(true).gather_orb(&existing, true, false).unwrap();
        assert_eq!(orb.orb_dir, "src/orb");
        assert_eq!(orb.install_method, "local");
        assert_eq!(orb.base_image, "debian:13-slim@sha256:abc");
        assert_eq!(orb.builder_image, "rust:1-slim-trixie@sha256:def");
        assert_eq!(orb.circleci_cli_version.as_deref(), Some("0.1.30000"));
    }

    #[test]
    fn init_flags_win_over_the_recorded_orb_settings() {
        let existing = OrbConfig {
            orb: Some(OrbSection {
                orb_dir: "src/orb".to_string(),
                ..OrbSection::default()
            }),
            ..OrbConfig::default()
        };
        let mut init = make_init(true);
        init.orb_dir = Some("elsewhere".to_string());
        init.builder_image = Some("rust:1-slim-trixie@sha256:flag".to_string());
        let orb = init.gather_orb(&existing, false, false).unwrap();
        assert_eq!(orb.orb_dir, "elsewhere");
        assert_eq!(orb.builder_image, "rust:1-slim-trixie@sha256:flag");
    }

    #[test]
    fn the_written_config_states_the_orb_settings() {
        let config = bootstrap_with(
            "mytool",
            &["my-org".to_string()],
            gathered_orb(),
            gathered_extras(),
            &[],
        );
        let orb = config.orb.as_ref().unwrap();
        assert_eq!(orb.orb_dir, crate::orb_config::DEFAULT_ORB_DIR);
        assert_eq!(
            orb.install_method,
            crate::orb_config::DEFAULT_INSTALL_METHOD
        );
        assert_eq!(orb.base_image, crate::orb_config::DEFAULT_BASE_IMAGE);
        assert_eq!(orb.builder_image, crate::orb_config::DEFAULT_BUILDER_IMAGE);
    }

    // ── #268: a re-run is a bootstrap, not a reset ─────────────────────────

    #[test]
    fn re_running_init_keeps_job_groups_extra_jobs_and_orbs() {
        use crate::orb_config::{ExtraJob, JobGroup};
        let mut orbs = IndexMap::new();
        orbs.insert("toolkit".to_string(), "org/toolkit@2.9.1".to_string());
        let existing = OrbConfig {
            orbs: Some(orbs.clone()),
            job_group: Some(vec![JobGroup {
                name: "sync_and_publish".to_string(),
                steps: vec!["prime".to_string()],
                ..JobGroup::default()
            }]),
            extra_job: Some(vec![ExtraJob {
                name: "smoke".to_string(),
                yaml: "  steps:\n    - checkout\n".to_string(),
            }]),
            ..OrbConfig::default()
        };
        let config = build_bootstrap_config(
            "mytool",
            &["my-org".to_string()],
            &gathered_orb(),
            &gathered_extras(),
            &existing,
            &[],
        );
        assert_eq!(config.orbs, Some(orbs));
        assert_eq!(
            config.job_group.as_ref().map(|g| g.len()),
            Some(1),
            "a hand-authored job group must survive a re-init"
        );
        assert_eq!(config.extra_job.as_ref().map(|j| j.len()), Some(1));
    }

    /// The advanced `[orb]` knobs are not in the dialogue, so nulling them had
    /// the same effect as nulling the sections above: a re-run deleted them.
    #[test]
    fn re_running_init_keeps_the_orb_settings_the_dialogue_never_asks_about() {
        let existing = OrbConfig {
            orb: Some(OrbSection {
                apt_packages: Some(vec!["gnupg".to_string()]),
                cargo_tools: Some(vec!["cargo-audit".to_string()]),
                custom_files: Some(vec!["src/scripts/build-container.sh".to_string()]),
                allow_unparsed_help: Some(true),
                crate_wait_attempts: 60,
                crate_wait_seconds: 20,
                ..OrbSection::default()
            }),
            ..OrbConfig::default()
        };
        let config = build_bootstrap_config(
            "mytool",
            &["my-org".to_string()],
            &gathered_orb(),
            &gathered_extras(),
            &existing,
            &[],
        );
        let orb = config.orb.as_ref().unwrap();
        assert_eq!(
            orb.apt_packages.as_deref(),
            Some(&["gnupg".to_string()][..])
        );
        assert_eq!(
            orb.cargo_tools.as_deref(),
            Some(&["cargo-audit".to_string()][..])
        );
        assert_eq!(
            orb.custom_files.as_deref(),
            Some(&["src/scripts/build-container.sh".to_string()][..])
        );
        assert_eq!(orb.allow_unparsed_help, Some(true));
        assert_eq!(orb.crate_wait_attempts, 60);
        assert_eq!(orb.crate_wait_seconds, 20);
    }

    // ── #271: the subcommand entries are merged, not rebuilt ───────────────

    /// The dialogue owns `interactive` and nothing else. A curated `label`, a
    /// `short_param` naming and a `param` override are all hand-authored, and
    /// rebuilding the map deleted them on every re-run.
    #[test]
    fn re_running_init_keeps_hand_authored_subcommand_settings() {
        use crate::orb_config::ParamOverride;
        let mut short_param = IndexMap::new();
        short_param.insert("f".to_string(), "force".to_string());
        let mut param = IndexMap::new();
        param.insert(
            "orb_path".to_string(),
            ParamOverride {
                default: Some("src/@orb.yml".to_string()),
            },
        );
        let mut recorded = IndexMap::new();
        recorded.insert(
            "generate".to_string(),
            SubcommandConfig {
                label: Some("Regenerate the orb source".to_string()),
                short_param: Some(short_param.clone()),
                param: Some(param.clone()),
                ..SubcommandConfig::default()
            },
        );
        let existing = OrbConfig {
            subcommand: Some(recorded),
            ..OrbConfig::default()
        };

        let config = build_bootstrap_config(
            "mytool",
            &["my-org".to_string()],
            &gathered_orb(),
            &gathered_extras(),
            &existing,
            &[("init".to_string(), true)],
        );

        let subcommands = config.subcommand.as_ref().expect("subcommand section lost");
        let generate = subcommands
            .get("generate")
            .expect("hand-authored entry deleted by the re-run");
        assert_eq!(generate.label.as_deref(), Some("Regenerate the orb source"));
        assert_eq!(generate.short_param, Some(short_param));
        assert_eq!(generate.param, Some(param));
        assert_eq!(
            subcommands.get("init").unwrap().interactive,
            Some(true),
            "the dialogue's own decision must still be applied"
        );
    }

    /// …and the dialogue's answer wins over a stale recorded one for the field
    /// it does own, so changing the answer on a re-run actually takes effect.
    #[test]
    fn the_dialogue_overrides_the_recorded_interactive_flag() {
        let mut recorded = IndexMap::new();
        recorded.insert(
            "config".to_string(),
            SubcommandConfig {
                interactive: Some(true),
                label: Some("Configure".to_string()),
                ..SubcommandConfig::default()
            },
        );
        let existing = OrbConfig {
            subcommand: Some(recorded),
            ..OrbConfig::default()
        };
        let config = build_bootstrap_config(
            "mytool",
            &["my-org".to_string()],
            &gathered_orb(),
            &gathered_extras(),
            &existing,
            &[("config".to_string(), false)],
        );
        let entry = config.subcommand.as_ref().unwrap().get("config").unwrap();
        assert_eq!(entry.interactive, Some(false), "the new answer must win");
        assert_eq!(
            entry.label.as_deref(),
            Some("Configure"),
            "the fields the dialogue does not own are untouched"
        );
    }

    // ── #269: the dialogue offers the detected repository URL ──────────────

    #[test]
    fn init_offers_the_detected_repository_url_for_home_and_source() {
        let extras = make_init(true)
            .gather_extras(
                &[],
                Some("https://github.com/my-org/mytool"),
                &OrbConfig::default(),
                false,
            )
            .unwrap();
        assert_eq!(
            extras.home_url.as_deref(),
            Some("https://github.com/my-org/mytool")
        );
        assert_eq!(
            extras.source_url.as_deref(),
            Some("https://github.com/my-org/mytool")
        );
    }

    #[test]
    fn a_recorded_url_wins_over_the_detected_one() {
        let existing = OrbConfig {
            orb: Some(OrbSection {
                home_url: Some("https://example.com/home".to_string()),
                ..OrbSection::default()
            }),
            ..OrbConfig::default()
        };
        let extras = make_init(true)
            .gather_extras(
                &[],
                Some("https://github.com/my-org/mytool"),
                &existing,
                false,
            )
            .unwrap();
        assert_eq!(extras.home_url.as_deref(), Some("https://example.com/home"));
        assert_eq!(
            extras.source_url.as_deref(),
            Some("https://github.com/my-org/mytool"),
            "the unrecorded one still takes the detected value"
        );
    }

    /// An explicit empty flag still means "no value" — detection must not
    /// override an answer the caller gave.
    #[test]
    fn an_explicit_empty_url_flag_beats_detection() {
        let mut init = make_init(true);
        init.home_url = Some(String::new());
        let extras = init
            .gather_extras(
                &[],
                Some("https://github.com/my-org/mytool"),
                &OrbConfig::default(),
                false,
            )
            .unwrap();
        assert_eq!(extras.home_url, None);
    }
}
