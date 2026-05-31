use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
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
#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
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

#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct OrbSection {
    pub binary: Option<String>,
    pub namespaces: Option<Vec<String>>,
    pub orb_dir: Option<String>,
    pub base_image: Option<String>,
    pub install_method: Option<String>,
    pub home_url: Option<String>,
    pub source_url: Option<String>,
    /// Subcommand names whose generated jobs include a set_https_remote step.
    /// Persisted here so `generate` can reproduce the same output without re-supplying the flag.
    pub git_push_subcommands: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct SubcommandConfig {
    pub generate_job: Option<bool>,
    pub param: Option<IndexMap<String, ParamOverride>>,
}

#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct ParamOverride {
    pub default: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct JobGroup {
    pub name: String,
    pub description: Option<String>,
    pub steps: Vec<String>,
    pub params: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct ExtraJob {
    pub name: String,
    pub yaml: String,
}
