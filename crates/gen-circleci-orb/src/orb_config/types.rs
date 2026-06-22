use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
pub struct OrbConfig {
    pub orb: Option<OrbSection>,
    pub ci: Option<CiSection>,
    pub orbs: Option<IndexMap<String, String>>,
    pub subcommand: Option<IndexMap<String, SubcommandConfig>>,
    pub job_group: Option<Vec<JobGroup>>,
    pub extra_job: Option<Vec<ExtraJob>>,
    pub record: Option<RecordConfig>,
}

/// Auto-record configuration: after `generate`, commit the regenerated orb
/// source back (GPG-signed) and push it **to the PR branch** so the change is
/// reviewable, keeping the published orb in sync with the CLI without the dev
/// maintaining the CLI locally. The fields name the **environment variables**
/// that hold the GPG signing material at runtime — the names are the consumer's
/// choice (no defaults), so the tool never dictates an env-var convention. Only
/// the names are stored here (committed); the secret values live in the CI
/// context(s).
///
/// The push uses **ambient authorization** — pcu's `Client::new_local()` carries
/// no credentials of its own and relies on whatever auth `checkout` left in the
/// environment. In a standard CircleCI + GitHub checkout that is the read-only
/// deploy key, so the push fails (by design, with guidance): the recommended
/// setup is a single user-supplied end-of-workflow push job. The tool therefore
/// holds no write token — only the GPG signing names needed to sign the commit.
#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
pub struct RecordConfig {
    /// Whether auto-record is enabled.
    pub enabled: bool,
    /// Name of the env var holding the base64-encoded GPG private key.
    pub gpg_key_env: String,
    /// Name of the env var holding the GPG ownertrust export.
    pub gpg_trust_env: String,
    /// Name of the env var holding the committer name.
    pub user_name_env: String,
    /// Name of the env var holding the committer email.
    pub user_email_env: String,
    /// Name of the env var holding the GPG signing key id.
    pub signing_key_env: String,
    /// CircleCI context(s) that supply the values for the env vars named above.
    /// The record CI job attaches these so the signing material is available at
    /// runtime.
    pub contexts: Vec<String>,
}

/// CI pipeline values gathered at `init` time, stored so future re-runs
/// can reproduce the same CI config without re-supplying every flag.
#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
pub struct CiSection {
    pub build_workflow: Option<String>,
    pub release_workflow: Option<String>,
    pub requires_job: Option<String>,
    pub release_after_job: Option<String>,
    pub crate_tag_prefix: Option<String>,
    pub docker_namespace: Option<String>,
    pub docker_context: Option<String>,
    pub orb_context: Option<String>,
    pub mcp: Option<bool>,
    pub mcp_context: Option<Vec<String>>,
    pub mcp_earliest_version: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
pub struct OrbSection {
    pub binary: Option<String>,
    pub namespaces: Option<Vec<String>>,
    pub orb_dir: Option<String>,
    pub base_image: Option<String>,
    /// Image for the Rust `builder` stage (Binstall method) that `cargo install`s
    /// the binary. Set a pinned `rust:…@sha256:…` here so the digest survives
    /// regeneration (Renovate tracks it in this file). Defaults to `rust:1-slim-trixie`.
    pub builder_image: Option<String>,
    pub install_method: Option<String>,
    /// Extra apt packages installed in the generated Docker image's runtime stage.
    /// Use for tools that need build dependencies at runtime (e.g. a tool that
    /// compiles code via `cargo` needs libssl-dev + pkg-config).
    pub apt_packages: Option<Vec<String>>,
    pub home_url: Option<String>,
    pub source_url: Option<String>,
    /// Subcommand names whose generated jobs include a set_https_remote step.
    /// Persisted here so `generate` can reproduce the same output without re-supplying the flag.
    pub git_push_subcommands: Option<Vec<String>>,
    /// Hand-authored orb files the generator does **not** produce but that must be
    /// kept — paths relative to the orb root (e.g. `src/commands/build_container.yml`,
    /// `src/jobs/build_container.yml`, `src/scripts/build-container.sh`). These
    /// "authorise" custom commands/jobs/scripts so the prune step preserves them
    /// instead of deleting them as orphans. The orb tree is otherwise treated as
    /// fully owned by (CLI subcommands ∪ config): anything in the generated dirs
    /// that is neither generated nor listed here is pruned.
    pub custom_files: Option<Vec<String>>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
pub struct SubcommandConfig {
    pub generate_job: Option<bool>,
    pub param: Option<IndexMap<String, ParamOverride>>,
    /// Curated display name for the command's `run` step. When unset, the
    /// generator falls back to the command's short about (the first sentence
    /// of its `--help`), then to the bare subcommand name.
    pub label: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
pub struct ParamOverride {
    pub default: Option<String>,
}

/// A composed job assembled from the tool's own generated commands plus optional
/// custom `run` steps and third-party orb steps.
///
/// Two authoring modes are supported:
///
/// * **Simple** — set [`steps`](Self::steps) to a list of command names. Each is
///   invoked in sequence with its parameters wired through from auto-detected
///   shared job parameters. This is what `config add-job-group` writes.
/// * **Rich** — set [`step`](Self::step) to an ordered list of [`JobGroupStep`]
///   values (built-ins, tool commands with explicit values, third-party orb
///   steps, custom `run` steps) and declare job parameters explicitly via
///   [`parameter`](Self::parameter). Use this to build goal-oriented jobs such as
///   `build_mcp_server`.
///
/// When [`step`](Self::step) is present it takes precedence over
/// [`steps`](Self::steps).
#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
pub struct JobGroup {
    pub name: String,
    pub description: Option<String>,
    /// Simple-mode command sequence (command names).
    #[serde(default)]
    pub steps: Vec<String>,
    /// Simple-mode explicit parameter selection (omit for auto-detected shared params).
    pub params: Option<Vec<String>>,
    /// Executor to run the job in (defaults to `default`).
    pub executor: Option<String>,
    /// Rich-mode explicit job parameter declarations.
    pub parameter: Option<Vec<JobGroupParam>>,
    /// Rich-mode ordered step list. Takes precedence over [`steps`](Self::steps).
    pub step: Option<Vec<JobGroupStep>>,
}

/// An explicitly declared job-level parameter for a rich-mode [`JobGroup`].
#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
pub struct JobGroupParam {
    pub name: String,
    /// CircleCI parameter type (default: `string`).
    #[serde(rename = "type")]
    pub param_type: Option<String>,
    pub default: Option<String>,
    pub description: Option<String>,
}

/// A single step in a rich-mode [`JobGroup`].
///
/// Exactly one of the discriminant fields should be set:
///
/// * [`builtin`](Self::builtin) — a built-in step (`checkout`, `attach_workspace`).
/// * [`command`](Self::command) — invoke a tool-generated orb command (e.g. `prime`),
///   or the synthetic `set_https_remote` command. Parameter values come from
///   [`with`](Self::with) (literals or `<< parameters.x >>` references); when
///   omitted the command's parameters are wired through by name.
/// * [`orb`](Self::orb) — invoke a third-party orb command (e.g. `toolkit/setup`),
///   with values from [`with`](Self::with).
/// * [`run`](Self::run) — a custom `run` step whose name is this field's value,
///   shell body is [`script`](Self::script), and env block is
///   [`environment`](Self::environment).
#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
pub struct JobGroupStep {
    pub builtin: Option<String>,
    pub command: Option<String>,
    pub orb: Option<String>,
    pub run: Option<String>,
    pub script: Option<String>,
    pub with: Option<IndexMap<String, String>>,
    pub environment: Option<IndexMap<String, String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ExtraJob {
    pub name: String,
    pub yaml: String,
}
