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
// A minimal `[record]` section (e.g. only `enabled = false`, the documented
// silence opt-out) must parse: fall back to Default for any field the user omits
// rather than erroring on "missing field".
#[serde(default)]
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
    /// SSH key fingerprint (a public-key hash, **not** a secret) for the
    /// end-of-workflow push job. When set, the push job loads this write key and
    /// drops the read-only checkout key, giving the push write authority. Empty
    /// falls back to the ambient environment credentials (the push then fails on
    /// a read-only key, with guidance). Stored as a value, not an env-var name,
    /// because CircleCI's `add_ssh_keys` resolves fingerprints at config-compile
    /// time and cannot read environment variables.
    #[serde(default)]
    pub push_ssh_fingerprint: String,
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
    /// jerus-org/gen-orb-mcp orb version pinned for the build_mcp_server job.
    /// Overrides the generator default when set. Only used when `mcp` is true.
    pub gen_orb_mcp_orb_version: Option<String>,
    /// Docker image the `build_rust_binary` CI jobs (`build-binary`,
    /// `orb-release-binary`) compile in. This configures the CI pipeline — not the
    /// orb's own container (see `[orb].base_image`/`builder_image` for that). Unset
    /// falls back to the job default (`rust:latest`), which has no libclang; set a
    /// clang-equipped image (e.g. `jerusdp/ci-rust:rolling-6mo@sha256:…`) when the
    /// workspace pulls a bindgen-based `-sys` crate so bindgen can run.
    pub rust_image: Option<String>,
}

/// Default number of `cargo install` attempts in the generated Dockerfile's
/// crates.io propagation gate.
pub const DEFAULT_CRATE_WAIT_ATTEMPTS: u32 = 40;
/// Default seconds between those attempts. 40 x 15s is 39 sleeps, ~9m45s.
pub const DEFAULT_CRATE_WAIT_SECONDS: u32 = 15;
/// Ceiling on `crate_wait_attempts`.
///
/// Past some point a "window" is just a hang: the job sits until CircleCI's own
/// timeout kills it, which is a worse outcome than the loud failure the gate
/// exists to produce. 240 attempts is an hour at the default interval — far
/// beyond any propagation delay ever observed.
pub const MAX_CRATE_WAIT_ATTEMPTS: u32 = 240;

/// Directory the generated orb source is written to, relative to the repo root.
pub const DEFAULT_ORB_DIR: &str = "orb";

/// How the generated Dockerfile gets the binary into the image. One of
/// `binstall`, `apt`, `local` — see `commands::generate::InstallMethod`.
pub const DEFAULT_INSTALL_METHOD: &str = "binstall";

/// Runtime stage image for the generated Dockerfile.
pub const DEFAULT_BASE_IMAGE: &str = "debian:13-slim";

/// Runtime stage image when the MCP feature is enabled. `build_mcp_server`
/// compiles the MCP server at runtime via `gen-orb-mcp generate --format
/// binary`, so the executor needs a Rust toolchain (cargo) present.
pub const MCP_DEFAULT_BASE_IMAGE: &str = "rust:1-slim-trixie";

/// Image for the Rust `builder` stage (Binstall method) that `cargo install`s
/// the binary.
pub const DEFAULT_BUILDER_IMAGE: &str = "rust:1-slim-trixie";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct OrbSection {
    pub binary: Option<String>,
    pub namespaces: Option<Vec<String>>,
    /// Directory the orb source is generated into, relative to the repo root.
    /// Carries [`DEFAULT_ORB_DIR`] rather than "unset" — see the note on
    /// [`crate_wait_attempts`](Self::crate_wait_attempts).
    pub orb_dir: String,
    /// Runtime stage image for the generated Dockerfile. Set a pinned
    /// `…@sha256:…` here so the digest survives regeneration (Renovate tracks
    /// it in this file). Defaults to [`DEFAULT_BASE_IMAGE`], corrected to
    /// [`MCP_DEFAULT_BASE_IMAGE`] when `[ci] mcp` is on and this still carries
    /// the plain default — `build_mcp_server` needs cargo in the executor.
    pub base_image: String,
    /// Image for the Rust `builder` stage (Binstall method) that `cargo install`s
    /// the binary. Set a pinned `rust:…@sha256:…` here so the digest survives
    /// regeneration (Renovate tracks it in this file). Defaults to
    /// [`DEFAULT_BUILDER_IMAGE`].
    pub builder_image: String,
    /// circleci-cli version to install into the generated Docker image. This is a
    /// version *override* only — it does not control *whether* the CLI is
    /// installed. Binaries that intrinsically need the CLI (those exposing a
    /// circleci-using subcommand, e.g. `ensure-orb-registered`) always get it, at
    /// `DEFAULT_CIRCLECI_CLI_VERSION` when this is unset — so removing this key
    /// cannot silently drop the CLI. Set it (Renovate-tracked) to pin/update the
    /// version, or to opt a consumer's own binary into bundling the CLI.
    ///
    /// Deliberately still `Option` while the settings around it carry real
    /// defaults: because a recorded version is *also* an opt-in, materialising
    /// one would bundle the CLI into every executor that never asked for it.
    /// `init` records it only for a binary that intrinsically needs the CLI.
    pub circleci_cli_version: Option<String>,
    /// How the generated Dockerfile obtains the binary: `binstall`, `apt` or
    /// `local`. Defaults to [`DEFAULT_INSTALL_METHOD`]; an unrecognised value
    /// is warned about and treated as `binstall`.
    pub install_method: String,
    /// Extra apt packages installed in the generated Docker image's runtime stage.
    /// Use for tools that need build dependencies at runtime (e.g. a tool that
    /// compiles code via `cargo` needs libssl-dev + pkg-config).
    pub apt_packages: Option<Vec<String>>,
    /// Extra cargo tools to install into the executor image (Binstall method
    /// only). Each entry is a crate name; the generated Dockerfile installs
    /// `cargo-binstall` in the builder stage, `cargo binstall`s these crates, and
    /// copies their binaries into the runtime stage. Use for orbs whose executor
    /// orchestrates other cargo tools (e.g. a security gate needs `cargo-audit`
    /// and `cargo-deny` on PATH). The installed binaries are available under their
    /// crate name (e.g. `cargo-audit`).
    pub cargo_tools: Option<Vec<String>>,
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
    /// Generate even when the binary's `--help` declares an option or argument
    /// the parser could not turn into an orb parameter.
    ///
    /// Generation fails on such a declaration by default: the alternative is a
    /// job that silently cannot supply an input the CLI requires (#240/#241/#242).
    /// Set this to `true` to ship in spite of the gap — the missing input is
    /// then logged as a warning — while the generator is taught the shape.
    pub allow_unparsed_help: Option<bool>,
    /// How many times the generated Dockerfile retries `cargo install` while
    /// waiting for crates.io to serve the version being released.
    ///
    /// The container is built from the crate that was just published, so it
    /// races the sparse index. When the gate expires the release stalls
    /// half-published — crate on crates.io, no container, no orb — so raise
    /// this (or `crate_wait_seconds`) if a release keeps outrunning it.
    ///
    /// Unlike the `Option` fields around it this carries a real default rather
    /// than "unset", so a saved config states the value in force and gives the
    /// consumer the knob already written out. Floored at 1 and capped at
    /// [`MAX_CRATE_WAIT_ATTEMPTS`].
    pub crate_wait_attempts: u32,
    /// Seconds between those retries. Floored at 1 — a zero would spin.
    pub crate_wait_seconds: u32,
}

/// Hand-written because `derive(Default)` would give the crate-wait settings
/// `u32::default()` — zero — which as an attempt count means "never wait" and
/// as an interval means "spin", and the image settings `String::default()` —
/// empty — which is an unbuildable `FROM`. The defaults are the constants
/// above, and this impl is what `#[serde(default)]` uses for a config that
/// omits them.
impl Default for OrbSection {
    fn default() -> Self {
        Self {
            binary: None,
            namespaces: None,
            orb_dir: DEFAULT_ORB_DIR.to_string(),
            base_image: DEFAULT_BASE_IMAGE.to_string(),
            builder_image: DEFAULT_BUILDER_IMAGE.to_string(),
            circleci_cli_version: None,
            install_method: DEFAULT_INSTALL_METHOD.to_string(),
            apt_packages: None,
            cargo_tools: None,
            home_url: None,
            source_url: None,
            git_push_subcommands: None,
            custom_files: None,
            allow_unparsed_help: None,
            crate_wait_attempts: DEFAULT_CRATE_WAIT_ATTEMPTS,
            crate_wait_seconds: DEFAULT_CRATE_WAIT_SECONDS,
        }
    }
}

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
pub struct SubcommandConfig {
    pub generate_job: Option<bool>,
    /// When true, the subcommand is reserved as interactive/CLI-only and is FULLY
    /// excluded from orb generation — no job, no command, no script (and, for a
    /// parent, its whole subtree). Stronger than `generate_job = false`, which
    /// drops only the job. Unset falls back to a built-in default: `init` and
    /// `config` are interactive by default; set `false` to expose them in CI.
    pub interactive: Option<bool>,
    pub param: Option<IndexMap<String, ParamOverride>>,
    /// Names for the subcommand's short-only options (`-f` with no `--force`),
    /// keyed by the short flag: `[subcommand.check.short_param] f = "force"`.
    ///
    /// A short-only option has no long form to name its orb parameter after.
    /// The generator derives one from the first word of the description where
    /// that works; set an entry here to choose the name, or to supply one where
    /// nothing usable can be derived (generation fails until you do).
    pub short_param: Option<IndexMap<String, String>>,
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
///   `sync_and_publish`.
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
