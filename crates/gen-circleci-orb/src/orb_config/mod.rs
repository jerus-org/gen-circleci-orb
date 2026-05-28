mod types;

pub use types::{ExtraJob, JobGroup, OrbConfig, OrbSection, ParamOverride, SubcommandConfig};

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
            }),
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
