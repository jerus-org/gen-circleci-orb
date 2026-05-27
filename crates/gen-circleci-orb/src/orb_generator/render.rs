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
        render_subcommand(sub, &cli.binary_name, opts, &mut files);
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

    // examples/example.yml (RC003)
    files.insert(
        PathBuf::from("src/examples/example.yml"),
        render_example(cli, opts),
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

fn render_subcommand(
    sub: &SubCommand,
    binary: &str,
    opts: &GenerateOpts,
    files: &mut HashMap<PathBuf, String>,
) {
    if sub.is_leaf {
        let snake = sub.name.replace('-', "_");
        files.insert(
            PathBuf::from(format!("src/commands/{snake}.yml")),
            render_command(sub),
        );
        files.insert(
            PathBuf::from(format!("src/jobs/{snake}.yml")),
            render_job(sub, opts),
        );
        files.insert(
            PathBuf::from(format!("src/scripts/{snake}.sh")),
            render_command_script_content(sub, binary),
        );
    }
    for child in &sub.subcommands {
        render_subcommand(child, binary, opts, files);
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

fn render_job(sub: &SubCommand, opts: &GenerateOpts) -> String {
    let parameters = build_orb_parameters(sub, RESERVED_JOB_PARAMS);
    let checkout_step: serde_yaml::Value = serde_yaml::Value::String("checkout".to_string());
    let invoke_step = build_invoke_step(sub, RESERVED_JOB_PARAMS);
    let mut steps = vec![checkout_step];
    if opts.git_push_subcommands.contains(&sub.name) {
        steps.push(serde_yaml::Value::String("set_https_remote".to_string()));
    }
    steps.push(invoke_step);
    let job = OrbJob {
        description: format!("Run {} {} in a dedicated job.", sub.name, "command"),
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

fn render_example(cli: &CliDefinition, opts: &GenerateOpts) -> String {
    let namespace = opts
        .namespaces
        .first()
        .map(String::as_str)
        .unwrap_or("my-org");
    let binary = &cli.binary_name;
    // Use the first leaf subcommand name for the example job, or the binary name if none.
    let first_sub = cli.subcommands.iter().find(|s| s.is_leaf);
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
            out.push_str(&format!("            {}: my-value\n", p.long_name));
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
    fn dockerfile_binstall_uses_multistage_build() {
        let cli = make_cli("mytool", vec![]);
        let files = generate(&cli, &default_opts());
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
        let files = generate(&cli, &default_opts());
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
        let files = generate(&cli, &default_opts());
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
        let files = generate(&cli, &default_opts());
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
        let files = generate(&cli, &default_opts());
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
        let files = generate(&cli, &opts);
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
        let files = generate(&cli, &opts);
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
        let files_default = generate(&cli, &default_opts());
        let opts_empty = GenerateOpts {
            apt_packages: vec![],
            ..default_opts()
        };
        let files_empty = generate(&cli, &opts_empty);
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
        let files = generate(&cli, &opts);
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
        let files = generate(&cli, &opts);
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

    // ── command files / scripts ─────────────────────────────────────────────

    #[test]
    fn command_step_uses_script_include() {
        let sub = make_leaf("generate", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts());
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
        let files = generate(&cli, &default_opts());
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
        let files = generate(&cli, &default_opts());
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
        let files = generate(&cli, &default_opts());
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
        let files = generate(&cli, &default_opts());
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
        let files = generate(&cli, &default_opts());
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
        let files = generate(&cli, &default_opts());
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
        let files = generate(&cli, &default_opts());
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
        let files = generate(&cli, &default_opts());
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
        let files = generate(&cli, &default_opts());
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
        let files = generate(&cli, &default_opts());
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
        let files = generate(&cli, &default_opts());
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
        let files = generate(&cli, &default_opts());
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
        let files = generate(&cli, &default_opts());
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
        let files = generate(&cli, &default_opts());
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
        let files = generate(&make_cli("mytool", vec![sub]), &default_opts());
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
        let files = generate(&make_cli("mytool", vec![sub]), &default_opts());
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
        let files = generate(&make_cli("mytool", vec![sub]), &default_opts());
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
        let files = generate(&cli, &default_opts());

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
        let files = generate(&cli, &default_opts());

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
        let files = generate(&cli, &default_opts());
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
        let files = generate(&cli, &opts);
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
        let files = generate(&cli, &opts);
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
        let files = generate(&cli, &opts);
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
        let files = generate(&cli, &opts);
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
        let files = generate(&cli, &opts);
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

    // ── set_https_remote in push jobs ───────────────────────────────────────

    #[test]
    fn push_subcommand_job_has_set_https_remote_step() {
        let sub = make_leaf("save", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let opts = GenerateOpts {
            git_push_subcommands: vec!["save".to_string()],
            ..default_opts()
        };
        let files = generate(&cli, &opts);
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
        let files = generate(&cli, &opts);
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
        let files = generate(&cli, &opts);
        let job = &files[&PathBuf::from("src/jobs/validate.yml")];
        assert!(
            !job.contains("set_https_remote"),
            "validate job must not have set_https_remote (not a push subcommand):\n{job}"
        );
    }

    #[test]
    fn script_file_ends_with_newline() {
        let sub = make_leaf("generate", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts());
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
        let files = generate(&cli, &default_opts());
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
            assert!(
                files.contains_key(&PathBuf::from(format!("src/scripts/{name}.sh"))),
                "missing scripts/{name}.sh"
            );
        }
    }
}
