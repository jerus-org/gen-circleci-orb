use indexmap::IndexMap;
use serde::Serialize;

/// Mirrors the unpacked CircleCI orb `@orb.yml` (metadata only).
#[derive(Debug, Clone, Serialize)]
pub struct OrbRoot {
    pub version: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<DisplayInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DisplayInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
}

/// A single orb command definition (`src/commands/<name>.yml`).
#[derive(Debug, Clone, Serialize)]
pub struct OrbCommand {
    pub description: String,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub parameters: IndexMap<String, OrbParameter>,
    pub steps: Vec<serde_yaml::Value>,
}

/// A single orb job definition (`src/jobs/<name>.yml`).
#[derive(Debug, Clone, Serialize)]
pub struct OrbJob {
    pub description: String,
    pub executor: String,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub parameters: IndexMap<String, OrbParameter>,
    pub steps: Vec<serde_yaml::Value>,
}

/// A single orb executor definition (`src/executors/<name>.yml`).
#[derive(Debug, Clone, Serialize)]
pub struct OrbExecutor {
    pub description: String,
    pub docker: Vec<DockerImage>,
    pub parameters: IndexMap<String, OrbParameter>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DockerImage {
    pub image: String,
}

/// An orb parameter definition.
#[derive(Debug, Clone, Serialize)]
pub struct OrbParameter {
    #[serde(rename = "type")]
    pub param_type: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_yaml::Value>,
    #[serde(rename = "enum", skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<String>>,
}
