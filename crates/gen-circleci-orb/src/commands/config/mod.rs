use anyhow::Result;
use indexmap::IndexMap;
use std::path::PathBuf;

use crate::orb_config::{self, JobGroup, OrbConfig, ParamOverride};

const DEFAULT_CONFIG_PATH: &str = "gen-circleci-orb.toml";

/// Manage the gen-circleci-orb.toml configuration file.
#[derive(Debug, clap::Args)]
pub struct Config {
    /// Path to gen-circleci-orb.toml (default: ./gen-circleci-orb.toml).
    #[arg(long, default_value = DEFAULT_CONFIG_PATH)]
    pub config: PathBuf,

    #[command(subcommand)]
    pub command: ConfigCommands,
}

#[derive(Debug, clap::Subcommand)]
pub enum ConfigCommands {
    /// Print the current configuration.
    Show,
    /// Suppress job generation for a subcommand (generate_job = false).
    Suppress {
        /// Subcommand name to suppress.
        subcommand: String,
    },
    /// Re-enable job generation for a previously suppressed subcommand.
    Unsuppress {
        /// Subcommand name to unsuppress.
        subcommand: String,
    },
    /// Append a composed job group.
    AddJobGroup {
        /// Name of the new job group.
        #[arg(long)]
        name: String,
        /// Comma-separated list of subcommand step names.
        #[arg(long, value_delimiter = ',')]
        steps: Vec<String>,
        /// Optional description for the job group.
        #[arg(long)]
        description: Option<String>,
        /// Comma-separated explicit parameter list (omit for auto-detected shared params).
        #[arg(long, value_delimiter = ',')]
        params: Option<Vec<String>>,
    },
    /// Set or update a parameter default override for a subcommand.
    SetDefault {
        /// Subcommand name.
        #[arg(long)]
        subcommand: String,
        /// Parameter name.
        #[arg(long)]
        param: String,
        /// New default value.
        #[arg(long)]
        default: String,
    },
}

impl Config {
    pub fn run(&self) -> Result<()> {
        match &self.command {
            ConfigCommands::Show => {
                let config = orb_config::load_config(&self.config)?;
                if config == OrbConfig::default() && !self.config.exists() {
                    println!("No config file found at {}", self.config.display());
                } else {
                    print!("{}", toml::to_string_pretty(&config)?);
                }
            }
            ConfigCommands::Suppress { subcommand } => {
                let mut config = orb_config::load_config(&self.config)?;
                suppress_subcommand(&mut config, subcommand);
                orb_config::save_config(&self.config, &config)?;
                println!("Suppressed job generation for '{subcommand}'");
            }
            ConfigCommands::Unsuppress { subcommand } => {
                let mut config = orb_config::load_config(&self.config)?;
                unsuppress_subcommand(&mut config, subcommand);
                orb_config::save_config(&self.config, &config)?;
                println!("Re-enabled job generation for '{subcommand}'");
            }
            ConfigCommands::AddJobGroup {
                name,
                steps,
                description,
                params,
            } => {
                let mut config = orb_config::load_config(&self.config)?;
                add_job_group(
                    &mut config,
                    JobGroup {
                        name: name.clone(),
                        description: description.clone(),
                        steps: steps.clone(),
                        params: params.clone(),
                    },
                );
                orb_config::save_config(&self.config, &config)?;
                println!("Added job group '{name}'");
            }
            ConfigCommands::SetDefault {
                subcommand,
                param,
                default,
            } => {
                let mut config = orb_config::load_config(&self.config)?;
                set_param_default(&mut config, subcommand, param, default);
                orb_config::save_config(&self.config, &config)?;
                println!("Set default for '{subcommand}.{param}' = '{default}'");
            }
        }
        Ok(())
    }
}

pub(crate) fn suppress_subcommand(config: &mut OrbConfig, name: &str) {
    let subcommands = config.subcommand.get_or_insert_with(IndexMap::new);
    let entry = subcommands.entry(name.to_string()).or_default();
    entry.generate_job = Some(false);
}

pub(crate) fn unsuppress_subcommand(config: &mut OrbConfig, name: &str) {
    if let Some(subcommands) = config.subcommand.as_mut() {
        if let Some(entry) = subcommands.get_mut(name) {
            entry.generate_job = None;
            // Remove the entry entirely if it's now empty
            if entry.generate_job.is_none() && entry.param.is_none() {
                subcommands.shift_remove(name);
            }
        }
    }
}

pub(crate) fn add_job_group(config: &mut OrbConfig, group: JobGroup) {
    config.job_group.get_or_insert_with(Vec::new).push(group);
}

pub(crate) fn set_param_default(
    config: &mut OrbConfig,
    subcommand: &str,
    param: &str,
    default: &str,
) {
    let subcommands = config.subcommand.get_or_insert_with(IndexMap::new);
    let sc = subcommands.entry(subcommand.to_string()).or_default();
    let params = sc.param.get_or_insert_with(IndexMap::new);
    params.insert(
        param.to_string(),
        ParamOverride {
            default: Some(default.to_string()),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_config() -> OrbConfig {
        OrbConfig::default()
    }

    #[test]
    fn suppress_sets_generate_job_false() {
        let mut config = empty_config();
        suppress_subcommand(&mut config, "help");
        let sc = config.subcommand.as_ref().unwrap().get("help").unwrap();
        assert_eq!(sc.generate_job, Some(false));
    }

    #[test]
    fn suppress_is_idempotent() {
        let mut config = empty_config();
        suppress_subcommand(&mut config, "help");
        suppress_subcommand(&mut config, "help");
        let count = config.subcommand.as_ref().unwrap().len();
        assert_eq!(count, 1, "second suppress must not add duplicate entry");
    }

    #[test]
    fn unsuppress_removes_generate_job_entry() {
        let mut config = empty_config();
        suppress_subcommand(&mut config, "help");
        unsuppress_subcommand(&mut config, "help");
        assert!(
            config
                .subcommand
                .as_ref()
                .map(|sc| sc.get("help").is_none())
                .unwrap_or(true),
            "unsuppress must remove the empty subcommand entry"
        );
    }

    #[test]
    fn unsuppress_preserves_other_entries() {
        let mut config = empty_config();
        suppress_subcommand(&mut config, "help");
        suppress_subcommand(&mut config, "validate");
        unsuppress_subcommand(&mut config, "help");
        let sc = config.subcommand.as_ref().unwrap();
        assert!(sc.get("help").is_none(), "help must be removed");
        assert!(sc.get("validate").is_some(), "validate must remain");
    }

    #[test]
    fn add_job_group_appends_to_list() {
        let mut config = empty_config();
        add_job_group(
            &mut config,
            JobGroup {
                name: "sync".to_string(),
                description: Some("Sync".to_string()),
                steps: vec!["generate".to_string(), "validate".to_string()],
                params: None,
            },
        );
        let groups = config.job_group.as_ref().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "sync");
    }

    #[test]
    fn add_job_group_multiple_times_appends_all() {
        let mut config = empty_config();
        add_job_group(
            &mut config,
            JobGroup {
                name: "sync".to_string(),
                description: None,
                steps: vec!["generate".to_string()],
                params: None,
            },
        );
        add_job_group(
            &mut config,
            JobGroup {
                name: "full".to_string(),
                description: None,
                steps: vec!["generate".to_string(), "validate".to_string()],
                params: None,
            },
        );
        let groups = config.job_group.as_ref().unwrap();
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn set_param_default_upserts_entry() {
        let mut config = empty_config();
        set_param_default(&mut config, "generate", "orb_path", "src/@orb.yml");
        let sc = config.subcommand.as_ref().unwrap().get("generate").unwrap();
        let override_ = sc.param.as_ref().unwrap().get("orb_path").unwrap();
        assert_eq!(override_.default.as_deref(), Some("src/@orb.yml"));
    }

    #[test]
    fn set_param_default_updates_existing_entry() {
        let mut config = empty_config();
        set_param_default(&mut config, "generate", "orb_path", "old/path");
        set_param_default(&mut config, "generate", "orb_path", "new/path");
        let sc = config.subcommand.as_ref().unwrap().get("generate").unwrap();
        let override_ = sc.param.as_ref().unwrap().get("orb_path").unwrap();
        assert_eq!(override_.default.as_deref(), Some("new/path"));
    }
}
