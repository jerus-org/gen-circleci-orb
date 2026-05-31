mod types;

pub use types::{
    CiSection, ExtraJob, JobGroup, OrbConfig, OrbSection, ParamOverride, SubcommandConfig,
};

use anyhow::Result;
use std::path::Path;

pub fn load_config(path: &Path) -> Result<OrbConfig> {
    if !path.exists() {
        return Ok(OrbConfig::default());
    }
    let content = std::fs::read_to_string(path)?;
    let config: OrbConfig = toml::from_str(&content)?;
    Ok(config)
}

pub fn save_config(path: &Path, config: &OrbConfig) -> Result<()> {
    let content = toml::to_string_pretty(config)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;
    use tempfile::TempDir;

    fn write_toml(dir: &TempDir, content: &str) -> std::path::PathBuf {
        let path = dir.path().join("gen-circleci-orb.toml");
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn load_config_parses_git_push_subcommands() {
        let dir = TempDir::new().unwrap();
        let path = write_toml(
            &dir,
            r#"
[orb]
binary = "mytool"
git_push_subcommands = ["save"]
"#,
        );
        let config = load_config(&path).unwrap();
        let orb = config.orb.as_ref().unwrap();
        assert_eq!(
            orb.git_push_subcommands.as_deref(),
            Some(&["save".to_string()][..])
        );
    }

    #[test]
    fn load_config_parses_ci_section() {
        let dir = TempDir::new().unwrap();
        let path = write_toml(
            &dir,
            r#"
[ci]
build_workflow = "validation"
release_workflow = "orb-release"
requires_job = "toolkit/common_tests"
release_after_job = "publish-orb-jerus-org"
crate_tag_prefix = "mytool-v"
docker_namespace = "myns"
docker_context = "docker"
orb_context = "orb-publishing"
mcp = true
mcp_context = ["release", "bot-check", "pcu-app"]
mcp_earliest_version = "0.1.0"
"#,
        );
        let config = load_config(&path).unwrap();
        let ci = config.ci.as_ref().expect("ci section missing");
        assert_eq!(ci.build_workflow.as_deref(), Some("validation"));
        assert_eq!(ci.docker_context.as_deref(), Some("docker"));
        assert_eq!(ci.mcp, Some(true));
        assert_eq!(ci.mcp_earliest_version.as_deref(), Some("0.1.0"));
        assert_eq!(
            ci.mcp_context.as_deref(),
            Some(
                &[
                    "release".to_string(),
                    "bot-check".to_string(),
                    "pcu-app".to_string()
                ][..]
            )
        );
    }

    #[test]
    fn load_config_returns_default_when_file_not_found() {
        let path = std::path::Path::new("/tmp/does-not-exist/gen-circleci-orb.toml");
        let config = load_config(path).unwrap();
        assert_eq!(config, OrbConfig::default());
    }

    #[test]
    fn load_config_parses_suppression_entry() {
        let dir = TempDir::new().unwrap();
        let path = write_toml(
            &dir,
            r#"
[subcommand.help]
generate_job = false
"#,
        );
        let config = load_config(&path).unwrap();
        let subcommands = config.subcommand.unwrap();
        let help = subcommands.get("help").unwrap();
        assert_eq!(help.generate_job, Some(false));
    }

    #[test]
    fn load_config_parses_param_override() {
        let dir = TempDir::new().unwrap();
        let path = write_toml(
            &dir,
            r#"
[subcommand.generate.param.orb_path]
default = "src/@orb.yml"
"#,
        );
        let config = load_config(&path).unwrap();
        let subcommands = config.subcommand.unwrap();
        let gen = subcommands.get("generate").unwrap();
        let params = gen.param.as_ref().unwrap();
        let override_ = params.get("orb_path").unwrap();
        assert_eq!(override_.default.as_deref(), Some("src/@orb.yml"));
    }

    #[test]
    fn load_config_parses_job_group_with_steps() {
        let dir = TempDir::new().unwrap();
        let path = write_toml(
            &dir,
            r#"
[[job_group]]
name = "sync"
description = "Regenerate and validate"
steps = ["generate", "validate"]
"#,
        );
        let config = load_config(&path).unwrap();
        let groups = config.job_group.unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "sync");
        assert_eq!(groups[0].steps, vec!["generate", "validate"]);
        assert_eq!(
            groups[0].description.as_deref(),
            Some("Regenerate and validate")
        );
        assert!(groups[0].params.is_none());
    }

    #[test]
    fn load_config_parses_job_group_with_explicit_params() {
        let dir = TempDir::new().unwrap();
        let path = write_toml(
            &dir,
            r#"
[[job_group]]
name = "build"
steps = ["generate", "validate"]
params = ["orb_path", "binary"]
"#,
        );
        let config = load_config(&path).unwrap();
        let groups = config.job_group.unwrap();
        let expected: Vec<String> = vec!["orb_path".to_string(), "binary".to_string()];
        assert_eq!(groups[0].params.as_deref(), Some(expected.as_slice()));
    }

    #[test]
    fn load_config_parses_extra_job_verbatim_yaml() {
        let dir = TempDir::new().unwrap();
        let path = write_toml(
            &dir,
            r#"
[[extra_job]]
name = "ensure_registered"
yaml = """
description: Ensure registered
executor: orb-tools/default
steps:
  - run:
      name: Check
      command: echo ok
"""
"#,
        );
        let config = load_config(&path).unwrap();
        let extra = config.extra_job.unwrap();
        assert_eq!(extra[0].name, "ensure_registered");
        assert!(extra[0].yaml.contains("description: Ensure registered"));
        assert!(extra[0].yaml.contains("executor: orb-tools/default"));
    }

    #[test]
    fn load_config_parses_orbs_section() {
        let dir = TempDir::new().unwrap();
        let path = write_toml(
            &dir,
            r#"
[orbs]
"orb-tools" = "circleci/orb-tools@12.3.3"
"#,
        );
        let config = load_config(&path).unwrap();
        let orbs = config.orbs.unwrap();
        assert_eq!(
            orbs.get("orb-tools").map(String::as_str),
            Some("circleci/orb-tools@12.3.3")
        );
    }

    #[test]
    fn save_config_round_trips() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");

        let mut subcommands = IndexMap::new();
        let mut params = IndexMap::new();
        params.insert(
            "orb_path".to_string(),
            ParamOverride {
                default: Some("src/@orb.yml".to_string()),
            },
        );
        subcommands.insert(
            "help".to_string(),
            SubcommandConfig {
                generate_job: Some(false),
                param: None,
            },
        );
        subcommands.insert(
            "generate".to_string(),
            SubcommandConfig {
                generate_job: None,
                param: Some(params),
            },
        );

        let original = OrbConfig {
            orb: Some(OrbSection {
                binary: Some("gen-orb-mcp".to_string()),
                namespaces: Some(vec!["jerus-org".to_string()]),
                orb_dir: Some("orb".to_string()),
                base_image: None,
                install_method: None,
                home_url: None,
                source_url: None,
                git_push_subcommands: None,
            }),
            ci: None,
            orbs: None,
            subcommand: Some(subcommands),
            job_group: Some(vec![JobGroup {
                name: "sync".to_string(),
                description: Some("Sync".to_string()),
                steps: vec!["generate".to_string(), "validate".to_string()],
                params: None,
            }]),
            extra_job: None,
        };

        save_config(&path, &original).unwrap();
        let loaded = load_config(&path).unwrap();
        assert_eq!(original, loaded);
    }
}
