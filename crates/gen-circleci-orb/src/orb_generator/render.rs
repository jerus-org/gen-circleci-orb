use super::types::{DockerImage, OrbCommand, OrbExecutor, OrbJob, OrbParameter};
use crate::commands::generate::InstallMethod;
use crate::help_parser::types::{CliDefinition, ParamType, SubCommand};
use crate::orb_config::OrbConfig;
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
    /// Subcommand names whose generated jobs should include a `set_https_remote` step.
    /// Use for subcommands that push to git (e.g. "save").
    pub git_push_subcommands: Vec<String>,
    /// When set, adds a cli-installer stage to the generated Dockerfile that downloads
    /// and checksum-verifies this version of the circleci CLI binary.  Required when the
    /// wrapped binary calls `circleci` commands at runtime (e.g. gen-circleci-orb itself).
    pub circleci_cli_version: Option<String>,
    /// Extra apt packages to install in the final Docker image stage (sorted together with
    /// the baseline packages: ca-certificates, git).
    pub apt_packages: Vec<String>,
}

/// Generate all orb artifact strings keyed by their relative output path.
pub fn generate(
    cli: &CliDefinition,
    opts: &GenerateOpts,
    config: Option<&OrbConfig>,
) -> HashMap<PathBuf, String> {
    let mut files = HashMap::new();

    // @orb.yml — metadata only; hand-formatted so `version: 2.1` stays unquoted
    files.insert(
        PathBuf::from("src/@orb.yml"),
        render_orb_root(cli, opts, config),
    );

    // executors/default.yml
    files.insert(
        PathBuf::from("src/executors/default.yml"),
        render_executor(&cli.binary_name),
    );

    // commands/<name>.yml and jobs/<name>.yml for each leaf subcommand
    for sub in &cli.subcommands {
        render_subcommand(sub, &cli.binary_name, opts, config, &mut files);
    }

    // Dockerfile
    files.insert(
        PathBuf::from("Dockerfile"),
        render_dockerfile(
            &cli.binary_name,
            &opts.install_method,
            &opts.base_image,
            opts.circleci_cli_version.as_deref(),
            &opts.apt_packages,
        ),
    );

    // src/jobs/<name>.yml for each job_group in config
    if let Some(groups) = config.and_then(|c| c.job_group.as_ref()) {
        for group in groups {
            let snake = group.name.replace('-', "_");
            files.insert(
                PathBuf::from(format!("src/jobs/{snake}.yml")),
                render_job_group(group, cli, config),
            );
        }
    }

    // src/jobs/<name>.yml for each extra_job in config (verbatim YAML)
    if let Some(extras) = config.and_then(|c| c.extra_job.as_ref()) {
        for extra in extras {
            let content = extra.yaml.trim().to_string() + "\n";
            files.insert(
                PathBuf::from(format!("src/jobs/{}.yml", extra.name)),
                content,
            );
        }
    }

    // add-workspace-to-path.sh — always generated; referenced by every job's
    // attach_workspace conditional step via <<include(scripts/add-workspace-to-path.sh)>>
    files.insert(
        PathBuf::from("src/scripts/add-workspace-to-path.sh"),
        "export PATH=\"${WORKSPACE_ROOT}:${PATH}\"\n".to_string(),
    );

    // set_https_remote command + script (generated whenever any push subcommand is named)
    if !opts.git_push_subcommands.is_empty() {
        files.insert(
            PathBuf::from("src/commands/set_https_remote.yml"),
            render_set_https_remote_command(),
        );
        files.insert(
            PathBuf::from("src/scripts/set_https_remote.sh"),
            render_set_https_remote_script(),
        );
    }

    // examples/example.yml (RC003)
    files.insert(
        PathBuf::from("src/examples/example.yml"),
        render_example(cli, opts, config),
    );

    files
}

fn render_orb_root(cli: &CliDefinition, opts: &GenerateOpts, config: Option<&OrbConfig>) -> String {
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
    if let Some(orbs) = config
        .and_then(|c| c.orbs.as_ref())
        .filter(|o| !o.is_empty())
    {
        out.push_str("orbs:\n");
        for (name, reference) in orbs {
            out.push_str(&format!("  {name}: {reference}\n"));
        }
    }
    out
}

fn is_job_suppressed(config: Option<&OrbConfig>, name: &str) -> bool {
    config
        .and_then(|c| c.subcommand.as_ref())
        .and_then(|sc| sc.get(name))
        .and_then(|sc_config| sc_config.generate_job)
        .map(|generate| !generate)
        .unwrap_or(false)
}

fn render_subcommand(
    sub: &SubCommand,
    binary: &str,
    opts: &GenerateOpts,
    config: Option<&OrbConfig>,
    files: &mut HashMap<PathBuf, String>,
) {
    if sub.is_leaf {
        let snake = sub.name.replace('-', "_");
        files.insert(
            PathBuf::from(format!("src/commands/{snake}.yml")),
            render_command(sub),
        );
        if !is_job_suppressed(config, &sub.name) {
            files.insert(
                PathBuf::from(format!("src/jobs/{snake}.yml")),
                render_job(sub, opts, config),
            );
        }
        files.insert(
            PathBuf::from(format!("src/scripts/{snake}.sh")),
            render_command_script_content(sub, binary),
        );
    }
    for child in &sub.subcommands {
        render_subcommand(child, binary, opts, config, files);
    }
}

/// CircleCI parameter names that are restricted in command definitions.
/// orb pack rejects these with "Restricted parameter: '<name>'".
/// Rather than dropping them, the generator renames them to `{subcommand}_{param}`
/// so the functionality is preserved under a descriptive, unambiguous name.
const RESTRICTED_COMMAND_PARAMS: &[&str] = &["name"];

/// Returns the orb parameter name to use for a CLI parameter in a command.
/// Restricted names are prefixed with the subcommand name
/// (e.g. `generate` + `name` → `generate_name`).
fn resolve_command_param_name(subcommand: &str, param: &str) -> String {
    if RESTRICTED_COMMAND_PARAMS.contains(&param) {
        format!("{subcommand}_{param}")
    } else {
        param.to_string()
    }
}

/// CircleCI job parameter names that are reserved by the platform and cannot be
/// used as user-defined parameters in job definitions.
const RESERVED_JOB_PARAMS: &[&str] = &[
    "name",
    "type",
    "filters",
    "matrix",
    "requires",
    "context",
    "pre_steps",
    "post_steps",
];

fn render_command(sub: &SubCommand) -> String {
    let parameters = build_command_orb_parameters(sub);
    let step = build_run_step(sub);
    let cmd = OrbCommand {
        description: sub.description.clone(),
        parameters,
        steps: vec![step],
    };
    serde_yaml::to_string(&cmd).unwrap()
}

/// Build orb parameters for a command, renaming any restricted names with a
/// subcommand prefix so they remain usable (e.g. `name` → `generate_name`).
fn build_command_orb_parameters(sub: &SubCommand) -> IndexMap<String, OrbParameter> {
    let mut params = IndexMap::new();
    for p in &sub.parameters {
        let orb_name = resolve_command_param_name(&sub.name, &p.long_name);
        let (type_str, enum_vals) = match &p.param_type {
            ParamType::String => ("string".to_string(), None),
            ParamType::Boolean => ("boolean".to_string(), None),
            ParamType::Integer => ("integer".to_string(), None),
            ParamType::Enum(vals) => ("enum".to_string(), Some(vals.clone())),
        };
        let default = match &p.param_type {
            ParamType::Boolean => {
                let val = p.default.as_ref().map(|d| d == "true").unwrap_or(false);
                Some(serde_yaml::Value::Bool(val))
            }
            _ if !p.required && p.default.is_none() => {
                Some(serde_yaml::Value::String(String::new()))
            }
            _ => p
                .default
                .as_ref()
                .map(|d| serde_yaml::Value::String(d.clone())),
        };
        params.insert(
            orb_name,
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

/// Build the shell script body for a command.
/// Parameters are received as uppercased env vars (set via the YAML environment: block).
fn render_command_script_content(sub: &SubCommand, binary: &str) -> String {
    let mut lines: Vec<String> = vec![format!("set -- {} {}", binary, sub.name.replace('_', "-"))];

    for p in &sub.parameters {
        let orb_name = resolve_command_param_name(&sub.name, &p.long_name);
        let env_var = orb_name.to_uppercase();
        let flag = format!("--{}", p.long_name.replace('_', "-"));
        let line = match &p.param_type {
            ParamType::Boolean => {
                format!(r#"[[ "${{{env_var}:-false}}" = "true" ]] && set -- "$@" {flag}"#)
            }
            _ => {
                if p.required {
                    format!(r#"set -- "$@" {flag} "${{{env_var}}}""#)
                } else {
                    format!(r#"[[ -n "${{{env_var}:-}}" ]] && set -- "$@" {flag} "${{{env_var}}}""#)
                }
            }
        };
        lines.push(line);
    }

    lines.push(r#""$@""#.to_string());
    lines.join("\n") + "\n"
}

fn render_job(sub: &SubCommand, opts: &GenerateOpts, config: Option<&OrbConfig>) -> String {
    let mut parameters = build_orb_parameters(sub, RESERVED_JOB_PARAMS);

    // Apply param default overrides from config
    if let Some(param_overrides) = config
        .and_then(|c| c.subcommand.as_ref())
        .and_then(|sc| sc.get(&sub.name))
        .and_then(|sc_config| sc_config.param.as_ref())
    {
        for (param_name, override_) in param_overrides {
            if let Some(param) = parameters.get_mut(param_name) {
                if let Some(new_default) = &override_.default {
                    param.default = Some(serde_yaml::Value::String(new_default.clone()));
                }
            }
        }
    }
    let (attach_param, root_param) = build_workspace_params();
    parameters.insert("attach_workspace".to_string(), attach_param);
    parameters.insert("workspace_root".to_string(), root_param);

    let invoke_step = build_invoke_step(sub, RESERVED_JOB_PARAMS);
    let mut steps = vec![
        serde_yaml::Value::String("checkout".to_string()),
        build_attach_workspace_step(),
    ];
    if opts.git_push_subcommands.contains(&sub.name) {
        steps.push(serde_yaml::Value::String("set_https_remote".to_string()));
    }
    steps.push(invoke_step);
    let job = OrbJob {
        description: format!("Run {} command in a dedicated job.", sub.name),
        executor: "default".to_string(),
        parameters,
        steps,
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

fn render_cli_installer_stage(ver: &str) -> String {
    let mut s = String::new();
    s.push_str("FROM debian:12-slim AS cli-installer\n");
    s.push_str(&format!("ARG CIRCLECI_CLI_VERSION={ver}\n"));
    s.push_str("RUN apt-get update \\\n");
    s.push_str("    && apt-get install -y --no-install-recommends ca-certificates curl \\\n");
    s.push_str("    && rm -rf /var/lib/apt/lists/* \\\n");
    s.push_str("    && cd /tmp \\\n");
    s.push_str("    && TARBALL=\"circleci-cli_${CIRCLECI_CLI_VERSION}_linux_amd64.tar.gz\" \\\n");
    s.push_str("    && curl -fLSs --proto '=https' \"https://github.com/CircleCI-Public/circleci-cli/releases/download/v${CIRCLECI_CLI_VERSION}/${TARBALL}\" -o \"${TARBALL}\" \\\n");
    s.push_str("    && curl -fLSs --proto '=https' \"https://github.com/CircleCI-Public/circleci-cli/releases/download/v${CIRCLECI_CLI_VERSION}/circleci-cli_${CIRCLECI_CLI_VERSION}_checksums.txt\" -o checksums.txt \\\n");
    s.push_str("    && grep \"${TARBALL}\" checksums.txt | sha256sum --check \\\n");
    s.push_str("    && tar -xzf \"${TARBALL}\" --strip 1 \\\n");
    s.push_str("    && install -m 755 circleci /usr/local/bin/circleci \\\n");
    s.push_str("    && rm -rf \"${TARBALL}\" checksums.txt circleci\n");
    s
}

fn sorted_package_list(extra: &[String]) -> String {
    let mut pkgs: Vec<&str> = vec!["ca-certificates", "git"];
    pkgs.extend(extra.iter().map(String::as_str));
    pkgs.sort_unstable();
    pkgs.dedup();
    pkgs.join(" ")
}

fn render_dockerfile(
    binary: &str,
    method: &InstallMethod,
    base_image: &str,
    circleci_cli_version: Option<&str>,
    apt_packages: &[String],
) -> String {
    match method {
        InstallMethod::Binstall => {
            let runtime_pkgs = sorted_package_list(apt_packages);
            let mut out = String::new();
            out.push_str("FROM rust:1-slim-bookworm AS builder\n");
            out.push_str("RUN apt-get update \\\n");
            out.push_str(
                "    && apt-get install -y --no-install-recommends ca-certificates libssl-dev pkg-config \\\n",
            );
            out.push_str("    && rm -rf /var/lib/apt/lists/* \\\n");
            out.push_str(&format!("    && cargo install {binary}\n"));
            if let Some(ver) = circleci_cli_version {
                out.push('\n');
                out.push_str(&render_cli_installer_stage(ver));
            }
            out.push('\n');
            out.push_str(&format!("FROM {base_image}\n"));
            out.push_str("RUN apt-get update \\\n");
            out.push_str(&format!(
                "    && apt-get install -y --no-install-recommends {runtime_pkgs} \\\n"
            ));
            out.push_str("    && rm -rf /var/lib/apt/lists/* \\\n");
            out.push_str("    && useradd -ms /bin/bash circleci\n");
            out.push_str(&format!(
                "COPY --from=builder /usr/local/cargo/bin/{binary} /usr/local/bin/{binary}\n"
            ));
            if circleci_cli_version.is_some() {
                out.push_str(
                    "COPY --from=cli-installer /usr/local/bin/circleci /usr/local/bin/circleci\n",
                );
            }
            out.push_str("USER circleci\n");
            out.push_str("WORKDIR /home/circleci/project\n");
            out
        }
        InstallMethod::Local => {
            let runtime_pkgs = sorted_package_list(apt_packages);
            let mut out = String::new();
            if let Some(ver) = circleci_cli_version {
                out.push_str(&render_cli_installer_stage(ver));
                out.push('\n');
            }
            out.push_str(&format!("FROM {base_image}\n"));
            out.push_str("RUN apt-get update \\\n");
            out.push_str(&format!(
                "    && apt-get install -y --no-install-recommends {runtime_pkgs} \\\n"
            ));
            out.push_str("    && rm -rf /var/lib/apt/lists/* \\\n");
            out.push_str("    && useradd -ms /bin/bash circleci\n");
            out.push_str(&format!("COPY {binary} /usr/local/bin/{binary}\n"));
            if circleci_cli_version.is_some() {
                out.push_str(
                    "COPY --from=cli-installer /usr/local/bin/circleci /usr/local/bin/circleci\n",
                );
            }
            out.push_str("USER circleci\n");
            out.push_str("WORKDIR /home/circleci/project\n");
            out
        }
        InstallMethod::Apt => {
            let extra_pkgs: Vec<&str> = apt_packages.iter().map(String::as_str).collect();
            let mut all_pkgs = vec!["git", binary];
            all_pkgs.extend(extra_pkgs);
            all_pkgs.sort_unstable();
            all_pkgs.dedup();
            let pkg_list = all_pkgs.join(" ");
            format!(
                "FROM {base_image}\nRUN apt-get update \\\n    && apt-get install -y --no-install-recommends {pkg_list} \\\n    && rm -rf /var/lib/apt/lists/*\n"
            )
        }
    }
}

fn render_set_https_remote_command() -> String {
    // CircleCI checkout injects url."ssh://git@github.com".insteadOf = https://github.com
    // into ~/.gitconfig.  This causes any subsequent HTTPS git operation (including libgit2
    // used by pcu) to be silently rewritten to SSH, bypassing the GitHub App token.
    // This command removes that rewrite and switches the remote to HTTPS so that jobs
    // that push (e.g. save) can authenticate with the GitHub App token.
    "description: >\n  Remove the SSH insteadOf rewrite rule that CircleCI checkout injects and\n  set both the fetch and push URLs for origin to HTTPS.\nsteps:\n- run:\n    name: Set HTTPS remote URLs (fetch and push)\n    command: <<include(scripts/set_https_remote.sh)>>\n".to_string()
}

fn render_set_https_remote_script() -> String {
    "# CircleCI's checkout step injects this rule into ~/.gitconfig:\n#   url.\"ssh://git@github.com\".insteadOf = https://github.com\n# This causes git (and libgit2 used by pcu) to transparently rewrite every\n# HTTPS GitHub URL back to SSH, so git remote set-url has no observable effect\n# on the effective URL. Remove the rule before setting the remote URLs.\ngit config --global --unset-all \"url.ssh://git@github.com.insteadOf\" 2>/dev/null || true\nHTTPS_ORIGIN=\"https://github.com/${CIRCLE_PROJECT_USERNAME}/${CIRCLE_PROJECT_REPONAME}.git\"\ngit remote set-url origin \"${HTTPS_ORIGIN}\"\ngit remote set-url --push origin \"${HTTPS_ORIGIN}\"\n".to_string()
}

fn render_example(cli: &CliDefinition, opts: &GenerateOpts, config: Option<&OrbConfig>) -> String {
    let namespace = opts
        .namespaces
        .first()
        .map(String::as_str)
        .unwrap_or("my-org");
    let binary = &cli.binary_name;
    // Use the first non-suppressed leaf subcommand for the example job.
    let first_sub = cli
        .subcommands
        .iter()
        .find(|s| s.is_leaf && !is_job_suppressed(config, &s.name));
    // RC010: job names in examples must use snake_case to match generated filenames.
    let job_name = first_sub
        .map(|s| s.name.replace('-', "_"))
        .unwrap_or_else(|| binary.to_string());
    // Collect required parameters (no default, not boolean) for the example.
    // orb-tools review validates that required params are supplied in examples.
    let required_params: Vec<&crate::help_parser::types::Parameter> = first_sub
        .map(|s| {
            s.parameters
                .iter()
                .filter(|p| {
                    p.required && p.default.is_none() && !matches!(p.param_type, ParamType::Boolean)
                })
                .collect()
        })
        .unwrap_or_default();

    let mut out = format!(
        "description: >\n  Example usage of the {binary} orb.\nusage:\n  version: 2.1\n  orbs:\n    {binary}: {namespace}/{binary}@1.0\n  workflows:\n    use-my-orb:\n      jobs:\n"
    );
    if required_params.is_empty() {
        out.push_str(&format!("        - {binary}/{job_name}\n"));
    } else {
        out.push_str(&format!("        - {binary}/{job_name}:\n"));
        for p in required_params {
            let placeholder = p.long_name.replace('_', "-");
            out.push_str(&format!(
                "            {}: your-{placeholder}\n",
                p.long_name
            ));
        }
    }
    out
}

fn build_orb_parameters(sub: &SubCommand, skip: &[&str]) -> IndexMap<String, OrbParameter> {
    let mut params = IndexMap::new();
    for p in &sub.parameters {
        if skip.contains(&p.long_name.as_str()) {
            continue;
        }
        let (type_str, enum_vals) = match &p.param_type {
            ParamType::String => ("string".to_string(), None),
            ParamType::Boolean => ("boolean".to_string(), None),
            ParamType::Integer => ("integer".to_string(), None),
            ParamType::Enum(vals) => ("enum".to_string(), Some(vals.clone())),
        };
        let default = match &p.param_type {
            ParamType::Boolean => {
                // Clap booleans default to false but never emit [default: false] in help
                // text. Always supply default: false so orb consumers can omit the param.
                let val = p.default.as_ref().map(|d| d == "true").unwrap_or(false);
                Some(serde_yaml::Value::Bool(val))
            }
            _ if !p.required && p.default.is_none() => {
                // Optional CLI flag with no default: use empty string so consumers can
                // omit the param. The run step uses a mustache conditional, so "" means
                // the flag is not forwarded to the binary.
                Some(serde_yaml::Value::String(String::new()))
            }
            _ => p
                .default
                .as_ref()
                .map(|d| serde_yaml::Value::String(d.clone())),
        };
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

/// Build the `run:` step for a command, referencing the script file (RC009 compliance).
/// Adds an `environment:` block so the script can read params as uppercased env vars.
fn build_run_step(sub: &SubCommand) -> serde_yaml::Value {
    serde_yaml::Value::Mapping({
        let mut m = serde_yaml::Mapping::new();
        let mut run_map = serde_yaml::Mapping::new();
        run_map.insert(
            serde_yaml::Value::String("name".to_string()),
            serde_yaml::Value::String(sub.name.clone()),
        );
        run_map.insert(
            serde_yaml::Value::String("command".to_string()),
            serde_yaml::Value::String(format!(
                "<<include(scripts/{}.sh)>>",
                sub.name.replace('-', "_")
            )),
        );
        if !sub.parameters.is_empty() {
            let mut env_map = serde_yaml::Mapping::new();
            for p in &sub.parameters {
                let orb_name = resolve_command_param_name(&sub.name, &p.long_name);
                let env_var = orb_name.to_uppercase();
                env_map.insert(
                    serde_yaml::Value::String(env_var),
                    serde_yaml::Value::String(format!("<< parameters.{orb_name} >>")),
                );
            }
            run_map.insert(
                serde_yaml::Value::String("environment".to_string()),
                serde_yaml::Value::Mapping(env_map),
            );
        }
        m.insert(
            serde_yaml::Value::String("run".to_string()),
            serde_yaml::Value::Mapping(run_map),
        );
        m
    })
}

/// Build the command invocation step for a job.
fn build_invoke_step(sub: &SubCommand, skip: &[&str]) -> serde_yaml::Value {
    let mut invoke_map = serde_yaml::Mapping::new();
    for p in &sub.parameters {
        if skip.contains(&p.long_name.as_str()) {
            continue;
        }
        invoke_map.insert(
            serde_yaml::Value::String(p.long_name.clone()),
            serde_yaml::Value::String(format!("<< parameters.{} >>", p.long_name)),
        );
    }
    serde_yaml::Value::Mapping({
        let mut m = serde_yaml::Mapping::new();
        m.insert(
            serde_yaml::Value::String(sub.name.replace('-', "_")),
            serde_yaml::Value::Mapping(invoke_map),
        );
        m
    })
}

fn build_workspace_params() -> (OrbParameter, OrbParameter) {
    let attach = OrbParameter {
        param_type: "boolean".to_string(),
        description: "Attach a workspace before running the command (use when the binary was built in a prior job).".to_string(),
        default: Some(serde_yaml::Value::Bool(false)),
        enum_values: None,
    };
    let root = OrbParameter {
        param_type: "string".to_string(),
        description: "Path at which to attach the workspace; also prepended to PATH (only used when attach_workspace is true).".to_string(),
        default: Some(serde_yaml::Value::String("/tmp/workspace".to_string())),
        enum_values: None,
    };
    (attach, root)
}

fn build_attach_workspace_step() -> serde_yaml::Value {
    let mut attach_map = serde_yaml::Mapping::new();
    attach_map.insert(
        serde_yaml::Value::String("at".to_string()),
        serde_yaml::Value::String("<< parameters.workspace_root >>".to_string()),
    );
    let mut add_path_env = serde_yaml::Mapping::new();
    add_path_env.insert(
        serde_yaml::Value::String("WORKSPACE_ROOT".to_string()),
        serde_yaml::Value::String("<< parameters.workspace_root >>".to_string()),
    );
    let mut add_path_run = serde_yaml::Mapping::new();
    add_path_run.insert(
        serde_yaml::Value::String("name".to_string()),
        serde_yaml::Value::String("Add workspace binaries to PATH".to_string()),
    );
    add_path_run.insert(
        serde_yaml::Value::String("command".to_string()),
        serde_yaml::Value::String("<<include(scripts/add-workspace-to-path.sh)>>".to_string()),
    );
    add_path_run.insert(
        serde_yaml::Value::String("environment".to_string()),
        serde_yaml::Value::Mapping(add_path_env),
    );
    let mut attach_ws_map = serde_yaml::Mapping::new();
    attach_ws_map.insert(
        serde_yaml::Value::String("attach_workspace".to_string()),
        serde_yaml::Value::Mapping(attach_map),
    );
    let inner_steps = serde_yaml::Value::Sequence(vec![
        serde_yaml::Value::Mapping(attach_ws_map),
        serde_yaml::Value::Mapping({
            let mut m = serde_yaml::Mapping::new();
            m.insert(
                serde_yaml::Value::String("run".to_string()),
                serde_yaml::Value::Mapping(add_path_run),
            );
            m
        }),
    ]);
    let mut when_inner = serde_yaml::Mapping::new();
    when_inner.insert(
        serde_yaml::Value::String("condition".to_string()),
        serde_yaml::Value::String("<< parameters.attach_workspace >>".to_string()),
    );
    when_inner.insert(serde_yaml::Value::String("steps".to_string()), inner_steps);
    let mut when_map = serde_yaml::Mapping::new();
    when_map.insert(
        serde_yaml::Value::String("when".to_string()),
        serde_yaml::Value::Mapping(when_inner),
    );
    serde_yaml::Value::Mapping(when_map)
}

fn cli_param_to_orb_param(p: &crate::help_parser::types::Parameter) -> OrbParameter {
    let (type_str, enum_vals) = match &p.param_type {
        ParamType::String => ("string".to_string(), None),
        ParamType::Boolean => ("boolean".to_string(), None),
        ParamType::Integer => ("integer".to_string(), None),
        ParamType::Enum(vals) => ("enum".to_string(), Some(vals.clone())),
    };
    let default = match &p.param_type {
        ParamType::Boolean => {
            let val = p.default.as_ref().map(|d| d == "true").unwrap_or(false);
            Some(serde_yaml::Value::Bool(val))
        }
        _ if !p.required && p.default.is_none() => Some(serde_yaml::Value::String(String::new())),
        _ => p
            .default
            .as_ref()
            .map(|d| serde_yaml::Value::String(d.clone())),
    };
    OrbParameter {
        param_type: type_str,
        description: p.description.clone(),
        default,
        enum_values: enum_vals,
    }
}

fn find_leaf_subcommand<'a>(cli: &'a CliDefinition, name: &str) -> Option<&'a SubCommand> {
    fn search<'a>(subs: &'a [SubCommand], name: &str) -> Option<&'a SubCommand> {
        for sub in subs {
            if sub.is_leaf && sub.name == name {
                return Some(sub);
            }
            if let Some(found) = search(&sub.subcommands, name) {
                return Some(found);
            }
        }
        None
    }
    search(&cli.subcommands, name)
}

fn resolve_explicit_params(
    explicit: &[String],
    step_subs: &[&SubCommand],
) -> IndexMap<String, OrbParameter> {
    let mut result = IndexMap::new();
    for param_name in explicit {
        for sub in step_subs.iter() {
            if let Some(p) = sub.parameters.iter().find(|p| &p.long_name == param_name) {
                result.insert(param_name.clone(), cli_param_to_orb_param(p));
                break;
            }
        }
    }
    result
}

fn resolve_shared_params(step_subs: &[&SubCommand]) -> IndexMap<String, OrbParameter> {
    let mut result = IndexMap::new();
    let Some(first) = step_subs.first() else {
        return result;
    };
    let shared_names: std::collections::HashSet<&str> = step_subs[1..].iter().fold(
        first
            .parameters
            .iter()
            .map(|p| p.long_name.as_str())
            .collect(),
        |acc: std::collections::HashSet<&str>, sub| {
            let sub_names: std::collections::HashSet<&str> = sub
                .parameters
                .iter()
                .map(|p| p.long_name.as_str())
                .collect();
            acc.intersection(&sub_names).copied().collect()
        },
    );
    for p in &first.parameters {
        if shared_names.contains(p.long_name.as_str()) {
            result.insert(p.long_name.clone(), cli_param_to_orb_param(p));
        }
    }
    result
}

fn add_mandatory_params(params: &mut IndexMap<String, OrbParameter>, step_subs: &[&SubCommand]) {
    let mut present: std::collections::HashSet<String> = params.keys().cloned().collect();
    for sub in step_subs {
        for p in &sub.parameters {
            if !p.required || matches!(p.param_type, ParamType::Boolean) {
                continue;
            }
            if present.contains(&p.long_name) {
                continue;
            }
            let collides = step_subs.iter().any(|other| {
                other.name != sub.name
                    && other
                        .parameters
                        .iter()
                        .any(|op| op.long_name == p.long_name)
            });
            let job_name = if collides {
                format!("{}_{}", sub.name, p.long_name)
            } else {
                p.long_name.clone()
            };
            params.insert(job_name.clone(), cli_param_to_orb_param(p));
            present.insert(job_name);
        }
    }
}

fn build_job_group_params(
    group: &crate::orb_config::JobGroup,
    step_subs: &[&SubCommand],
) -> IndexMap<String, OrbParameter> {
    let mut params = if let Some(explicit) = &group.params {
        resolve_explicit_params(explicit, step_subs)
    } else {
        resolve_shared_params(step_subs)
    };
    add_mandatory_params(&mut params, step_subs);
    params
}

fn build_job_group_invoke_step(
    sub: &SubCommand,
    job_params: &IndexMap<String, OrbParameter>,
) -> serde_yaml::Value {
    let mut invoke_map = serde_yaml::Mapping::new();
    for p in &sub.parameters {
        // Find the name this param has in the merged job parameter set
        let job_param_name = if job_params.contains_key(&p.long_name) {
            Some(p.long_name.clone())
        } else {
            let prefixed = format!("{}_{}", sub.name, p.long_name);
            if job_params.contains_key(&prefixed) {
                Some(prefixed)
            } else {
                None
            }
        };
        if let Some(job_name) = job_param_name {
            invoke_map.insert(
                serde_yaml::Value::String(p.long_name.clone()),
                serde_yaml::Value::String(format!("<< parameters.{job_name} >>")),
            );
        }
    }
    serde_yaml::Value::Mapping({
        let mut m = serde_yaml::Mapping::new();
        m.insert(
            serde_yaml::Value::String(sub.name.replace('-', "_")),
            serde_yaml::Value::Mapping(invoke_map),
        );
        m
    })
}

fn render_job_group(
    group: &crate::orb_config::JobGroup,
    cli: &CliDefinition,
    _config: Option<&OrbConfig>,
) -> String {
    let step_subs: Vec<&SubCommand> = group
        .steps
        .iter()
        .filter_map(|name| find_leaf_subcommand(cli, name))
        .collect();

    let mut parameters = build_job_group_params(group, &step_subs);

    let (attach_param, root_param) = build_workspace_params();
    parameters.insert("attach_workspace".to_string(), attach_param);
    parameters.insert("workspace_root".to_string(), root_param);

    let mut steps = vec![
        serde_yaml::Value::String("checkout".to_string()),
        build_attach_workspace_step(),
    ];
    for sub in &step_subs {
        steps.push(build_job_group_invoke_step(sub, &parameters));
    }

    let description = group
        .description
        .clone()
        .unwrap_or_else(|| format!("Run {} in sequence.", group.steps.join(", ")));

    let job = OrbJob {
        description,
        executor: "default".to_string(),
        parameters,
        steps,
    };
    serde_yaml::to_string(&job).unwrap()
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
            git_push_subcommands: vec![],
            circleci_cli_version: None,
            apt_packages: vec![],
        }
    }

    // ── @orb.yml ────────────────────────────────────────────────────────────

    #[test]
    fn orb_yml_has_no_commands_jobs_executors_keys() {
        let cli = make_cli("mytool", vec![]);
        let files = generate(&cli, &default_opts(), None);
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
        let files = generate(&cli, &default_opts(), None);
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
        let files = generate(&cli, &default_opts(), None);
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
    fn dockerfile_binstall_uses_multistage_build() {
        let cli = make_cli("mytool", vec![]);
        let files = generate(&cli, &default_opts(), None);
        let content = &files[&PathBuf::from("Dockerfile")];
        // Builder stage uses Rust on Bookworm so binary links against same GLIBC as runtime
        assert!(
            content.contains("FROM rust:1-slim-bookworm AS builder"),
            "should use rust:1-slim-bookworm builder stage:\n{content}"
        );
        // Runtime stage is the slim Debian image
        assert!(
            content.contains("FROM debian:12-slim"),
            "should use debian:12-slim runtime stage:\n{content}"
        );
        // Binary compiled from source in builder stage — no curl|bash
        assert!(
            content.contains("cargo install mytool"),
            "should install via cargo install in builder stage:\n{content}"
        );
        // Binary copied from builder to runtime
        assert!(
            content.contains("COPY --from=builder"),
            "should copy binary from builder stage:\n{content}"
        );
        // No pipe-to-bash pattern
        assert!(
            !content.contains("| bash"),
            "must not use curl|bash pattern:\n{content}"
        );
    }

    #[test]
    fn dockerfile_binstall_runtime_has_ca_certs_and_git() {
        let cli = make_cli("mytool", vec![]);
        let files = generate(&cli, &default_opts(), None);
        let content = &files[&PathBuf::from("Dockerfile")];
        assert!(
            content.contains("ca-certificates"),
            "runtime stage should install ca-certificates:\n{content}"
        );
        assert!(
            content.contains("apt-get install") && content.contains(" git"),
            "runtime stage must install git for CircleCI checkout step:\n{content}"
        );
    }

    #[test]
    fn dockerfile_binstall_includes_git() {
        let cli = make_cli("mytool", vec![]);
        let files = generate(&cli, &default_opts(), None);
        let content = &files[&PathBuf::from("Dockerfile")];
        // git must appear as an apt package install, not just in cargo paths
        assert!(
            content.contains("apt-get install") && content.contains(" git"),
            "Dockerfile must install git via apt for CircleCI checkout step:\n{content}"
        );
    }

    #[test]
    fn dockerfile_binstall_has_circleci_user_and_workdir() {
        let cli = make_cli("mytool", vec![]);
        let files = generate(&cli, &default_opts(), None);
        let content = &files[&PathBuf::from("Dockerfile")];
        assert!(
            content.contains("useradd") && content.contains("circleci"),
            "runtime stage must create circleci user:\n{content}"
        );
        assert!(
            content.contains("USER circleci"),
            "runtime stage must set USER circleci:\n{content}"
        );
        assert!(
            content.contains("WORKDIR /home/circleci/project"),
            "runtime stage must set WORKDIR /home/circleci/project:\n{content}"
        );
    }

    #[test]
    fn dockerfile_binstall_does_not_run_as_root() {
        let cli = make_cli("mytool", vec![]);
        let files = generate(&cli, &default_opts(), None);
        let content = &files[&PathBuf::from("Dockerfile")];
        // USER circleci must appear after the binary is copied — not root at final layer
        let user_pos = content
            .rfind("USER circleci")
            .expect("USER circleci not found");
        let copy_pos = content
            .rfind("COPY --from=builder")
            .expect("COPY --from=builder not found");
        assert!(
            user_pos > copy_pos,
            "USER circleci must appear after COPY --from=builder:\n{content}"
        );
    }

    #[test]
    fn dockerfile_extra_apt_packages_appear_in_final_stage() {
        let cli = make_cli("mytool", vec![]);
        let opts = GenerateOpts {
            apt_packages: vec!["libssl-dev".to_string(), "pkg-config".to_string()],
            ..default_opts()
        };
        let files = generate(&cli, &opts, None);
        let content = &files[&PathBuf::from("Dockerfile")];
        assert!(
            content.contains("libssl-dev"),
            "extra apt package libssl-dev must appear in Dockerfile:\n{content}"
        );
        assert!(
            content.contains("pkg-config"),
            "extra apt package pkg-config must appear in Dockerfile:\n{content}"
        );
    }

    #[test]
    fn dockerfile_final_stage_packages_are_sorted() {
        let cli = make_cli("mytool", vec![]);
        let opts = GenerateOpts {
            apt_packages: vec!["libssl-dev".to_string(), "pkg-config".to_string()],
            ..default_opts()
        };
        let files = generate(&cli, &opts, None);
        let content = &files[&PathBuf::from("Dockerfile")];
        // Isolate the final stage (starts at "FROM debian:12-slim")
        let final_stage = content
            .find("FROM debian:12-slim")
            .map(|pos| &content[pos..])
            .expect("FROM debian:12-slim not found");
        // ca-certificates < git < libssl-dev < pkg-config alphabetically
        let ca_pos = final_stage
            .find("ca-certificates")
            .expect("ca-certificates not found in final stage");
        let git_pos = final_stage
            .find(" git ")
            .expect("git not found in final stage");
        let ssl_pos = final_stage
            .find("libssl-dev")
            .expect("libssl-dev not found in final stage");
        let pkg_pos = final_stage
            .find("pkg-config")
            .expect("pkg-config not found in final stage");
        assert!(
            ca_pos < git_pos,
            "ca-certificates must come before git (sorted):\n{final_stage}"
        );
        assert!(
            git_pos < ssl_pos,
            "git must come before libssl-dev (sorted):\n{final_stage}"
        );
        assert!(
            ssl_pos < pkg_pos,
            "libssl-dev must come before pkg-config (sorted):\n{final_stage}"
        );
    }

    #[test]
    fn dockerfile_no_extra_packages_unchanged() {
        let cli = make_cli("mytool", vec![]);
        let files_default = generate(&cli, &default_opts(), None);
        let opts_empty = GenerateOpts {
            apt_packages: vec![],
            ..default_opts()
        };
        let files_empty = generate(&cli, &opts_empty, None);
        assert_eq!(
            files_default[&PathBuf::from("Dockerfile")],
            files_empty[&PathBuf::from("Dockerfile")],
            "empty apt_packages must produce identical Dockerfile to default"
        );
    }

    #[test]
    fn dockerfile_apt_method_extra_packages() {
        let cli = make_cli("mytool", vec![]);
        let opts = GenerateOpts {
            install_method: InstallMethod::Apt,
            apt_packages: vec!["libssl-dev".to_string()],
            ..default_opts()
        };
        let files = generate(&cli, &opts, None);
        let content = &files[&PathBuf::from("Dockerfile")];
        assert!(
            content.contains("libssl-dev"),
            "extra apt package must appear in Dockerfile (apt method):\n{content}"
        );
    }

    #[test]
    fn dockerfile_apt_includes_git() {
        let cli = make_cli("mytool", vec![]);
        let opts = GenerateOpts {
            install_method: InstallMethod::Apt,
            ..default_opts()
        };
        let files = generate(&cli, &opts, None);
        let content = &files[&PathBuf::from("Dockerfile")];
        assert!(
            content.contains("apt-get install") && content.contains(" git"),
            "Dockerfile (apt) must install git via apt for CircleCI checkout step:\n{content}"
        );
    }

    #[test]
    fn dockerfile_apt() {
        let cli = make_cli("mytool", vec![]);
        let opts = GenerateOpts {
            install_method: InstallMethod::Apt,
            ..default_opts()
        };
        let files = generate(&cli, &opts, None);
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

    // ── command files / scripts ─────────────────────────────────────────────

    #[test]
    fn command_step_uses_script_include() {
        let sub = make_leaf("generate", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let content = &files[&PathBuf::from("src/commands/generate.yml")];
        assert!(
            content.contains("<<include(scripts/generate.sh)>>"),
            "command step must use script include for RC009 compliance:\n{content}"
        );
    }

    #[test]
    fn script_file_generated_for_each_subcommand() {
        let subs = vec![make_leaf("generate", vec![]), make_leaf("validate", vec![])];
        let cli = make_cli("mytool", subs);
        let files = generate(&cli, &default_opts(), None);
        for name in &["generate", "validate"] {
            assert!(
                files.contains_key(&PathBuf::from(format!("src/scripts/{name}.sh"))),
                "missing scripts/{name}.sh"
            );
        }
    }

    #[test]
    fn script_file_contains_required_param_flag() {
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
        let files = generate(&cli, &default_opts(), None);
        let script = &files[&PathBuf::from("src/scripts/generate.sh")];
        assert!(
            script.contains("set -- \"$@\" --orb-path \"${ORB_PATH}\""),
            "script must append required param via env var:\n{script}"
        );
        assert!(
            !script.contains("<<"),
            "script must not contain unsubstituted << parameters >> literals:\n{script}"
        );
    }

    #[test]
    fn script_file_contains_optional_param_conditional() {
        let params = vec![Parameter {
            long_name: "output".to_string(),
            short: None,
            param_type: ParamType::String,
            default: Some("./dist".to_string()),
            required: false,
            description: "Output dir.".to_string(),
        }];
        let sub = make_leaf("generate", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let script = &files[&PathBuf::from("src/scripts/generate.sh")];
        assert!(
            script.contains("[[ -n \"${OUTPUT:-}\" ]]")
                && script.contains("--output \"${OUTPUT}\""),
            "optional param in script must use shell conditional on env var:\n{script}"
        );
    }

    #[test]
    fn script_file_contains_boolean_flag() {
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
        let files = generate(&cli, &default_opts(), None);
        let script = &files[&PathBuf::from("src/scripts/generate.sh")];
        assert!(
            script.contains("[[ \"${FORCE:-false}\" = \"true\" ]]") && script.contains("--force"),
            "boolean flag in script must use shell conditional on env var:\n{script}"
        );
    }

    #[test]
    fn command_run_step_has_environment_block() {
        let params = vec![
            Parameter {
                long_name: "orb_path".to_string(),
                short: None,
                param_type: ParamType::String,
                default: None,
                required: true,
                description: "Path to orb.".to_string(),
            },
            Parameter {
                long_name: "force".to_string(),
                short: None,
                param_type: ParamType::Boolean,
                default: None,
                required: false,
                description: "Force.".to_string(),
            },
        ];
        let sub = make_leaf("generate", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let content = &files[&PathBuf::from("src/commands/generate.yml")];
        assert!(
            content.contains("environment:"),
            "command run step must have environment block:\n{content}"
        );
        assert!(
            content.contains("ORB_PATH: << parameters.orb_path >>"),
            "environment must map ORB_PATH:\n{content}"
        );
        assert!(
            content.contains("FORCE: << parameters.force >>"),
            "environment must map FORCE:\n{content}"
        );
    }

    // ── examples ────────────────────────────────────────────────────────────

    #[test]
    fn example_file_generated() {
        let sub = make_leaf("generate", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        assert!(
            files.contains_key(&PathBuf::from("src/examples/example.yml")),
            "src/examples/example.yml must be generated for RC003 compliance"
        );
        let example = &files[&PathBuf::from("src/examples/example.yml")];
        assert!(
            example.contains("usage:"),
            "example must have a usage block:\n{example}"
        );
        assert!(
            example.contains("my-org/mytool"),
            "example must reference the orb:\n{example}"
        );
    }

    #[test]
    fn example_includes_required_params_with_placeholder() {
        // orb-tools review validates examples: a job with required params must
        // supply them or the example YAML is invalid and the review fails.
        let params = vec![
            Parameter {
                long_name: "orb_name".to_string(),
                short: None,
                param_type: ParamType::String,
                default: None,
                required: true,
                description: "The orb name.".to_string(),
            },
            Parameter {
                long_name: "optional_flag".to_string(),
                short: None,
                param_type: ParamType::Boolean,
                default: Some("false".to_string()),
                required: false,
                description: "An optional boolean.".to_string(),
            },
        ];
        let sub = make_leaf("dosomething", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let example = &files[&PathBuf::from("src/examples/example.yml")];
        assert!(
            example.contains("orb_name:"),
            "example must include required param 'orb_name':\n{example}"
        );
        assert!(
            !example.contains("optional_flag:"),
            "example must not include optional params with defaults:\n{example}"
        );
    }

    // ── RC010: component filenames must be snake_case ───────────────────────

    #[test]
    fn hyphenated_subcommand_generates_snake_case_file_paths() {
        // RC010: orb component names (filenames) must be snake_cased.
        // A subcommand named "do-something" must produce do_something.yml, not do-something.yml.
        let sub = make_leaf("do-something", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        assert!(
            files.contains_key(&PathBuf::from("src/commands/do_something.yml")),
            "command file must use snake_case filename:\n{:?}",
            files.keys().collect::<Vec<_>>()
        );
        assert!(
            files.contains_key(&PathBuf::from("src/jobs/do_something.yml")),
            "job file must use snake_case filename:\n{:?}",
            files.keys().collect::<Vec<_>>()
        );
        assert!(
            files.contains_key(&PathBuf::from("src/scripts/do_something.sh")),
            "script file must use snake_case filename:\n{:?}",
            files.keys().collect::<Vec<_>>()
        );
        assert!(
            !files.contains_key(&PathBuf::from("src/commands/do-something.yml")),
            "command must NOT use hyphenated filename"
        );
    }

    #[test]
    fn hyphenated_subcommand_job_invokes_snake_case_command() {
        // The job's invoke step key must match the command's snake_case filename.
        let sub = make_leaf("do-something", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let job = &files[&PathBuf::from("src/jobs/do_something.yml")];
        assert!(
            job.contains("do_something:"),
            "job invoke step must use snake_case command name:\n{job}"
        );
        assert!(
            !job.contains("do-something:"),
            "job must not reference hyphenated command name:\n{job}"
        );
    }

    #[test]
    fn hyphenated_subcommand_command_includes_snake_case_script() {
        // The command's <<include(scripts/...)>> path must match the snake_case script filename.
        let sub = make_leaf("do-something", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let cmd = &files[&PathBuf::from("src/commands/do_something.yml")];
        assert!(
            cmd.contains("<<include(scripts/do_something.sh)>>"),
            "command must include snake_case script path:\n{cmd}"
        );
    }

    #[test]
    fn hyphenated_subcommand_example_uses_snake_case_job_name() {
        // The example must reference the job by its snake_case name.
        let sub = make_leaf("do-something", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let example = &files[&PathBuf::from("src/examples/example.yml")];
        assert!(
            example.contains("mytool/do_something"),
            "example must use snake_case job name:\n{example}"
        );
        assert!(
            !example.contains("mytool/do-something"),
            "example must not use hyphenated job name:\n{example}"
        );
    }

    #[test]
    fn required_param_renders_without_conditional() {
        // Required params are always appended — no guard needed.
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
        let files = generate(&cli, &default_opts(), None);
        let script = &files[&PathBuf::from("src/scripts/generate.sh")];
        assert!(
            script.contains("set -- \"$@\" --orb-path \"${ORB_PATH}\""),
            "required param must unconditionally append via env var:\n{script}"
        );
        assert!(
            !script.contains("[ -n") || !script.contains("ORB_PATH"),
            "required param must not use conditional guard:\n{script}"
        );
    }

    #[test]
    fn optional_string_param_renders_with_env_var_conditional() {
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
        let files = generate(&cli, &default_opts(), None);
        let script = &files[&PathBuf::from("src/scripts/generate.sh")];
        assert!(
            script.contains("[[ -n \"${OUTPUT:-}\" ]]"),
            "optional param should use shell conditional on env var:\n{script}"
        );
    }

    #[test]
    fn boolean_flag_renders_with_env_var_conditional() {
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
        let files = generate(&cli, &default_opts(), None);
        let script = &files[&PathBuf::from("src/scripts/generate.sh")];
        assert!(
            script.contains("[[ \"${FORCE:-false}\" = \"true\" ]]"),
            "boolean flag must use shell conditional on env var:\n{script}"
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
        let files = generate(&cli, &default_opts(), None);
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

    // ── orb parameter defaults ─────────────────────────────────────────────

    #[test]
    fn boolean_orb_parameter_has_false_default() {
        // Clap boolean flags never emit [default: false] in help text, so p.default is None.
        // The orb must supply default: false so consumers can omit the parameter.
        let params = vec![Parameter {
            long_name: "force".to_string(),
            short: None,
            param_type: ParamType::Boolean,
            default: None,
            required: false,
            description: "Force overwrite.".to_string(),
        }];
        let sub = make_leaf("cmd", params);
        let files = generate(&make_cli("mytool", vec![sub]), &default_opts(), None);
        let content = &files[&PathBuf::from("src/commands/cmd.yml")];
        assert!(
            content.contains("default: false"),
            "boolean param must have default: false so it is optional for orb consumers:\n{content}"
        );
    }

    #[test]
    fn optional_string_no_default_has_empty_string_default() {
        // Optional CLI flag (inside [OPTIONS], no [default:]) must get default: "" so
        // the orb consumer does not have to supply it.  The mustache conditional ensures
        // the flag is not forwarded to the binary when the value is empty.
        let params = vec![Parameter {
            long_name: "output".to_string(),
            short: None,
            param_type: ParamType::String,
            default: None,
            required: false,
            description: "Output path.".to_string(),
        }];
        let sub = make_leaf("cmd", params);
        let files = generate(&make_cli("mytool", vec![sub]), &default_opts(), None);
        let content = &files[&PathBuf::from("src/commands/cmd.yml")];
        // serde_yaml serialises an empty string as ''
        assert!(
            content.contains("default: ''"),
            "optional no-default string param must have default: '' so consumers can omit it:\n{content}"
        );
    }

    #[test]
    fn required_string_no_default_has_no_default_key() {
        // Truly required params (listed outside [OPTIONS] on the Usage line) must NOT
        // have a default: key — CircleCI will then enforce that the consumer supplies them.
        let params = vec![Parameter {
            long_name: "orb_path".to_string(),
            short: None,
            param_type: ParamType::String,
            default: None,
            required: true,
            description: "Path to orb.".to_string(),
        }];
        let sub = make_leaf("cmd", params);
        let files = generate(&make_cli("mytool", vec![sub]), &default_opts(), None);
        let content = &files[&PathBuf::from("src/commands/cmd.yml")];
        assert!(
            !content.contains("default:"),
            "required param must not have a default key:\n{content}"
        );
    }

    // ── job files ───────────────────────────────────────────────────────────

    #[test]
    fn job_references_executor_default() {
        let sub = make_leaf("validate", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
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
        let files = generate(&cli, &default_opts(), None);
        let content = &files[&PathBuf::from("src/jobs/validate.yml")];
        assert!(
            content.contains("checkout"),
            "job missing checkout step:\n{content}"
        );
    }

    #[test]
    fn job_excludes_reserved_circleci_parameter_names() {
        // CircleCI reserves "name" (and others) as job-level parameters.
        // The generator must omit reserved names from job files so orb pack
        // does not reject the output with "Reserved job parameter name: 'name'".
        let params = vec![
            Parameter {
                long_name: "name".to_string(),
                short: Some('n'),
                param_type: ParamType::String,
                default: None,
                required: false,
                description: "Name for the output.".to_string(),
            },
            Parameter {
                long_name: "output".to_string(),
                short: Some('o'),
                param_type: ParamType::String,
                default: Some("./dist".to_string()),
                required: false,
                description: "Output dir.".to_string(),
            },
        ];
        let sub = make_leaf("generate", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);

        // Job must NOT contain `name:` as a parameter key (2-space indent = parameter level).
        let job = &files[&PathBuf::from("src/jobs/generate.yml")];
        assert!(
            !job.contains("\n  name:\n"),
            "job must not contain reserved parameter 'name':\n{job}"
        );

        // Non-reserved param must still appear in the job
        assert!(
            job.contains("output:"),
            "job must still contain non-reserved parameter 'output':\n{job}"
        );
    }

    #[test]
    fn command_renames_restricted_parameter_with_subcommand_prefix() {
        // CircleCI restricts "name" as a command parameter.
        // Rather than silently dropping it, the generator must rename it to
        // "{subcommand}_{param}" so the functionality is preserved under a
        // descriptive, unambiguous name — e.g. "generate" + "name" → "generate_name".
        // The CLI flag emitted in the script stays --name (the original flag).
        let params = vec![
            Parameter {
                long_name: "name".to_string(),
                short: Some('n'),
                param_type: ParamType::String,
                default: Some(String::new()),
                required: false,
                description: "Name for the output.".to_string(),
            },
            Parameter {
                long_name: "output".to_string(),
                short: Some('o'),
                param_type: ParamType::String,
                default: Some("./dist".to_string()),
                required: false,
                description: "Output dir.".to_string(),
            },
        ];
        let sub = make_leaf("generate", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);

        let cmd = &files[&PathBuf::from("src/commands/generate.yml")];
        // Must NOT appear as the bare restricted name
        assert!(
            !cmd.contains("\n  name:\n"),
            "command must not use bare restricted parameter 'name':\n{cmd}"
        );
        // MUST appear under the prefixed name
        assert!(
            cmd.contains("generate_name:"),
            "command must expose 'name' as 'generate_name':\n{cmd}"
        );

        let script = &files[&PathBuf::from("src/scripts/generate.sh")];
        // Script uses the uppercased env var for the renamed orb parameter …
        assert!(
            script.contains("GENERATE_NAME"),
            "script must reference 'GENERATE_NAME' env var:\n{script}"
        );
        // … but still emits the original CLI flag to the binary
        assert!(
            script.contains("--name"),
            "script must still emit '--name' flag to the binary:\n{script}"
        );

        // Non-restricted param must still appear unchanged
        assert!(
            cmd.contains("output:"),
            "command must still contain non-restricted parameter 'output':\n{cmd}"
        );
    }

    // ── circleci CLI installer stage ────────────────────────────────────────

    #[test]
    fn dockerfile_without_circleci_cli_version_has_no_installer_stage() {
        let cli = make_cli("mytool", vec![]);
        let files = generate(&cli, &default_opts(), None);
        let content = &files[&PathBuf::from("Dockerfile")];
        assert!(
            !content.contains("cli-installer"),
            "Dockerfile without --circleci-cli-version must not have cli-installer stage:\n{content}"
        );
        assert!(
            !content.contains("CIRCLECI_CLI_VERSION"),
            "Dockerfile without --circleci-cli-version must not reference CIRCLECI_CLI_VERSION:\n{content}"
        );
    }

    #[test]
    fn dockerfile_with_circleci_cli_version_has_installer_stage() {
        let cli = make_cli("mytool", vec![]);
        let opts = GenerateOpts {
            circleci_cli_version: Some("0.1.36202".to_string()),
            ..default_opts()
        };
        let files = generate(&cli, &opts, None);
        let content = &files[&PathBuf::from("Dockerfile")];
        assert!(
            content.contains("AS cli-installer"),
            "Dockerfile with --circleci-cli-version must have cli-installer stage:\n{content}"
        );
        assert!(
            content.contains("ARG CIRCLECI_CLI_VERSION=0.1.36202"),
            "Dockerfile must pin the specified circleci-cli version:\n{content}"
        );
    }

    #[test]
    fn dockerfile_with_circleci_cli_version_uses_checksum_verification() {
        let cli = make_cli("mytool", vec![]);
        let opts = GenerateOpts {
            circleci_cli_version: Some("0.1.36202".to_string()),
            ..default_opts()
        };
        let files = generate(&cli, &opts, None);
        let content = &files[&PathBuf::from("Dockerfile")];
        assert!(
            content.contains("sha256sum --check"),
            "cli-installer stage must verify checksum:\n{content}"
        );
        assert!(
            !content.contains("| bash"),
            "cli-installer must not use curl|bash:\n{content}"
        );
    }

    #[test]
    fn dockerfile_with_circleci_cli_version_copies_binary_to_final_stage() {
        let cli = make_cli("mytool", vec![]);
        let opts = GenerateOpts {
            circleci_cli_version: Some("0.1.36202".to_string()),
            ..default_opts()
        };
        let files = generate(&cli, &opts, None);
        let content = &files[&PathBuf::from("Dockerfile")];
        assert!(
            content.contains(
                "COPY --from=cli-installer /usr/local/bin/circleci /usr/local/bin/circleci"
            ),
            "final stage must copy circleci binary from cli-installer:\n{content}"
        );
    }

    #[test]
    fn dockerfile_cli_installer_curl_enforces_https_protocol() {
        // SonarQube S6506: curl -L can follow redirects to non-HTTPS URLs.
        // --proto '=https' restricts curl to HTTPS-only, preventing downgrade attacks.
        let cli = make_cli("mytool", vec![]);
        let opts = GenerateOpts {
            circleci_cli_version: Some("0.1.36202".to_string()),
            ..default_opts()
        };
        let files = generate(&cli, &opts, None);
        let content = &files[&PathBuf::from("Dockerfile")];
        let curl_lines: Vec<&str> = content
            .lines()
            .filter(|l| l.trim_start().starts_with("&& curl "))
            .collect();
        assert!(
            !curl_lines.is_empty(),
            "cli-installer stage must contain curl invocations:\n{content}"
        );
        for line in &curl_lines {
            assert!(
                line.contains("--proto '=https'"),
                "curl invocation must enforce HTTPS with --proto '=https' (SonarQube S6506):\n{line}"
            );
        }
    }

    #[test]
    fn dockerfile_with_circleci_cli_version_stage_order() {
        let cli = make_cli("mytool", vec![]);
        let opts = GenerateOpts {
            circleci_cli_version: Some("0.1.36202".to_string()),
            ..default_opts()
        };
        let files = generate(&cli, &opts, None);
        let content = &files[&PathBuf::from("Dockerfile")];
        let builder_pos = content.find("AS builder").expect("builder stage missing");
        let installer_pos = content
            .find("AS cli-installer")
            .expect("cli-installer stage missing");
        let final_from_pos = content.rfind("\nFROM").expect("final FROM missing");
        assert!(
            builder_pos < installer_pos && installer_pos < final_from_pos,
            "cli-installer stage must appear between builder and final stage:\n{content}"
        );
    }

    #[test]
    fn dockerfile_binstall_builder_packages_sorted() {
        // SonarQube S7018: package lists must be sorted alphanumerically.
        let dockerfile = render_dockerfile(
            "mytool",
            &InstallMethod::Binstall,
            "debian:12-slim",
            None,
            &[],
        );
        // builder stage: ca-certificates libssl-dev pkg-config (alphabetical)
        assert!(
            dockerfile.contains("ca-certificates libssl-dev pkg-config"),
            "builder packages must be sorted: ca-certificates libssl-dev pkg-config\n{dockerfile}"
        );
    }

    // ── InstallMethod::Local Dockerfile ────────────────────────────────────

    fn local_opts() -> GenerateOpts {
        GenerateOpts {
            install_method: InstallMethod::Local,
            ..default_opts()
        }
    }

    #[test]
    fn dockerfile_local_uses_copy_not_cargo_install() {
        let dockerfile =
            render_dockerfile("mytool", &InstallMethod::Local, "debian:12-slim", None, &[]);
        assert!(
            dockerfile.contains("COPY mytool /usr/local/bin/mytool"),
            "local method must COPY binary from build context:\n{dockerfile}"
        );
        assert!(
            !dockerfile.contains("cargo install"),
            "local method must not use cargo install:\n{dockerfile}"
        );
    }

    #[test]
    fn dockerfile_local_has_no_rust_builder_stage() {
        let dockerfile =
            render_dockerfile("mytool", &InstallMethod::Local, "debian:12-slim", None, &[]);
        assert!(
            !dockerfile.contains("FROM rust"),
            "local method must not have a Rust builder stage:\n{dockerfile}"
        );
        assert!(
            !dockerfile.contains("AS builder"),
            "local method must not have a builder stage:\n{dockerfile}"
        );
    }

    #[test]
    fn dockerfile_local_runtime_has_ca_certs_and_git() {
        let dockerfile =
            render_dockerfile("mytool", &InstallMethod::Local, "debian:12-slim", None, &[]);
        assert!(
            dockerfile.contains("ca-certificates"),
            "local runtime must install ca-certificates:\n{dockerfile}"
        );
        assert!(
            dockerfile.contains("apt-get install") && dockerfile.contains(" git"),
            "local runtime must install git:\n{dockerfile}"
        );
    }

    #[test]
    fn dockerfile_local_has_circleci_user_and_workdir() {
        let dockerfile =
            render_dockerfile("mytool", &InstallMethod::Local, "debian:12-slim", None, &[]);
        assert!(
            dockerfile.contains("useradd") && dockerfile.contains("circleci"),
            "local method must create circleci user:\n{dockerfile}"
        );
        assert!(
            dockerfile.contains("USER circleci"),
            "local method must set USER circleci:\n{dockerfile}"
        );
        assert!(
            dockerfile.contains("WORKDIR /home/circleci/project"),
            "local method must set WORKDIR:\n{dockerfile}"
        );
    }

    #[test]
    fn dockerfile_local_does_not_run_as_root() {
        let dockerfile =
            render_dockerfile("mytool", &InstallMethod::Local, "debian:12-slim", None, &[]);
        let copy_pos = dockerfile.find("COPY mytool").expect("COPY not found");
        let user_pos = dockerfile.find("USER circleci").expect("USER not found");
        assert!(
            user_pos > copy_pos,
            "USER circleci must appear after COPY:\n{dockerfile}"
        );
    }

    #[test]
    fn dockerfile_local_with_circleci_cli_includes_installer_stage() {
        let dockerfile = render_dockerfile(
            "mytool",
            &InstallMethod::Local,
            "debian:12-slim",
            Some("0.1.36202"),
            &[],
        );
        assert!(
            dockerfile.contains("AS cli-installer"),
            "local + circleci_cli must include cli-installer stage:\n{dockerfile}"
        );
        assert!(
            dockerfile.contains("COPY --from=cli-installer /usr/local/bin/circleci"),
            "local + circleci_cli must copy circleci binary:\n{dockerfile}"
        );
    }

    #[test]
    fn dockerfile_local_generate_produces_dockerfile() {
        let cli = make_cli("mytool", vec![]);
        let files = generate(&cli, &local_opts(), None);
        assert!(
            files.contains_key(&PathBuf::from("Dockerfile")),
            "generate with Local install must produce a Dockerfile"
        );
        let content = &files[&PathBuf::from("Dockerfile")];
        assert!(
            content.contains("COPY mytool /usr/local/bin/mytool"),
            "generated Dockerfile must COPY binary:\n{content}"
        );
    }

    // ── add-workspace-to-path.sh always generated ───────────────────────────

    #[test]
    fn add_workspace_to_path_script_always_generated() {
        // Every generated orb includes jobs with an attach_workspace conditional that
        // references <<include(scripts/add-workspace-to-path.sh)>>.  The script must
        // always be generated so `circleci orb pack` does not fail with "could not open".
        let sub = make_leaf("generate", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        assert!(
            files.contains_key(&PathBuf::from("src/scripts/add-workspace-to-path.sh")),
            "add-workspace-to-path.sh must always be generated; orb pack fails without it"
        );
    }

    #[test]
    fn add_workspace_script_exports_path() {
        let sub = make_leaf("generate", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let script = &files[&PathBuf::from("src/scripts/add-workspace-to-path.sh")];
        assert!(
            script.contains("PATH"),
            "add-workspace-to-path.sh must export the workspace root onto PATH:\n{script}"
        );
        assert!(
            script.contains("WORKSPACE_ROOT"),
            "add-workspace-to-path.sh must use the WORKSPACE_ROOT env var:\n{script}"
        );
    }

    // ── set_https_remote command + script generation ────────────────────────

    #[test]
    fn set_https_remote_command_file_generated_when_git_push_subcommands_set() {
        let sub = make_leaf("save", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let opts = GenerateOpts {
            git_push_subcommands: vec!["save".to_string()],
            ..default_opts()
        };
        let files = generate(&cli, &opts, None);
        assert!(
            files.contains_key(&PathBuf::from("src/commands/set_https_remote.yml")),
            "set_https_remote command file must be generated when git_push_subcommands is set"
        );
    }

    #[test]
    fn set_https_remote_script_file_generated_when_git_push_subcommands_set() {
        let sub = make_leaf("save", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let opts = GenerateOpts {
            git_push_subcommands: vec!["save".to_string()],
            ..default_opts()
        };
        let files = generate(&cli, &opts, None);
        assert!(
            files.contains_key(&PathBuf::from("src/scripts/set_https_remote.sh")),
            "set_https_remote script file must be generated when git_push_subcommands is set"
        );
    }

    #[test]
    fn set_https_remote_command_contains_include_script() {
        let sub = make_leaf("save", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let opts = GenerateOpts {
            git_push_subcommands: vec!["save".to_string()],
            ..default_opts()
        };
        let files = generate(&cli, &opts, None);
        let cmd = &files[&PathBuf::from("src/commands/set_https_remote.yml")];
        assert!(
            cmd.contains("<<include(scripts/set_https_remote.sh)>>"),
            "set_https_remote command must include the script:\n{cmd}"
        );
    }

    #[test]
    fn set_https_remote_script_unsets_insteadof_rule() {
        let sub = make_leaf("save", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let opts = GenerateOpts {
            git_push_subcommands: vec!["save".to_string()],
            ..default_opts()
        };
        let files = generate(&cli, &opts, None);
        let script = &files[&PathBuf::from("src/scripts/set_https_remote.sh")];
        assert!(
            script.contains("insteadOf") || script.contains("unset"),
            "set_https_remote script must unset the CircleCI SSH insteadOf rule:\n{script}"
        );
        assert!(
            script.contains("git remote set-url"),
            "set_https_remote script must set the remote URL to HTTPS:\n{script}"
        );
    }

    #[test]
    fn set_https_remote_not_generated_when_no_push_subcommands() {
        let sub = make_leaf("generate", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        assert!(
            !files.contains_key(&PathBuf::from("src/commands/set_https_remote.yml")),
            "set_https_remote command must NOT be generated when git_push_subcommands is empty"
        );
        assert!(
            !files.contains_key(&PathBuf::from("src/scripts/set_https_remote.sh")),
            "set_https_remote script must NOT be generated when git_push_subcommands is empty"
        );
    }

    // ── set_https_remote in push jobs ───────────────────────────────────────

    #[test]
    fn push_subcommand_job_has_set_https_remote_step() {
        let sub = make_leaf("save", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let opts = GenerateOpts {
            git_push_subcommands: vec!["save".to_string()],
            ..default_opts()
        };
        let files = generate(&cli, &opts, None);
        let job = &files[&PathBuf::from("src/jobs/save.yml")];
        assert!(
            job.contains("set_https_remote"),
            "save job must include set_https_remote step when listed in git_push_subcommands:\n{job}"
        );
    }

    #[test]
    fn push_subcommand_job_set_https_remote_placed_between_checkout_and_invoke() {
        let sub = make_leaf("save", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let opts = GenerateOpts {
            git_push_subcommands: vec!["save".to_string()],
            ..default_opts()
        };
        let files = generate(&cli, &opts, None);
        let job = &files[&PathBuf::from("src/jobs/save.yml")];
        let checkout_pos = job.find("- checkout").expect("checkout step missing");
        let https_pos = job
            .find("set_https_remote")
            .expect("set_https_remote step missing");
        let invoke_pos = job.find("save:").expect("save invoke step missing");
        assert!(
            checkout_pos < https_pos && https_pos < invoke_pos,
            "set_https_remote must appear after checkout and before the invoke step:\n{job}"
        );
    }

    #[test]
    fn non_push_subcommand_job_has_no_set_https_remote() {
        let sub = make_leaf("validate", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let opts = GenerateOpts {
            git_push_subcommands: vec!["save".to_string()],
            ..default_opts()
        };
        let files = generate(&cli, &opts, None);
        let job = &files[&PathBuf::from("src/jobs/validate.yml")];
        assert!(
            !job.contains("set_https_remote"),
            "validate job must not have set_https_remote (not a push subcommand):\n{job}"
        );
    }

    #[test]
    fn job_has_attach_workspace_parameter() {
        let sub = make_leaf("generate", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let job = &files[&PathBuf::from("src/jobs/generate.yml")];
        assert!(
            job.contains("attach_workspace:"),
            "job must declare attach_workspace parameter:\n{job}"
        );
    }

    #[test]
    fn job_has_workspace_root_parameter() {
        let sub = make_leaf("generate", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let job = &files[&PathBuf::from("src/jobs/generate.yml")];
        assert!(
            job.contains("workspace_root:"),
            "job must declare workspace_root parameter:\n{job}"
        );
    }

    #[test]
    fn job_workspace_root_default_is_tmp_workspace() {
        let sub = make_leaf("generate", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let job = &files[&PathBuf::from("src/jobs/generate.yml")];
        assert!(
            job.contains("/tmp/workspace"),
            "workspace_root default must be /tmp/workspace:\n{job}"
        );
    }

    #[test]
    fn job_has_conditional_attach_workspace_step() {
        let sub = make_leaf("generate", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let job = &files[&PathBuf::from("src/jobs/generate.yml")];
        assert!(
            job.contains("condition: << parameters.attach_workspace >>"),
            "job must have conditional step gated on attach_workspace parameter:\n{job}"
        );
    }

    #[test]
    fn script_file_ends_with_newline() {
        let sub = make_leaf("generate", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let script = &files[&PathBuf::from("src/scripts/generate.sh")];
        assert!(
            script.ends_with('\n'),
            "generated script must end with a newline:\n{script:?}"
        );
    }

    #[test]
    fn job_with_no_push_subcommands_has_no_set_https_remote() {
        let sub = make_leaf("save", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let job = &files[&PathBuf::from("src/jobs/save.yml")];
        assert!(
            !job.contains("set_https_remote"),
            "save job must not include set_https_remote when git_push_subcommands is empty:\n{job}"
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
        let files = generate(&cli, &default_opts(), None);
        for name in &["generate", "validate", "diff"] {
            assert!(
                files.contains_key(&PathBuf::from(format!("src/commands/{name}.yml"))),
                "missing commands/{name}.yml"
            );
            assert!(
                files.contains_key(&PathBuf::from(format!("src/jobs/{name}.yml"))),
                "missing jobs/{name}.yml"
            );
            assert!(
                files.contains_key(&PathBuf::from(format!("src/scripts/{name}.sh"))),
                "missing scripts/{name}.sh"
            );
        }
    }

    // ── Phase 2: config-driven suppression, param overrides, orbs section ─────

    #[test]
    fn suppressed_subcommand_has_no_job_file() {
        use crate::orb_config::{OrbConfig, SubcommandConfig};
        use indexmap::IndexMap;

        let sub = make_leaf("help", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let mut subcommands = IndexMap::new();
        subcommands.insert(
            "help".to_string(),
            SubcommandConfig {
                generate_job: Some(false),
                param: None,
            },
        );
        let config = OrbConfig {
            subcommand: Some(subcommands),
            ..OrbConfig::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        assert!(
            !files.contains_key(&PathBuf::from("src/jobs/help.yml")),
            "suppressed subcommand must not generate a job file"
        );
    }

    #[test]
    fn suppressed_subcommand_still_has_command_file() {
        use crate::orb_config::{OrbConfig, SubcommandConfig};
        use indexmap::IndexMap;

        let sub = make_leaf("help", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let mut subcommands = IndexMap::new();
        subcommands.insert(
            "help".to_string(),
            SubcommandConfig {
                generate_job: Some(false),
                param: None,
            },
        );
        let config = OrbConfig {
            subcommand: Some(subcommands),
            ..OrbConfig::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        assert!(
            files.contains_key(&PathBuf::from("src/commands/help.yml")),
            "suppressed subcommand must still generate a command file"
        );
    }

    #[test]
    fn suppressed_subcommand_not_in_example_yml() {
        use crate::orb_config::{OrbConfig, SubcommandConfig};
        use indexmap::IndexMap;

        let subs = vec![make_leaf("generate", vec![]), make_leaf("help", vec![])];
        let cli = make_cli("mytool", subs);
        let mut subcommands = IndexMap::new();
        subcommands.insert(
            "help".to_string(),
            SubcommandConfig {
                generate_job: Some(false),
                param: None,
            },
        );
        let config = OrbConfig {
            subcommand: Some(subcommands),
            ..OrbConfig::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        let example = &files[&PathBuf::from("src/examples/example.yml")];
        assert!(
            !example.contains("help:"),
            "suppressed subcommand must not appear in example.yml:\n{example}"
        );
    }

    #[test]
    fn param_override_changes_default_in_generated_job() {
        use crate::help_parser::types::Parameter;
        use crate::orb_config::{OrbConfig, ParamOverride, SubcommandConfig};
        use indexmap::IndexMap;

        let orb_path_param = Parameter {
            long_name: "orb_path".to_string(),
            short: None,
            param_type: ParamType::String,
            default: Some("src/@orb.yml".to_string()),
            required: false,
            description: "Path to orb file.".to_string(),
        };
        let sub = make_leaf("generate", vec![orb_path_param]);
        let cli = make_cli("mytool", vec![sub]);

        let mut param_overrides = IndexMap::new();
        param_overrides.insert(
            "orb_path".to_string(),
            ParamOverride {
                default: Some("custom/@orb.yml".to_string()),
            },
        );
        let mut subcommands = IndexMap::new();
        subcommands.insert(
            "generate".to_string(),
            SubcommandConfig {
                generate_job: None,
                param: Some(param_overrides),
            },
        );
        let config = OrbConfig {
            subcommand: Some(subcommands),
            ..OrbConfig::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        let job = &files[&PathBuf::from("src/jobs/generate.yml")];
        assert!(
            job.contains("custom/@orb.yml"),
            "param override must change default in generated job:\n{job}"
        );
        assert!(
            !job.contains("src/@orb.yml"),
            "original default must be replaced by param override:\n{job}"
        );
    }

    #[test]
    fn orb_yml_has_orbs_section_when_config_provides_orbs() {
        use crate::orb_config::OrbConfig;
        use indexmap::IndexMap;

        let cli = make_cli("mytool", vec![]);
        let mut orbs = IndexMap::new();
        orbs.insert(
            "orb-tools".to_string(),
            "circleci/orb-tools@12.3.3".to_string(),
        );
        let config = OrbConfig {
            orbs: Some(orbs),
            ..OrbConfig::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        let orb_yml = &files[&PathBuf::from("src/@orb.yml")];
        assert!(
            orb_yml.contains("orbs:"),
            "@orb.yml must include orbs: section when config provides orbs:\n{orb_yml}"
        );
        assert!(
            orb_yml.contains("orb-tools: circleci/orb-tools@12.3.3"),
            "@orb.yml must include the orb reference:\n{orb_yml}"
        );
    }

    #[test]
    fn generate_with_no_config_matches_default_behavior() {
        let sub = make_leaf("generate", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        assert!(
            files.contains_key(&PathBuf::from("src/jobs/generate.yml")),
            "no-config generate must still produce job file"
        );
        assert!(
            files.contains_key(&PathBuf::from("src/commands/generate.yml")),
            "no-config generate must still produce command file"
        );
    }

    // ── Phase 3: job_group composed job generation ─────────────────────────

    fn make_param(name: &str, default: Option<&str>, required: bool) -> Parameter {
        Parameter {
            long_name: name.to_string(),
            short: None,
            param_type: ParamType::String,
            default: default.map(String::from),
            required,
            description: format!("{name} param."),
        }
    }

    #[test]
    fn job_group_file_created_for_each_group() {
        use crate::orb_config::{JobGroup, OrbConfig};

        let subs = vec![make_leaf("generate", vec![]), make_leaf("validate", vec![])];
        let cli = make_cli("mytool", subs);
        let config = OrbConfig {
            job_group: Some(vec![JobGroup {
                name: "sync".to_string(),
                description: Some("Regenerate and validate".to_string()),
                steps: vec!["generate".to_string(), "validate".to_string()],
                params: None,
            }]),
            ..OrbConfig::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        assert!(
            files.contains_key(&PathBuf::from("src/jobs/sync.yml")),
            "job_group must produce src/jobs/sync.yml"
        );
    }

    #[test]
    fn job_group_contains_both_steps_in_order() {
        use crate::orb_config::{JobGroup, OrbConfig};

        let subs = vec![make_leaf("generate", vec![]), make_leaf("validate", vec![])];
        let cli = make_cli("mytool", subs);
        let config = OrbConfig {
            job_group: Some(vec![JobGroup {
                name: "sync".to_string(),
                description: None,
                steps: vec!["generate".to_string(), "validate".to_string()],
                params: None,
            }]),
            ..OrbConfig::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        let job = &files[&PathBuf::from("src/jobs/sync.yml")];
        let gen_pos = job.find("generate:").expect("generate step missing");
        let val_pos = job.find("validate:").expect("validate step missing");
        assert!(
            gen_pos < val_pos,
            "generate step must appear before validate step:\n{job}"
        );
    }

    #[test]
    fn job_group_description_in_job_yaml() {
        use crate::orb_config::{JobGroup, OrbConfig};

        let subs = vec![make_leaf("generate", vec![]), make_leaf("validate", vec![])];
        let cli = make_cli("mytool", subs);
        let config = OrbConfig {
            job_group: Some(vec![JobGroup {
                name: "sync".to_string(),
                description: Some("Regenerate and validate".to_string()),
                steps: vec!["generate".to_string(), "validate".to_string()],
                params: None,
            }]),
            ..OrbConfig::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        let job = &files[&PathBuf::from("src/jobs/sync.yml")];
        assert!(
            job.contains("Regenerate and validate"),
            "job_group description must appear in job YAML:\n{job}"
        );
    }

    #[test]
    fn job_group_shared_param_appears_in_merged_job() {
        use crate::orb_config::{JobGroup, OrbConfig};

        let shared_param = make_param("orb_path", Some("src/@orb.yml"), false);
        let subs = vec![
            make_leaf("generate", vec![shared_param.clone()]),
            make_leaf("validate", vec![shared_param]),
        ];
        let cli = make_cli("mytool", subs);
        let config = OrbConfig {
            job_group: Some(vec![JobGroup {
                name: "sync".to_string(),
                description: None,
                steps: vec!["generate".to_string(), "validate".to_string()],
                params: None,
            }]),
            ..OrbConfig::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        let job = &files[&PathBuf::from("src/jobs/sync.yml")];
        assert!(
            job.contains("orb_path:"),
            "shared param orb_path must appear in merged job:\n{job}"
        );
    }

    #[test]
    fn job_group_explicit_params_restricts_to_listed_params() {
        use crate::orb_config::{JobGroup, OrbConfig};

        let subs = vec![
            make_leaf(
                "generate",
                vec![
                    make_param("orb_path", Some("src/@orb.yml"), false),
                    make_param("format", Some("yaml"), false),
                ],
            ),
            make_leaf(
                "validate",
                vec![make_param("orb_path", Some("src/@orb.yml"), false)],
            ),
        ];
        let cli = make_cli("mytool", subs);
        let config = OrbConfig {
            job_group: Some(vec![JobGroup {
                name: "sync".to_string(),
                description: None,
                steps: vec!["generate".to_string(), "validate".to_string()],
                params: Some(vec!["orb_path".to_string()]),
            }]),
            ..OrbConfig::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        let job = &files[&PathBuf::from("src/jobs/sync.yml")];
        assert!(
            job.contains("orb_path:"),
            "explicitly listed orb_path must appear in job:\n{job}"
        );
        assert!(
            !job.contains("format:"),
            "non-listed format param must not appear in job:\n{job}"
        );
    }

    // ── Phase 4: extra_job verbatim YAML generation ────────────────────────

    #[test]
    fn extra_job_file_created_at_jobs_path() {
        use crate::orb_config::{ExtraJob, OrbConfig};

        let cli = make_cli("mytool", vec![]);
        let config = OrbConfig {
            extra_job: Some(vec![ExtraJob {
                name: "ensure_registered".to_string(),
                yaml: "description: Ensure registered\nexecutor: orb-tools/default\n".to_string(),
            }]),
            ..OrbConfig::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        assert!(
            files.contains_key(&PathBuf::from("src/jobs/ensure_registered.yml")),
            "extra_job must produce src/jobs/ensure_registered.yml"
        );
    }

    #[test]
    fn extra_job_yaml_emitted_verbatim() {
        use crate::orb_config::{ExtraJob, OrbConfig};

        let yaml_content =
            "description: Ensure registered\nexecutor: orb-tools/default\nsteps:\n  - run: echo ok\n";
        let cli = make_cli("mytool", vec![]);
        let config = OrbConfig {
            extra_job: Some(vec![ExtraJob {
                name: "ensure_registered".to_string(),
                yaml: yaml_content.to_string(),
            }]),
            ..OrbConfig::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        let job = &files[&PathBuf::from("src/jobs/ensure_registered.yml")];
        assert!(
            job.contains("description: Ensure registered"),
            "extra_job yaml must be emitted verbatim:\n{job}"
        );
        assert!(
            job.contains("executor: orb-tools/default"),
            "extra_job yaml must contain executor:\n{job}"
        );
    }

    #[test]
    fn extra_job_file_ends_with_newline() {
        use crate::orb_config::{ExtraJob, OrbConfig};

        let cli = make_cli("mytool", vec![]);
        let config = OrbConfig {
            extra_job: Some(vec![ExtraJob {
                name: "my_job".to_string(),
                yaml: "description: A job".to_string(),
            }]),
            ..OrbConfig::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        let job = &files[&PathBuf::from("src/jobs/my_job.yml")];
        assert!(
            job.ends_with('\n'),
            "extra_job output must end with a newline:\n{job:?}"
        );
    }

    #[test]
    fn extra_job_hyphenated_name_preserved_as_filename() {
        use crate::orb_config::{ExtraJob, OrbConfig};

        let cli = make_cli("mytool", vec![]);
        let config = OrbConfig {
            extra_job: Some(vec![ExtraJob {
                name: "ensure-registered".to_string(),
                yaml: "description: Test".to_string(),
            }]),
            ..OrbConfig::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        assert!(
            files.contains_key(&PathBuf::from("src/jobs/ensure-registered.yml")),
            "extra_job with hyphenated name must use hyphen in filename"
        );
    }
}
