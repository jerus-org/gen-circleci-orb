mod types;

pub use types::{
    CiSection, ExtraJob, JobGroup, JobGroupParam, JobGroupStep, OrbConfig, OrbSection,
    ParamOverride, RecordConfig, SubcommandConfig,
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
    fn load_config_parses_minimal_record_opt_out() {
        // The documented silence opt-out (#155) — a [record] section with only
        // `enabled = false` — must parse: the env-name fields default rather than
        // erroring on "missing field".
        let dir = TempDir::new().unwrap();
        let path = write_toml(
            &dir,
            r#"
[record]
enabled = false
"#,
        );
        let config = load_config(&path).unwrap();
        let record = config.record.expect("[record] must parse");
        assert!(!record.enabled);
        assert!(record.gpg_key_env.is_empty());
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
    fn load_config_parses_rich_job_group_with_parameters_and_steps() {
        let dir = TempDir::new().unwrap();
        let path = write_toml(
            &dir,
            r#"
[[job_group]]
name = "sync_and_publish"
description = "Prime, generate, publish and commit back."

[[job_group.parameter]]
name = "binary_name"
description = "Consumer binary name."

[[job_group.parameter]]
name = "tag_prefix"
type = "string"
default = "v"

[[job_group.step]]
builtin = "checkout"

[[job_group.step]]
command = "set_https_remote"

[[job_group.step]]
run = "Set up git and environment"
script = "git fetch origin main"
[job_group.step.environment]
TAG_PREFIX = "<< parameters.tag_prefix >>"

[[job_group.step]]
command = "generate"
[job_group.step.with]
format = "binary"
orb_path = "<< parameters.orb_path >>"

[[job_group.step]]
orb = "toolkit/setup"
"#,
        );
        let config = load_config(&path).unwrap();
        let groups = config.job_group.unwrap();
        assert_eq!(groups.len(), 1);
        let g = &groups[0];
        assert_eq!(g.name, "sync_and_publish");

        let params = g.parameter.as_ref().expect("parameters missing");
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "binary_name");
        assert_eq!(params[1].param_type.as_deref(), Some("string"));
        assert_eq!(params[1].default.as_deref(), Some("v"));

        let steps = g.step.as_ref().expect("steps missing");
        assert_eq!(steps.len(), 5);
        assert_eq!(steps[0].builtin.as_deref(), Some("checkout"));
        assert_eq!(steps[1].command.as_deref(), Some("set_https_remote"));
        assert_eq!(steps[2].run.as_deref(), Some("Set up git and environment"));
        assert_eq!(steps[2].script.as_deref(), Some("git fetch origin main"));
        assert_eq!(
            steps[2]
                .environment
                .as_ref()
                .and_then(|e| e.get("TAG_PREFIX"))
                .map(String::as_str),
            Some("<< parameters.tag_prefix >>")
        );
        assert_eq!(steps[3].command.as_deref(), Some("generate"));
        assert_eq!(
            steps[3]
                .with
                .as_ref()
                .and_then(|w| w.get("format"))
                .map(String::as_str),
            Some("binary")
        );
        assert_eq!(steps[4].orb.as_deref(), Some("toolkit/setup"));
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
                interactive: None,
                param: None,
                label: None,
            },
        );
        subcommands.insert(
            "generate".to_string(),
            SubcommandConfig {
                generate_job: None,
                interactive: None,
                param: Some(params),
                label: None,
            },
        );

        let original = OrbConfig {
            orb: Some(OrbSection {
                binary: Some("gen-orb-mcp".to_string()),
                namespaces: Some(vec!["jerus-org".to_string()]),
                orb_dir: Some("orb".to_string()),
                base_image: None,
                builder_image: None,
                circleci_cli_version: None,
                install_method: None,
                apt_packages: None,
                home_url: None,
                source_url: None,
                git_push_subcommands: None,
                custom_files: None,
            }),
            ci: None,
            orbs: None,
            subcommand: Some(subcommands),
            job_group: Some(vec![JobGroup {
                name: "sync".to_string(),
                description: Some("Sync".to_string()),
                steps: vec!["generate".to_string(), "validate".to_string()],
                params: None,
                ..Default::default()
            }]),
            extra_job: None,
            record: None,
        };

        save_config(&path, &original).unwrap();
        let loaded = load_config(&path).unwrap();
        assert_eq!(original, loaded);
    }
}
