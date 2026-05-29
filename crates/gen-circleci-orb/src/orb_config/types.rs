use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct OrbConfig {
    pub orb: Option<OrbSection>,
    pub orbs: Option<IndexMap<String, String>>,
    pub subcommand: Option<IndexMap<String, SubcommandConfig>>,
    pub job_group: Option<Vec<JobGroup>>,
    pub extra_job: Option<Vec<ExtraJob>>,
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
