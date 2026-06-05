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
}

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
pub struct SubcommandConfig {
    pub generate_job: Option<bool>,
    pub param: Option<IndexMap<String, ParamOverride>>,
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
