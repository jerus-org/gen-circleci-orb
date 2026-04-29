use super::types::{DockerImage, OrbCommand, OrbExecutor, OrbJob, OrbParameter};
use crate::commands::generate::InstallMethod;
use crate::help_parser::types::{CliDefinition, ParamType, SubCommand};
use indexmap::IndexMap;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct GenerateOpts {
    pub namespaces: Vec<String>,
    pub install_method: InstallMethod,
    pub base_image: String,
    pub home_url: Option<String>,
    pub source_url: Option<String>,
    /// Binary name included in generated run-step commands.
    pub binary_name: String,
}

/// Generate all orb artifact strings keyed by their relative output path.
pub fn generate(cli: &CliDefinition, opts: &GenerateOpts) -> HashMap<PathBuf, String> {
    let mut files = HashMap::new();

    // @orb.yml — metadata only; hand-formatted so `version: 2.1` stays unquoted
    files.insert(PathBuf::from("src/@orb.yml"), render_orb_root(cli, opts));

    // executors/default.yml
    files.insert(
        PathBuf::from("src/executors/default.yml"),
        render_executor(&cli.binary_name),
    );

    // commands/<name>.yml and jobs/<name>.yml for each leaf subcommand
    for sub in &cli.subcommands {
        render_subcommand(sub, &cli.binary_name, &mut files);
    }

    // Dockerfile
    files.insert(
        PathBuf::from("Dockerfile"),
        render_dockerfile(&cli.binary_name, &opts.install_method, &opts.base_image),
    );

    files
}

fn render_orb_root(cli: &CliDefinition, opts: &GenerateOpts) -> String {
    // version must be the YAML float 2.1, not a quoted string
    let mut out = format!("version: 2.1\ndescription: >\n  {}\n", cli.description);
    if opts.home_url.is_some() || opts.source_url.is_some() {
        out.push_str("display:\n");
        if let Some(url) = &opts.home_url {
            out.push_str(&format!("  home_url: \"{url}\"\n"));
        }
        if let Some(url) = &opts.source_url {
            out.push_str(&format!("  source_url: \"{url}\"\n"));
        }
    }
    out
}

fn render_subcommand(sub: &SubCommand, binary: &str, files: &mut HashMap<PathBuf, String>) {
    if sub.is_leaf {
        files.insert(
            PathBuf::from(format!("src/commands/{}.yml", sub.name)),
            render_command(sub, binary),
        );
        files.insert(
            PathBuf::from(format!("src/jobs/{}.yml", sub.name)),
            render_job(sub),
        );
    }
    for child in &sub.subcommands {
        render_subcommand(child, binary, files);
    }
}

fn render_command(sub: &SubCommand, binary: &str) -> String {
    let parameters = build_orb_parameters(sub);
    let step = build_run_step(sub, binary);
    let cmd = OrbCommand {
        description: sub.description.clone(),
        parameters,
        steps: vec![step],
    };
    serde_yaml::to_string(&cmd).unwrap()
}

fn render_job(sub: &SubCommand) -> String {
    let parameters = build_orb_parameters(sub);
    let checkout_step: serde_yaml::Value = serde_yaml::Value::String("checkout".to_string());
    let invoke_step = build_invoke_step(sub);
    let job = OrbJob {
        description: format!("Run {} {} in a dedicated job.", sub.name, "command"),
        executor: "default".to_string(),
        parameters,
        steps: vec![checkout_step, invoke_step],
    };
    serde_yaml::to_string(&job).unwrap()
}

fn render_executor(binary_name: &str) -> String {
    let mut params = IndexMap::new();
    params.insert(
        "tag".to_string(),
        OrbParameter {
            param_type: "string".to_string(),
            description: "Docker image tag.".to_string(),
            default: Some(serde_yaml::Value::String("latest".to_string())),
            enum_values: None,
        },
    );
    let executor = OrbExecutor {
        description: format!("Execution environment with {binary_name} pre-installed."),
        docker: vec![DockerImage {
            image: format!("jerusdp/{binary_name}:<< parameters.tag >>"),
        }],
        parameters: params,
    };
    serde_yaml::to_string(&executor).unwrap()
}

fn render_dockerfile(binary: &str, method: &InstallMethod, base_image: &str) -> String {
    let install_block = match method {
        InstallMethod::Binstall => format!(
            r#"RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && curl -L --proto '=https' --tlsv1.2 -sSf \
       https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash \
    && cargo-binstall --no-confirm {binary} \
    && rm -rf /root/.cargo/registry /root/.cargo/git"#
        ),
        InstallMethod::Apt => format!(
            "RUN apt-get update \\\n    && apt-get install -y --no-install-recommends {binary} \\\n    && rm -rf /var/lib/apt/lists/*"
        ),
    };
    format!("FROM {base_image}\n{install_block}\n")
}

fn build_orb_parameters(sub: &SubCommand) -> IndexMap<String, OrbParameter> {
    let mut params = IndexMap::new();
    for p in &sub.parameters {
        let (type_str, enum_vals) = match &p.param_type {
            ParamType::String => ("string".to_string(), None),
            ParamType::Boolean => ("boolean".to_string(), None),
            ParamType::Integer => ("integer".to_string(), None),
            ParamType::Enum(vals) => ("enum".to_string(), Some(vals.clone())),
        };
        let default = p.default.as_ref().map(|d| {
            if type_str == "boolean" {
                serde_yaml::Value::Bool(d == "true")
            } else {
                serde_yaml::Value::String(d.clone())
            }
        });
        params.insert(
            p.long_name.clone(),
            OrbParameter {
                param_type: type_str,
                description: p.description.clone(),
                default,
                enum_values: enum_vals,
            },
        );
    }
    params
}

/// Build the `run:` step for a command, interpolating all parameters.
fn build_run_step(sub: &SubCommand, binary: &str) -> serde_yaml::Value {
    let mut cmd_parts: Vec<String> = vec![format!("{} {}", binary, sub.name.replace('_', "-"))];

    for p in &sub.parameters {
        let flag = format!("--{}", p.long_name.replace('_', "-"));
        match &p.param_type {
            ParamType::Boolean => {
                // Mustache conditional: include flag only when param is truthy
                cmd_parts.push(format!(
                    "<<# parameters.{0} >>{1}<</ parameters.{0} >>",
                    p.long_name, flag
                ));
            }
            _ => {
                if p.required {
                    cmd_parts.push(format!("{flag} \"<< parameters.{} >>\"", p.long_name));
                } else {
                    cmd_parts.push(format!(
                        "<<# parameters.{0} >>{flag} \"<< parameters.{0} >>\"<</ parameters.{0} >>",
                        p.long_name
                    ));
                }
            }
        }
    }

    let command_str = cmd_parts.join(" \\\n  ");
    serde_yaml::Value::Mapping({
        let mut m = serde_yaml::Mapping::new();
        let mut run_map = serde_yaml::Mapping::new();
        run_map.insert(
            serde_yaml::Value::String("name".to_string()),
            serde_yaml::Value::String(sub.name.clone()),
        );
        run_map.insert(
            serde_yaml::Value::String("command".to_string()),
            serde_yaml::Value::String(command_str),
        );
        m.insert(
            serde_yaml::Value::String("run".to_string()),
            serde_yaml::Value::Mapping(run_map),
        );
        m
    })
}

/// Build the command invocation step for a job.
fn build_invoke_step(sub: &SubCommand) -> serde_yaml::Value {
    let mut invoke_map = serde_yaml::Mapping::new();
    for p in &sub.parameters {
        invoke_map.insert(
            serde_yaml::Value::String(p.long_name.clone()),
            serde_yaml::Value::String(format!("<< parameters.{} >>", p.long_name)),
        );
    }
    serde_yaml::Value::Mapping({
        let mut m = serde_yaml::Mapping::new();
        m.insert(
            serde_yaml::Value::String(sub.name.clone()),
            serde_yaml::Value::Mapping(invoke_map),
        );
        m
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::help_parser::types::{ParamType, Parameter, SubCommand};

    fn make_leaf(name: &str, params: Vec<Parameter>) -> SubCommand {
        SubCommand {
            name: name.to_string(),
            description: format!("Does {name} things."),
            is_leaf: true,
            parameters: params,
            subcommands: vec![],
        }
    }

    fn make_cli(binary: &str, subs: Vec<SubCommand>) -> CliDefinition {
        CliDefinition {
            binary_name: binary.to_string(),
            description: format!("The {binary} tool."),
            subcommands: subs,
        }
    }

    fn default_opts() -> GenerateOpts {
        GenerateOpts {
            namespaces: vec!["my-org".to_string()],
            install_method: InstallMethod::Binstall,
            base_image: "debian:12-slim".to_string(),
            home_url: None,
            source_url: None,
            binary_name: "mytool".to_string(),
        }
    }

    // ── @orb.yml ────────────────────────────────────────────────────────────

    #[test]
    fn orb_yml_has_no_commands_jobs_executors_keys() {
        let cli = make_cli("mytool", vec![]);
        let files = generate(&cli, &default_opts());
        let content = files[&PathBuf::from("src/@orb.yml")].clone();
        assert!(
            !content.contains("commands:"),
            "@orb.yml must not list commands:\n{content}"
        );
        assert!(
            !content.contains("jobs:"),
            "@orb.yml must not list jobs:\n{content}"
        );
        assert!(
            !content.contains("executors:"),
            "@orb.yml must not list executors:\n{content}"
        );
    }

    #[test]
    fn orb_yml_contains_version_and_description() {
        let cli = make_cli("mytool", vec![]);
        let files = generate(&cli, &default_opts());
        let content = &files[&PathBuf::from("src/@orb.yml")];
        // version must be the YAML float 2.1, not a quoted string
        assert!(
            content.contains("version: 2.1"),
            "version must be unquoted:\n{content}"
        );
        assert!(content.contains("The mytool tool."));
    }

    // ── executor ────────────────────────────────────────────────────────────

    #[test]
    fn executor_has_docker_image_with_tag_param() {
        let cli = make_cli("mytool", vec![]);
        let files = generate(&cli, &default_opts());
        let content = &files[&PathBuf::from("src/executors/default.yml")];
        assert!(
            content.contains("jerusdp/mytool:<< parameters.tag >>"),
            "executor image wrong:\n{content}"
        );
        assert!(
            content.contains("tag:"),
            "executor missing tag param:\n{content}"
        );
        assert!(
            content.contains("default: latest"),
            "tag default missing:\n{content}"
        );
    }

    // ── Dockerfile ──────────────────────────────────────────────────────────

    #[test]
    fn dockerfile_binstall_uses_slim_base_and_bootstrap() {
        let cli = make_cli("mytool", vec![]);
        let files = generate(&cli, &default_opts());
        let content = &files[&PathBuf::from("Dockerfile")];
        // Correct base image
        assert!(
            content.contains("FROM debian:12-slim"),
            "should use debian:12-slim:\n{content}"
        );
        // Bootstrap cargo-binstall (no cargo pre-installed on slim images)
        assert!(
            content.contains("cargo-bins/cargo-binstall"),
            "should bootstrap cargo-binstall:\n{content}"
        );
        // Install the binary via cargo-binstall (hyphen form, cargo not required)
        assert!(
            content.contains("cargo-binstall --no-confirm mytool"),
            "should install via cargo-binstall:\n{content}"
        );
        // Required runtime deps for TLS binaries
        assert!(
            content.contains("ca-certificates"),
            "should install ca-certificates:\n{content}"
        );
        // Clean up apt lists to keep image small
        assert!(
            content.contains("rm -rf /var/lib/apt/lists"),
            "should clean apt lists:\n{content}"
        );
    }

    #[test]
    fn dockerfile_binstall_cleans_cargo_cache() {
        let cli = make_cli("mytool", vec![]);
        let files = generate(&cli, &default_opts());
        let content = &files[&PathBuf::from("Dockerfile")];
        assert!(
            content.contains(".cargo/registry") || content.contains(".cargo/git"),
            "should clean cargo cache:\n{content}"
        );
    }

    #[test]
    fn dockerfile_apt() {
        let cli = make_cli("mytool", vec![]);
        let opts = GenerateOpts {
            install_method: InstallMethod::Apt,
            ..default_opts()
        };
        let files = generate(&cli, &opts);
        let content = &files[&PathBuf::from("Dockerfile")];
        assert!(
            content.contains("apt-get install -y"),
            "missing apt-get install:\n{content}"
        );
        assert!(
            content.contains("mytool"),
            "missing binary name:\n{content}"
        );
        assert!(
            content.contains("--no-install-recommends"),
            "apt should use --no-install-recommends:\n{content}"
        );
        assert!(
            content.contains("rm -rf /var/lib/apt/lists"),
            "apt should clean lists:\n{content}"
        );
    }

    // ── command files ───────────────────────────────────────────────────────

    #[test]
    fn required_param_renders_without_conditional() {
        let params = vec![Parameter {
            long_name: "orb_path".to_string(),
            short: Some('p'),
            param_type: ParamType::String,
            default: None,
            required: true,
            description: "Path to orb.".to_string(),
        }];
        let sub = make_leaf("generate", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts());
        let content = &files[&PathBuf::from("src/commands/generate.yml")];
        assert!(
            content.contains("--orb-path \"<< parameters.orb_path >>\""),
            "required param not rendered correctly:\n{content}"
        );
        assert!(
            !content.contains("<<# parameters.orb_path >>"),
            "required param should not use conditional:\n{content}"
        );
    }

    #[test]
    fn optional_string_param_renders_with_mustache_conditional() {
        let params = vec![Parameter {
            long_name: "output".to_string(),
            short: Some('o'),
            param_type: ParamType::String,
            default: Some("./dist".to_string()),
            required: false,
            description: "Output dir.".to_string(),
        }];
        let sub = make_leaf("generate", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts());
        let content = &files[&PathBuf::from("src/commands/generate.yml")];
        assert!(
            content.contains("<<# parameters.output >>"),
            "optional param should use mustache conditional:\n{content}"
        );
    }

    #[test]
    fn boolean_flag_renders_with_mustache_conditional_no_value() {
        let params = vec![Parameter {
            long_name: "force".to_string(),
            short: None,
            param_type: ParamType::Boolean,
            default: None,
            required: false,
            description: "Force overwrite.".to_string(),
        }];
        let sub = make_leaf("generate", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts());
        let content = &files[&PathBuf::from("src/commands/generate.yml")];
        assert!(
            content.contains("<<# parameters.force >>--force<</ parameters.force >>"),
            "boolean flag rendered incorrectly:\n{content}"
        );
    }

    #[test]
    fn enum_parameter_has_enum_key() {
        let params = vec![Parameter {
            long_name: "format".to_string(),
            short: Some('f'),
            param_type: ParamType::Enum(vec!["binary".to_string(), "source".to_string()]),
            default: Some("source".to_string()),
            required: false,
            description: "Output format.".to_string(),
        }];
        let sub = make_leaf("generate", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts());
        let content = &files[&PathBuf::from("src/commands/generate.yml")];
        assert!(
            content.contains("enum:"),
            "enum param missing enum key:\n{content}"
        );
        assert!(
            content.contains("binary"),
            "enum missing value 'binary':\n{content}"
        );
        assert!(
            content.contains("source"),
            "enum missing value 'source':\n{content}"
        );
    }

    // ── job files ───────────────────────────────────────────────────────────

    #[test]
    fn job_references_executor_default() {
        let sub = make_leaf("validate", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts());
        let content = &files[&PathBuf::from("src/jobs/validate.yml")];
        assert!(
            content.contains("executor: default"),
            "job must reference default executor:\n{content}"
        );
    }

    #[test]
    fn job_has_checkout_step() {
        let sub = make_leaf("validate", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts());
        let content = &files[&PathBuf::from("src/jobs/validate.yml")];
        assert!(
            content.contains("checkout"),
            "job missing checkout step:\n{content}"
        );
    }

    #[test]
    fn command_and_job_files_created_for_each_leaf() {
        let subs = vec![
            make_leaf("generate", vec![]),
            make_leaf("validate", vec![]),
            make_leaf("diff", vec![]),
        ];
        let cli = make_cli("mytool", subs);
        let files = generate(&cli, &default_opts());
        for name in &["generate", "validate", "diff"] {
            assert!(
                files.contains_key(&PathBuf::from(format!("src/commands/{name}.yml"))),
                "missing commands/{name}.yml"
            );
            assert!(
                files.contains_key(&PathBuf::from(format!("src/jobs/{name}.yml"))),
                "missing jobs/{name}.yml"
            );
        }
    }
}
