use super::types::{DockerImage, OrbCommand, OrbExecutor, OrbJob, OrbParameter};
use crate::commands::generate::InstallMethod;
use crate::help_parser::types::{CliDefinition, ParamKind, ParamType, Parameter, SubCommand};
use crate::orb_config::OrbConfig;
use indexmap::IndexMap;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct GenerateOpts {
    pub namespaces: Vec<String>,
    pub install_method: InstallMethod,
    pub base_image: String,
    /// Image for the Rust `builder` stage that `cargo install`s the binary
    /// (Binstall method). Config-driven so a pinned `…@sha256:…` digest can be
    /// kept in gen-circleci-orb.toml and tracked by Renovate, rather than being
    /// stripped on every regeneration.
    pub builder_image: String,
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
    /// Extra cargo tools to install into the executor image via cargo-binstall
    /// in the builder stage, with their binaries copied into the runtime.
    /// Binstall install method only. Each triple is
    /// `(crate_name, binary_name, version)` — `version` is `None` for the
    /// default floating install, or `Some(v)` to pin `crate@v` on the
    /// `cargo binstall` command line. Pre-validated by
    /// `commands::generate::validate_cargo_tool_entries`.
    pub cargo_tools: Vec<(String, String, Option<String>)>,
    /// How long the generated Dockerfile waits for crates.io to serve the
    /// version being released.
    pub crate_wait: CrateWait,
}

/// The generated Dockerfile's crates.io propagation gate.
///
/// The container is built from the crate that was *just* published, so the
/// build races the sparse index. The gate retries a bounded number of times and
/// then fails loudly — never silently installing the previous version (#200).
/// Only the size of the window is tunable: a release that outruns it stalls
/// half-published, with the crate on crates.io and no container or orb (#236).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrateWait {
    /// How many times to try the install before failing loudly.
    pub attempts: u32,
    /// Seconds to wait between tries.
    pub seconds: u32,
}

/// 40 x 15s — 39 sleeps, so ~9m45s of waiting. Twice the window that proved too
/// short on the 0.1.4 release.
///
/// The numbers live in `orb_config`, which is where a consumer sets them: the
/// same values seed `[orb]`'s defaults, so a saved config and an unconfigured
/// build cannot disagree about what the window is.
impl Default for CrateWait {
    fn default() -> Self {
        Self {
            attempts: crate::orb_config::DEFAULT_CRATE_WAIT_ATTEMPTS,
            seconds: crate::orb_config::DEFAULT_CRATE_WAIT_SECONDS,
        }
    }
}

/// Generate all orb artifact strings keyed by their relative output path.
pub fn generate(
    cli: &CliDefinition,
    opts: &GenerateOpts,
    config: Option<&OrbConfig>,
) -> HashMap<PathBuf, String> {
    // Merge each subcommand's repeatable verbose/quiet pair before anything
    // else reads `cli.subcommands` — every consumer below (job/command param
    // building, script codegen, examples) then sees the merged shape
    // automatically, with nothing else in this function aware of #348.
    let cli = normalize_verbosity_flags(cli, config);
    let cli = &cli;

    // gen-circleci-orb#358: computed once, threaded through every leaf's
    // rendering so a colliding bare name is qualified consistently
    // everywhere it's used (file names, the job's own invoke-step key).
    let effective_names = compute_effective_names(cli, config);

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
        render_subcommand(
            sub,
            std::slice::from_ref(&sub.name),
            &cli.binary_name,
            opts,
            config,
            &effective_names,
            &mut files,
        );
    }

    // Dockerfile
    files.insert(
        PathBuf::from("Dockerfile"),
        render_dockerfile(&cli.binary_name, opts),
    );

    // src/jobs/<name>.yml for each job_group in config
    if let Some(groups) = config.and_then(|c| c.job_group.as_ref()) {
        for group in groups {
            let snake = group.name.replace('-', "_");
            // render_job_group may also emit run-step scripts into `files`.
            let job_yaml = render_job_group(group, cli, config, &effective_names, &mut files);
            files.insert(PathBuf::from(format!("src/jobs/{snake}.yml")), job_yaml);
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
    // attach_workspace conditional step via <<include(scripts/add-workspace-to-path.sh)>>.
    // Must append to $BASH_ENV: a bare `export` only affects its own step, so the
    // attached workspace binary would not be on PATH for the subsequent step.
    files.insert(
        PathBuf::from("src/scripts/add-workspace-to-path.sh"),
        "echo \"export PATH=\\\"${WORKSPACE_ROOT}:\\$PATH\\\"\" >> \"$BASH_ENV\"\n".to_string(),
    );

    // resolve_workspace_param.sh — generated only when at least one param,
    // anywhere in the tree, is configured `workspace_sourced = true`.
    // Unlike add-workspace-to-path.sh, this script is genuinely unused by
    // the (overwhelmingly common) literal-only case, so it stays absent
    // rather than cluttering every consumer's orb with an unreferenced file.
    if any_workspace_sourced_param(&cli.subcommands, &effective_names, "", config) {
        files.insert(
            PathBuf::from("src/scripts/resolve_workspace_param.sh"),
            RESOLVE_WORKSPACE_PARAM_SCRIPT.to_string(),
        );
    }

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

/// The enum values a merged `log_level` parameter offers, in declaration
/// order: named for the flag count they match (`v`, `vv`, `vvv`, `vvvv` —
/// clap-verbosity-flag's own -v/-vv/-vvv/-vvvv), never absolute names like
/// `warn`/`info`/`debug` — the generator has no way to know which level a
/// given CLI's `clap-verbosity-flag` is actually configured to default to
/// (`--help` text doesn't say), so an absolute name would be a guess that's
/// right for some consumers and wrong for others (#348).
const LOG_LEVEL_VALUES: &[&str] = &["quiet", "default", "v", "vv", "vvv", "vvvv"];

/// The synthetic parameter name a merged verbose/quiet pair becomes.
/// `render_command_script_content` special-cases this exact name to emit a
/// `case` translation instead of the generic enum handling — reserved the
/// same way `attach_workspace`/`workspace_root` are elsewhere in this file.
const LOG_LEVEL_PARAM: &str = "log_level";

/// Recursively merges each subcommand's repeatable `verbose`/`quiet` pair —
/// clap-verbosity-flag's own two Count args, used org-wide — into one
/// `log_level` enum parameter, so the generated orb never lets a consumer
/// set both independently and get a self-canceling combination (#348).
fn normalize_verbosity_flags(cli: &CliDefinition, config: Option<&OrbConfig>) -> CliDefinition {
    CliDefinition {
        subcommands: cli
            .subcommands
            .iter()
            .map(|s| normalize_subcommand(s, config))
            .collect(),
        ..cli.clone()
    }
}

/// `..sub.clone()` (rather than naming every field) so a future field added
/// to `SubCommand` is carried through by default instead of silently
/// dropping out of the normalized tree.
fn normalize_subcommand(sub: &SubCommand, config: Option<&OrbConfig>) -> SubCommand {
    // Merging is the default; a consumer whose verbose/quiet aren't
    // clap-verbosity-flag's linked counter pair can opt out per subcommand.
    let merge_enabled = config
        .and_then(|c| c.subcommand.as_ref())
        .and_then(|sc| sc.get(&sub.name))
        .and_then(|sc_config| sc_config.merge_verbosity)
        .unwrap_or(true);
    SubCommand {
        parameters: if merge_enabled {
            merge_verbosity_pair(&sub.parameters)
        } else {
            sub.parameters.clone()
        },
        subcommands: sub
            .subcommands
            .iter()
            .map(|s| normalize_subcommand(s, config))
            .collect(),
        ..sub.clone()
    }
}

/// When exactly one repeatable `verbose` and one repeatable `quiet`
/// parameter are both present, replace both with a single `log_level` enum
/// at the earlier of their two positions (keeps declaration order stable).
/// A repeatable flag without its pair is left as-is — nothing to merge it
/// into without guessing at a counterpart that isn't there.
fn merge_verbosity_pair(parameters: &[Parameter]) -> Vec<Parameter> {
    let verbose_idx = parameters
        .iter()
        .position(|p| p.repeatable && p.long_name == "verbose");
    let quiet_idx = parameters
        .iter()
        .position(|p| p.repeatable && p.long_name == "quiet");
    let (Some(verbose_idx), Some(quiet_idx)) = (verbose_idx, quiet_idx) else {
        return parameters.to_vec();
    };
    let insert_at = verbose_idx.min(quiet_idx);

    let log_level = Parameter {
        long_name: LOG_LEVEL_PARAM.to_string(),
        short: None,
        kind: ParamKind::Long,
        param_type: ParamType::Enum(LOG_LEVEL_VALUES.iter().map(ToString::to_string).collect()),
        default: Some("default".to_string()),
        required: false,
        description: "Logging verbosity: consolidates this tool's clap-verbosity-flag \
                       -q/--quiet and -v/--verbose repeat-counters (its own linked pair) into \
                       one selector, so they can't be set independently and cancel out. \
                       clap-verbosity-flag escalates off -> error -> warn -> info -> debug -> \
                       trace, typically starting at error: `quiet` is a single -q (one step \
                       quieter); `default` is this tool's own starting point on that scale; \
                       `v` / `vv` / `vvv` / `vvvv` are -v / -vv / -vvv / -vvvv (one to four \
                       steps louder)."
            .to_string(),
        repeatable: false,
        inherited: parameters[verbose_idx].inherited && parameters[quiet_idx].inherited,
    };

    let mut merged: Vec<Parameter> = parameters
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != verbose_idx && *i != quiet_idx)
        .map(|(_, p)| p.clone())
        .collect();
    merged.insert(insert_at, log_level);
    merged
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

/// The CLI flag name `[subcommand.<name>] hardcode_check = true` bakes in as
/// a literal, unconditional flag instead of a forwarded parameter (#350).
const HARDCODED_CHECK_PARAM: &str = "check";

fn is_hardcode_check(config: Option<&OrbConfig>, effective_name: &str) -> bool {
    config
        .and_then(|c| c.subcommand.as_ref())
        .and_then(|sc| sc.get(effective_name))
        .and_then(|sc_config| sc_config.hardcode_check)
        .unwrap_or(false)
}

/// Whether `p` is the `check` flag of the leaf rendered under
/// `effective_name`, with `hardcode_check` set for THAT leaf — the one
/// param that must never appear as a forwarded orb parameter (command or
/// job), only as a literal baked into the generated script
/// (gen-circleci-orb#350). Mirrors `check_ci_wiring`'s existing safety
/// property (`build_check_ci_wiring_step`, orb-producing jobs only) for any
/// subcommand with its own genuine `--check`-shaped flag.
///
/// Keyed by `effective_name`, not a bare subcommand name — two leaves
/// sharing a bare name (one qualified by `compute_effective_names`) must
/// each be addressable by their own config section, not have one leaf's
/// `[subcommand.<bare-name>] hardcode_check` bleed into the other's
/// rendering just because they share that bare name (#418/#425 class).
fn is_hardcoded_check_param(
    effective_name: &str,
    p: &Parameter,
    config: Option<&OrbConfig>,
) -> bool {
    p.long_name == HARDCODED_CHECK_PARAM && is_hardcode_check(config, effective_name)
}

/// Whether `p` is configured `[subcommand.<effective_name>.param.<flag>]
/// workspace_sourced = true` — opting it into a runtime-resolved fallback
/// (see `build_workspace_sourced_params`/`render_command_script_content`)
/// alongside its ordinary pipeline-compile-time literal. Config keys an
/// override by the CLI flag name (`p.long_name`), matching every other
/// per-param override lookup (`render_job`'s default-override loop).
fn is_workspace_sourced_param(
    effective_name: &str,
    p: &Parameter,
    config: Option<&OrbConfig>,
) -> bool {
    config
        .and_then(|c| c.subcommand.as_ref())
        .and_then(|sc| sc.get(effective_name))
        .and_then(|sc_config| sc_config.param.as_ref())
        .and_then(|params| params.get(&p.long_name))
        .and_then(|o| o.workspace_sourced)
        .unwrap_or(false)
}

/// Subcommands that are interactive (CLI-only) by default, unless the consumer
/// opts them back in with `[subcommand.<name>] interactive = false`. `help` is
/// not listed here — it is reserved earlier, at the `--help` parser.
pub(crate) const DEFAULT_INTERACTIVE: &[&str] = &["init", "config"];

/// Whether a subcommand is reserved as interactive/CLI-only: an explicit
/// `interactive` value wins, otherwise it falls back to [`DEFAULT_INTERACTIVE`].
/// Interactive commands are fully excluded from the orb (job + command + script,
/// and a parent's whole subtree).
pub(crate) fn is_interactive(config: Option<&OrbConfig>, name: &str) -> bool {
    config
        .and_then(|c| c.subcommand.as_ref())
        .and_then(|sc| sc.get(name))
        .and_then(|sc_config| sc_config.interactive)
        .unwrap_or_else(|| DEFAULT_INTERACTIVE.contains(&name))
}

/// The orb-resource name to use for every leaf subcommand `render_subcommand`
/// will actually render, keyed by that leaf's full dotted path (e.g.
/// `"ci.release"`). A bare name unique across the whole tree keeps its bare
/// name — zero behavior change for the overwhelming majority of consumers.
/// A bare name used at more than one path is qualified at EVERY occurrence
/// by its full underscore-joined path (e.g. `ci_release`) — a root-level
/// occurrence's own path already equals its bare name, so it's naturally
/// unaffected without any special-casing.
///
/// gen-circleci-orb#358: this REPLACES an earlier reject-based validator.
/// Rejecting an ambiguous CLI pushed the generator's own bare-name-only
/// addressing limitation onto the CLI author — painful or impossible for a
/// CLI they don't control. Qualifying instead means no user-visible
/// workaround is ever required; a non-colliding CLI, at any depth, is
/// completely unaffected.
///
/// Mirrors `render_subcommand`'s own traversal exactly: a non-leaf group
/// never writes its own file, so its name is never collected; a subtree
/// excluded via `is_interactive` writes nothing at all, so it's skipped
/// entirely, same as `render_subcommand` skips recursing into it.
pub(crate) fn compute_effective_names(
    cli: &CliDefinition,
    config: Option<&OrbConfig>,
) -> HashMap<String, String> {
    let mut by_bare_name: HashMap<&str, Vec<String>> = HashMap::new();
    collect_leaf_paths(&cli.subcommands, "", config, &mut by_bare_name);

    let mut effective = HashMap::new();
    for (bare_name, paths) in by_bare_name {
        if paths.len() == 1 {
            effective.insert(paths.into_iter().next().unwrap(), bare_name.to_string());
        } else {
            for path in paths {
                let qualified = path.replace('.', "_");
                effective.insert(path, qualified);
            }
        }
    }
    effective
}

/// Recursively collects every rendered leaf's dotted path, keyed by its bare
/// name, so `compute_effective_names` can tell which bare names are unique.
fn collect_leaf_paths<'a>(
    subs: &'a [SubCommand],
    prefix: &str,
    config: Option<&OrbConfig>,
    by_bare_name: &mut HashMap<&'a str, Vec<String>>,
) {
    for sub in subs {
        if is_interactive(config, &sub.name) {
            continue;
        }
        let path = if prefix.is_empty() {
            sub.name.clone()
        } else {
            format!("{prefix}.{}", sub.name)
        };
        if sub.is_leaf {
            by_bare_name
                .entry(&sub.name)
                .or_default()
                .push(path.clone());
        }
        collect_leaf_paths(&sub.subcommands, &path, config, by_bare_name);
    }
}

/// `path` is the full chain of subcommand names from the root down to (and
/// including) `sub` — e.g. `["ci", "release"]` for `ci release`. Needed so
/// the invocation script (`render_command_script_content`) can reconstruct
/// the real CLI command line for a nested subcommand, not just its own bare
/// leaf name (gen-circleci-orb#358 redesign prerequisite).
///
/// `effective_names` (from `compute_effective_names`) is looked up by the
/// dotted form of `path` to get the orb-resource name to render THIS leaf
/// under — its own bare name unless it collides with another leaf
/// elsewhere in the tree, in which case it's already been qualified by its
/// full path.
#[allow(clippy::too_many_arguments)]
/// Whether any leaf subcommand in `subs` (recursively) has a param configured
/// `workspace_sourced = true` — decides whether `resolve_workspace_param.sh`
/// needs to be generated at all. Mirrors `render_subcommand`'s own
/// interactive-skip + effective-name resolution so it agrees with what
/// actually gets rendered.
fn any_workspace_sourced_param(
    subs: &[SubCommand],
    effective_names: &HashMap<String, String>,
    path_prefix: &str,
    config: Option<&OrbConfig>,
) -> bool {
    subs.iter().any(|sub| {
        if is_interactive(config, &sub.name) {
            return false;
        }
        let dotted_path = if path_prefix.is_empty() {
            sub.name.clone()
        } else {
            format!("{path_prefix}.{}", sub.name)
        };
        if sub.is_leaf {
            let effective_name = effective_names
                .get(&dotted_path)
                .cloned()
                .unwrap_or_else(|| sub.name.clone());
            if sub
                .parameters
                .iter()
                .any(|p| is_workspace_sourced_param(&effective_name, p, config))
            {
                return true;
            }
        }
        any_workspace_sourced_param(&sub.subcommands, effective_names, &dotted_path, config)
    })
}

fn render_subcommand(
    sub: &SubCommand,
    path: &[String],
    binary: &str,
    opts: &GenerateOpts,
    config: Option<&OrbConfig>,
    effective_names: &HashMap<String, String>,
    files: &mut HashMap<PathBuf, String>,
) {
    // Interactive/CLI-only: emit nothing for this subcommand or its subtree.
    if is_interactive(config, &sub.name) {
        return;
    }
    if sub.is_leaf {
        let dotted_path = path.join(".");
        let effective_name = effective_names
            .get(&dotted_path)
            .cloned()
            .unwrap_or_else(|| sub.name.clone());
        let snake = effective_name.replace('-', "_");
        files.insert(
            PathBuf::from(format!("src/commands/{snake}.yml")),
            render_command(sub, &effective_name, config),
        );
        if !is_job_suppressed(config, &effective_name) {
            files.insert(
                PathBuf::from(format!("src/jobs/{snake}.yml")),
                render_job(sub, &effective_name, opts, config),
            );
        }
        files.insert(
            PathBuf::from(format!("src/scripts/{snake}.sh")),
            render_command_script_content(sub, &effective_name, path, binary, config),
        );
    }
    for child in &sub.subcommands {
        let mut child_path = path.to_vec();
        child_path.push(child.name.clone());
        render_subcommand(
            child,
            &child_path,
            binary,
            opts,
            config,
            effective_names,
            files,
        );
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
    prefix_if_reserved(RESTRICTED_COMMAND_PARAMS, subcommand, param)
}

/// `{scope}_{name}` when `name` is in `reserved`, else `name` unchanged. The
/// one rename rule behind every generated key that must avoid a
/// CircleCI-reserved name — command params (scope: the subcommand), job-group
/// inherited params (scope: the group). The scope is snake-cased: a
/// multi-word name (e.g. `add-job-group`) must not leak hyphens into an orb
/// param key or its derived env var — orb param keys must be snake_case
/// (RC010).
fn prefix_if_reserved(reserved: &[&str], scope: &str, name: &str) -> String {
    if reserved.contains(&name) {
        scoped(scope, name)
    } else {
        name.to_string()
    }
}

/// `{scope}_{name}`, with the scope snake-cased (see [`prefix_if_reserved`]).
fn scoped(scope: &str, name: &str) -> String {
    format!("{}_{name}", scope.replace('-', "_"))
}

/// Resolve the orb-facing parameter name for a CLI parameter, honoring an
/// explicit `[subcommand.<name>.param.<flag>] orb_name = "..."` override
/// before falling back to automatic restricted-name renaming
/// (`resolve_command_param_name`). The override is always available to a
/// consumer via `gen-circleci-orb.toml`, even when they don't control the
/// underlying CLI — it's the resolution path for a renamed-key collision
/// between a restricted param's automatic rename and an unrelated, genuinely
/// same-named flag (gen-circleci-orb#412; see #358 for why "force the
/// consumer to redesign a CLI they may not control" is the wrong shape for
/// this kind of fix).
///
/// `subcommand` (bare) and `effective_name` (bare, or collision-qualified —
/// `compute_effective_names`'s output) are deliberately separate: `subcommand`
/// only feeds the automatic restricted-rename prefix (`resolve_command_param_name`,
/// which stays tied to the CLI's own bare name — that's what a consumer sees
/// on the command line), while `effective_name` is the CONFIG SECTION lookup
/// key — the same qualified name a colliding leaf's own files are rendered
/// under (`is_job_suppressed`/`resolve_run_step_name`/#418's pattern). Looking
/// the config section up by the bare `subcommand` instead ambiguously applies
/// one `[subcommand.<name>]` override to every leaf sharing that bare name
/// (gen-circleci-orb#425).
pub(crate) fn resolve_param_orb_name(
    subcommand: &str,
    effective_name: &str,
    param: &str,
    config: Option<&OrbConfig>,
) -> String {
    if let Some(name) = config
        .and_then(|c| c.subcommand.as_ref())
        .and_then(|m| m.get(effective_name))
        .and_then(|sc| sc.param.as_ref())
        .and_then(|p| p.get(param))
        .and_then(|po| po.orb_name.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return name.to_string();
    }
    resolve_command_param_name(subcommand, param)
}

/// Env var name for a CLI parameter's value inside a generated script or
/// `environment:` block. Always `GCO_`-prefixed — never a bare uppercase of
/// the orb parameter name — so it can never collide with a shell-reserved or
/// tool-reserved variable (`PATH`, `HOME`, `IFS`, `GIT_*`, `SSH_*`, ...)
/// without needing to enumerate them (gen-circleci-orb#370). `GCO_` mirrors
/// `CIRCLE_*` being CircleCI's own meaningfully-named env-var namespace.
/// Applied unconditionally, to every param — not just ones that happen to
/// collide with something today.
fn env_var_name(orb_param_name: &str) -> String {
    format!("GCO_{}", orb_param_name.to_uppercase())
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

/// Job parameter keys `render_job` synthesizes itself — `attach_workspace`/
/// `workspace_root` unconditionally, the rest only for an orb-producing
/// subcommand (one with an `orb_dir` param) — and inserts AFTER
/// `build_orb_parameters`, unconditionally overwriting any CLI-derived entry
/// already at that key (gen-circleci-orb#412). A resolved param key (bare,
/// restricted-renamed, or `orb_name`-overridden) landing on one of these is
/// silent corruption, not a normal collision `orb_name` can be used to avoid
/// on its own — since these names aren't CLI-derived, a consumer has no way
/// to know to avoid them without this list. Checked as an unconditional
/// superset (not just the orb-producing subset) by
/// `validate_param_key_collisions`: whether a given subcommand actually
/// triggers the orb-producing branch depends on its own parsed params, which
/// the pre-render validator doesn't re-derive — failing loudly for the full
/// superset is the safe direction, matching this codebase's established
/// preference for erroring over silently picking a winner.
pub(crate) const SYNTHESIZED_JOB_PARAMS: &[&str] = &[
    "attach_workspace",
    "workspace_root",
    "persist_orb_workspace",
    "ssh_fingerprint",
    "check_ci_wiring",
    "target_branch",
];

/// Resolve the display name for a command's `run` step: a curated `label`
/// from config, else `short_about`, else the bare subcommand name.
fn resolve_run_step_name(sub: &SubCommand, config: Option<&OrbConfig>) -> String {
    if let Some(label) = config
        .and_then(|c| c.subcommand.as_ref())
        .and_then(|m| m.get(&sub.name))
        .and_then(|sc| sc.label.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return label.to_string();
    }
    let about = sub.short_about.trim();
    if !about.is_empty() {
        return about.to_string();
    }
    sub.name.clone()
}

fn render_command(sub: &SubCommand, effective_name: &str, config: Option<&OrbConfig>) -> String {
    let parameters = build_command_orb_parameters(sub, effective_name, config);
    // Boolean flags are set as string env vars via `when` steps (reliable),
    // ahead of the run step that consumes them via BASH_ENV.
    let mut steps = build_boolean_flag_env_steps(sub, effective_name, config);
    steps.push(build_run_step(
        sub,
        effective_name,
        &resolve_run_step_name(sub, config),
        config,
    ));
    let cmd = OrbCommand {
        description: sub.description.clone(),
        parameters,
        steps,
    };
    serde_yaml::to_string(&cmd).unwrap()
}

/// For each boolean parameter, emit a `when: condition: << parameters.X >>` step
/// that exports `X=true` to `BASH_ENV`. This gives the command's script a real
/// string env var to test (`[[ "${X:-false}" = "true" ]]`), instead of a YAML
/// boolean in the `environment:` block — which CircleCI does not reliably expose
/// to the shell as "true", so the flag would silently never be passed.
fn build_boolean_flag_env_steps(
    sub: &SubCommand,
    effective_name: &str,
    config: Option<&OrbConfig>,
) -> Vec<serde_yaml::Value> {
    let mut steps = Vec::new();
    for p in &sub.parameters {
        if !matches!(p.param_type, ParamType::Boolean) {
            continue;
        }
        if is_hardcoded_check_param(effective_name, p, config) {
            continue;
        }
        let orb_name = resolve_param_orb_name(&sub.name, effective_name, &p.long_name, config);
        let env_var = env_var_name(&orb_name);

        let mut run_map = serde_yaml::Mapping::new();
        run_map.insert(
            serde_yaml::Value::String("name".to_string()),
            serde_yaml::Value::String(format!("Set {env_var} flag")),
        );
        run_map.insert(
            serde_yaml::Value::String("command".to_string()),
            serde_yaml::Value::String(format!("echo 'export {env_var}=true' >> \"$BASH_ENV\"")),
        );
        let mut run_step = serde_yaml::Mapping::new();
        run_step.insert(
            serde_yaml::Value::String("run".to_string()),
            serde_yaml::Value::Mapping(run_map),
        );
        let inner_steps = serde_yaml::Value::Sequence(vec![serde_yaml::Value::Mapping(run_step)]);

        let mut when_inner = serde_yaml::Mapping::new();
        when_inner.insert(
            serde_yaml::Value::String("condition".to_string()),
            serde_yaml::Value::String(format!("<< parameters.{orb_name} >>")),
        );
        when_inner.insert(serde_yaml::Value::String("steps".to_string()), inner_steps);
        let mut when_map = serde_yaml::Mapping::new();
        when_map.insert(
            serde_yaml::Value::String("when".to_string()),
            serde_yaml::Value::Mapping(when_inner),
        );
        steps.push(serde_yaml::Value::Mapping(when_map));
    }
    steps
}

/// Build orb parameters for a command, renaming any restricted names with a
/// subcommand prefix so they remain usable (e.g. `name` → `generate_name`).
/// Compute the orb parameter default for a CLI parameter.
///
/// Enum params must default to one of their values — `circleci orb validate`
/// rejects an empty default for an enum — so an enum with no CLI default falls
/// back to its first value. Other non-required params with no default get an
/// empty string (the CircleCI convention for "optional/unset").
fn orb_param_default(p: &crate::help_parser::types::Parameter) -> Option<serde_yaml::Value> {
    match &p.param_type {
        ParamType::Boolean => {
            let val = p.default.as_ref().map(|d| d == "true").unwrap_or(false);
            Some(serde_yaml::Value::Bool(val))
        }
        ParamType::Enum(vals) => {
            Some(serde_yaml::Value::String(p.default.clone().unwrap_or_else(
                || vals.first().cloned().unwrap_or_default(),
            )))
        }
        ParamType::Integer if p.default.is_some() => Some(coerce_override_default(
            p.default.as_deref().expect("checked by guard"),
            "integer",
        )),
        _ if !p.required && p.default.is_none() => Some(serde_yaml::Value::String(String::new())),
        _ => p
            .default
            .as_ref()
            .map(|d| serde_yaml::Value::String(d.clone())),
    }
}

/// Coerce a raw default string into the YAML value shape its declared
/// `param_type` (`"boolean"`/`"integer"`/…) requires. Shared by every site
/// that builds an `OrbParameter` from a string-typed source — a
/// `[subcommand.<name>.param.<param>] default = "..."` override
/// (`render_job`) and a declared rich `job_group` parameter
/// (`render_rich_job_group`) — plus `orb_param_default`'s own
/// `ParamType::Integer` arm, for the one caller that already has a typed
/// `ParamType` instead of a bare string. Without this, a boolean/integer
/// param renders as a quoted YAML string (`default: "true"`), which
/// CircleCI's orb schema rejects as a type mismatch against `type:
/// boolean`/`type: integer` (#347).
fn coerce_override_default(raw: &str, param_type: &str) -> serde_yaml::Value {
    match param_type {
        "boolean" => serde_yaml::Value::Bool(raw == "true"),
        "integer" => raw
            .parse::<i64>()
            .map(|n| serde_yaml::Value::Number(n.into()))
            .unwrap_or_else(|_| serde_yaml::Value::String(raw.to_string())),
        _ => serde_yaml::Value::String(raw.to_string()),
    }
}

fn build_command_orb_parameters(
    sub: &SubCommand,
    effective_name: &str,
    config: Option<&OrbConfig>,
) -> IndexMap<String, OrbParameter> {
    let mut params = IndexMap::new();
    for p in &sub.parameters {
        if is_hardcoded_check_param(effective_name, p, config) {
            continue;
        }
        let orb_name = resolve_param_orb_name(&sub.name, effective_name, &p.long_name, config);
        let (type_str, enum_vals) = match &p.param_type {
            ParamType::String => ("string".to_string(), None),
            ParamType::Boolean => ("boolean".to_string(), None),
            ParamType::Integer => ("integer".to_string(), None),
            ParamType::Enum(vals) => ("enum".to_string(), Some(vals.clone())),
        };
        let default = orb_param_default(p);
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
///
/// Options are appended first and positionals last, in declaration order: a
/// positional emitted between an option and its value would be read as that
/// value.
fn render_command_script_content(
    sub: &SubCommand,
    effective_name: &str,
    path: &[String],
    binary: &str,
    config: Option<&OrbConfig>,
) -> String {
    // The real CLI invocation needs every ancestor segment, not just this
    // subcommand's own bare name — `ci release` must run `binary ci
    // release`, not `binary release` (gen-circleci-orb#358 redesign
    // prerequisite: previously only `sub.name` was used here, silently
    // breaking any subcommand nested more than one level deep).
    let full_command: Vec<String> = path.iter().map(|seg| seg.replace('_', "-")).collect();
    let mut lines: Vec<String> = vec![format!("set -- {} {}", binary, full_command.join(" "))];

    let (positionals, options): (Vec<&Parameter>, Vec<&Parameter>) = sub
        .parameters
        .iter()
        .partition(|p| p.kind == ParamKind::Positional);

    for p in options.into_iter().chain(positionals) {
        if is_hardcoded_check_param(effective_name, p, config) {
            // Baked in as a literal, unconditional flag -- never read from a
            // consumer-settable env var (gen-circleci-orb#350).
            lines.push(format!(
                r#"set -- "$@" --{}"#,
                p.long_name.replace('_', "-")
            ));
            continue;
        }
        let orb_name = resolve_param_orb_name(&sub.name, effective_name, &p.long_name, config);
        let env_var = env_var_name(&orb_name);
        // A positional is passed bare; a short-only option by its short flag,
        // which is the only form the CLI accepts.
        let flag = match p.kind {
            ParamKind::Positional => String::new(),
            ParamKind::ShortOnly => p.short.map(|c| format!("-{c} ")).unwrap_or_default(),
            ParamKind::Long => format!("--{} ", p.long_name.replace('_', "-")),
        };
        let line = if is_workspace_sourced_param(effective_name, p, config) {
            // Prefer the literal (unchanged precedence for every existing
            // consumer) -> fall back to the workspace-resolved value (set by
            // build_resolve_workspace_param_step's script into a distinctly
            // named var, so there's no environment:-vs-$BASH_ENV precedence
            // to reason about) -> for a REQUIRED param, a loud error naming
            // both if neither is set; for an OPTIONAL one, silently omit the
            // flag (matching that param's own pre-existing, still-valid
            // "unset is fine" behavior — workspace_sourced adds a second way
            // to supply the value, it must not narrow an optional param into
            // a mandatory one).
            let resolved_var = env_var_name(&format!("{orb_name}_resolved"));
            let value_var = format!("{env_var}_VALUE");
            // A bare `-z` only rejects a genuinely zero-length string -- a
            // whitespace-only value (an upstream template accidentally
            // passing `version: " "`) would otherwise slip through as
            // "present" at every check below, right up to being forwarded
            // to the CLI as a literal argument. `[[:space:]]*$` treats
            // whitespace-only the same as empty everywhere a value is
            // tested, including the literal-vs-resolved fallback decision.
            let is_blank = |var: &str| format!("[[ \"${{{var}}}\" =~ ^[[:space:]]*$ ]]");
            let resolve_lines = format!(
                "{value_var}=\"${{{env_var}:-}}\"\n\
                 if {blank}; then\n  \
                 {value_var}=\"${{{resolved_var}:-}}\"\n\
                 fi",
                blank = is_blank(&value_var)
            );
            if p.required {
                format!(
                    "{resolve_lines}\n\
                     if {blank}; then\n  \
                     echo \"ERROR: no value for {orb_name} -- set the '{orb_name}' \
                     parameter, or '{orb_name}_env_var' (with attach_workspace) to \
                     resolve one at runtime.\" >&2\n  \
                     exit 1\n\
                     fi\n\
                     set -- \"$@\" {flag}\"${{{value_var}}}\"",
                    blank = is_blank(&value_var)
                )
            } else {
                format!(
                    "{resolve_lines}\n\
                     {blank_negated} && set -- \"$@\" {flag}\"${{{value_var}}}\"",
                    blank_negated = is_blank(&value_var).replacen("[[ ", "[[ ! ", 1)
                )
            }
        } else if p.long_name == LOG_LEVEL_PARAM {
            render_log_level_case(&env_var)
        } else {
            match &p.param_type {
                ParamType::Boolean => {
                    let flag = flag.trim_end();
                    format!(r#"[[ "${{{env_var}:-false}}" = "true" ]] && set -- "$@" {flag}"#)
                }
                _ => {
                    if p.required {
                        format!(r#"set -- "$@" {flag}"${{{env_var}}}""#)
                    } else {
                        format!(
                            r#"[[ -n "${{{env_var}:-}}" ]] && set -- "$@" {flag}"${{{env_var}}}""#
                        )
                    }
                }
            }
        };
        lines.push(line);
    }

    lines.push(r#""$@""#.to_string());
    lines.join("\n") + "\n"
}

/// Translates the merged `log_level` enum (see `merge_verbosity_pair`) into
/// the repeated `--verbose`/`--quiet` occurrences the underlying CLI
/// actually expects. `default` has no arm — it falls through and adds
/// nothing, matching the tool's own baseline.
fn render_log_level_case(env_var: &str) -> String {
    let mut out = format!("case \"${{{env_var}:-default}}\" in\n");
    out.push_str("  quiet) set -- \"$@\" --quiet ;;\n");
    out.push_str("  v) set -- \"$@\" --verbose ;;\n");
    out.push_str("  vv) set -- \"$@\" --verbose --verbose ;;\n");
    out.push_str("  vvv) set -- \"$@\" --verbose --verbose --verbose ;;\n");
    out.push_str("  vvvv) set -- \"$@\" --verbose --verbose --verbose --verbose ;;\n");
    out.push_str("esac");
    out
}

fn render_job(
    sub: &SubCommand,
    effective_name: &str,
    opts: &GenerateOpts,
    config: Option<&OrbConfig>,
) -> String {
    let mut parameters = build_orb_parameters(sub, effective_name, RESERVED_JOB_PARAMS, config);

    // Apply param default overrides from config. Keyed by `effective_name`,
    // not `sub.name` — for a colliding leaf, that's the SAME qualified name
    // its own files are rendered under (`is_job_suppressed`/
    // `resolve_run_step_name`'s pattern, #416); looking this up by bare
    // `sub.name` would apply one config section's override to every
    // occurrence sharing that bare name (gen-circleci-orb#418).
    if let Some(param_overrides) = config
        .and_then(|c| c.subcommand.as_ref())
        .and_then(|sc| sc.get(effective_name))
        .and_then(|sc_config| sc_config.param.as_ref())
    {
        for (param_name, override_) in param_overrides {
            // Config keys an override by its CLI flag name ("name"), but
            // build_orb_parameters stores the param under its RESOLVED key —
            // the same key resolve_param_orb_name computes (an explicit
            // orb_name override, or the automatic restricted rename, e.g.
            // "generate_name") — so the default lookup must resolve the same
            // way, or it silently no-ops (#369 follow-up, generalized by
            // #412's orb_name override).
            let resolved_key =
                resolve_param_orb_name(&sub.name, effective_name, param_name, config);
            if let Some(param) = parameters.get_mut(&resolved_key) {
                if let Some(new_default) = &override_.default {
                    param.default = Some(coerce_override_default(new_default, &param.param_type));
                }
            }
        }
    }
    let (attach_param, root_param) = build_workspace_params();
    parameters.insert("attach_workspace".to_string(), attach_param);
    parameters.insert("workspace_root".to_string(), root_param);

    // Params opted into runtime workspace-sourced resolution
    // (`[subcommand.<name>.param.<flag>] workspace_sourced = true`) — see
    // `is_workspace_sourced_param`. Computed from the subcommand's OWN
    // params (unlike the orb-producing block below), since this targets one
    // specific existing param, not a fixed job-wide concern.
    let workspace_sourced_keys: Vec<String> = sub
        .parameters
        .iter()
        .filter(|p| is_workspace_sourced_param(effective_name, p, config))
        .map(|p| resolve_param_orb_name(&sub.name, effective_name, &p.long_name, config))
        .collect();
    for key in &workspace_sourced_keys {
        let (env_var_param, source_file_param) = build_workspace_sourced_params(key);
        parameters.insert(format!("{key}_env_var"), env_var_param);
        parameters.insert(format!("{key}_source_file"), source_file_param);
        // A required CLI param (e.g. release-prep's positional `version`)
        // otherwise gets no `default:` at all (`orb_param_default` only
        // omits it for a required param), which CircleCI then also requires
        // at job-invocation time -- defeating the whole point of the
        // fallback below. workspace_sourced means "the literal is now
        // optional, resolution can supply it instead", so its own job
        // parameter must gain an empty default regardless of the CLI's own
        // required-ness.
        if let Some(param) = parameters.get_mut(key) {
            if param.default.is_none() {
                param.default = Some(serde_yaml::Value::String(String::new()));
            }
        }
    }

    // Orb-producing jobs (those with an `orb_dir` param) can persist the
    // regenerated orb to the workspace so it flows to downstream pack/review/push
    // jobs without an immediate push (Model B). Default off; activated by the
    // consumer workflow on the regenerate-orb step.
    let is_orb_producing = parameters.contains_key("orb_dir");
    if is_orb_producing {
        parameters.insert(
            "persist_orb_workspace".to_string(),
            build_persist_orb_param(),
        );
        parameters.insert("ssh_fingerprint".to_string(), build_ssh_fingerprint_param());
        parameters.insert("check_ci_wiring".to_string(), build_check_ci_wiring_param());
        parameters.insert("target_branch".to_string(), build_target_branch_param());
    }

    let invoke_step = build_invoke_step(sub, effective_name, RESERVED_JOB_PARAMS, config);
    let mut steps = vec![serde_yaml::Value::String("checkout".to_string())];
    if is_orb_producing {
        steps.push(build_target_branch_switch_step());
    }
    steps.push(build_attach_workspace_step());
    for key in &workspace_sourced_keys {
        steps.push(build_resolve_workspace_param_step(key));
    }
    if opts.git_push_subcommands.contains(&sub.name) {
        steps.push(serde_yaml::Value::String("set_https_remote".to_string()));
    }
    // Load the configured SSH write key (and drop the read-only checkout key)
    // before invoking, so the end-of-workflow push authenticates with write
    // authority. No-op when ssh_fingerprint is empty (ambient credentials used).
    if is_orb_producing {
        steps.push(build_ssh_setup_step());
    }
    steps.push(invoke_step);
    if is_orb_producing {
        steps.push(build_persist_orb_step());
        steps.push(build_check_ci_wiring_step());
    }
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
    s.push_str("FROM debian:13-slim AS cli-installer\n");
    s.push_str(&format!("ARG CIRCLECI_CLI_VERSION={ver}\n"));
    s.push_str("RUN apt-get update \\\n");
    s.push_str("    && apt-get install -y --no-install-recommends ca-certificates curl \\\n");
    s.push_str("    && rm -rf /var/lib/apt/lists/* \\\n");
    s.push_str("    && cd /tmp \\\n");
    s.push_str("    && TARBALL=\"circleci-cli_${CIRCLECI_CLI_VERSION}_linux_amd64.tar.gz\" \\\n");
    // The release URL exceeds the line limit on its own, so it is assembled from
    // variables and each download splits over continuations (docker:S7020).
    s.push_str("    && BASE=\"https://github.com/CircleCI-Public/circleci-cli/releases\" \\\n");
    s.push_str("    && REL=\"${BASE}/download/v${CIRCLECI_CLI_VERSION}\" \\\n");
    s.push_str("    && curl -fLSs --proto '=https' \\\n");
    s.push_str("         \"${REL}/${TARBALL}\" -o \"${TARBALL}\" \\\n");
    s.push_str("    && curl -fLSs --proto '=https' \\\n");
    s.push_str("         \"${REL}/circleci-cli_${CIRCLECI_CLI_VERSION}_checksums.txt\" \\\n");
    s.push_str("         -o checksums.txt \\\n");
    s.push_str("    && grep \"${TARBALL}\" checksums.txt | sha256sum --check \\\n");
    s.push_str("    && tar -xzf \"${TARBALL}\" circleci \\\n");
    s.push_str("    && install -m 755 circleci /usr/local/bin/circleci \\\n");
    s.push_str("    && rm -rf \"${TARBALL}\" checksums.txt circleci\n");
    s
}

fn sorted_packages(extra: &[String]) -> Vec<&str> {
    let mut pkgs: Vec<&str> = vec!["ca-certificates", "git"];
    pkgs.extend(extra.iter().map(String::as_str));
    pkgs.sort_unstable();
    pkgs.dedup();
    pkgs
}

/// Emit an `apt-get install` fragment with one package per `\`-continued line,
/// per docker:S7020 (a single long package line trips the length limit). The
/// result slots into a `RUN apt-get update \` chain: it opens with
/// `    && apt-get install …` and every line — including the last package —
/// ends with a `\` continuation, so the caller appends `    && rm -rf …` next.
fn render_apt_install(pkgs: &[&str]) -> String {
    let mut s = String::from("    && apt-get install -y --no-install-recommends \\\n");
    for pkg in pkgs {
        s.push_str(&format!("    {pkg} \\\n"));
    }
    s
}

fn render_dockerfile(binary: &str, opts: &GenerateOpts) -> String {
    match opts.install_method {
        InstallMethod::Binstall => render_binstall_dockerfile(binary, opts),
        InstallMethod::Local => render_local_dockerfile(binary, opts),
        InstallMethod::Apt => render_apt_dockerfile(binary, opts),
    }
}

/// A self-sufficient build toolchain: clang (libclang) and cmake cover native
/// crates using bindgen or cmake, so the builder does not depend on the base
/// image's age.
const BUILDER_PACKAGES: &[&str] = &[
    "build-essential",
    "ca-certificates",
    "clang",
    "cmake",
    "libssl-dev",
    "pkg-config",
];

/// The binary name the CLI-installer stage copies in. Single source of truth
/// so `commands::generate::validate_cargo_tool_entries` can reserve it against
/// `cargo_tools` collisions rather than duplicating the literal.
pub(crate) const CIRCLECI_CLI_BINARY: &str = "circleci";

/// Build from crates.io in a builder stage, then copy the binary into a slim
/// runtime.
fn render_binstall_dockerfile(binary: &str, opts: &GenerateOpts) -> String {
    // One read, because the installer stage below and the COPY that pulls from
    // it must never disagree — a COPY from a stage that was not emitted fails
    // the container build in the release pipeline.
    let cli = opts.circleci_cli_version.as_deref();

    // Plain sort — not `sorted_packages`, which prepends the apt baseline and
    // would put `ca-certificates` and `git` in the COPY list below.
    let mut tools = opts.cargo_tools.to_vec();
    tools.sort();

    let mut out = format!("FROM {} AS builder\n", opts.builder_image);
    // CRATE_VERSION pins the exact released version passed by build-container.sh
    // (from CIRCLE_TAG), and must precede the RUN that consumes it.
    out.push_str("ARG CRATE_VERSION\n");
    out.push_str("RUN apt-get update \\\n");
    out.push_str(&render_apt_install(BUILDER_PACKAGES));
    out.push_str("    && rm -rf /var/lib/apt/lists/* \\\n");
    out.push_str(&render_propagation_gate(binary, &opts.crate_wait));
    out.push_str(&render_cargo_tools_install(&tools));

    if let Some(ver) = cli {
        out.push('\n');
        out.push_str(&render_cli_installer_stage(ver));
    }
    out.push('\n');

    // The runtime only cares about each entry's binary half — that's the name
    // `cargo binstall` actually wrote under /usr/local/cargo/bin/ — and lands
    // it on PATH as a standalone binary, so the runtime needs no cargo or Rust
    // toolchain to run it.
    let mut copies = vec![format!(
        "COPY --from=builder /usr/local/cargo/bin/{binary} /usr/local/bin/{binary}\n"
    )];
    copies.extend(tools.iter().map(|(_, tool_binary, _)| {
        format!(
            "COPY --from=builder /usr/local/cargo/bin/{tool_binary} /usr/local/bin/{tool_binary}\n"
        )
    }));
    out.push_str(&render_runtime_stage(
        &opts.base_image,
        &opts.apt_packages,
        &copies,
        cli.is_some(),
    ));
    out
}

/// `cargo install`, wrapped in a bounded retry that waits out crates.io sparse
/// index lag.
///
/// The retry *is* the propagation gate — cargo-only, because the builder image
/// has no curl or wget. An unpinned or ungated install resolves the previous
/// version while the index lags the publish API, shipping a container whose
/// binary version does not match its own tag (#200). The window is deliberately
/// generous: a gate that expires leaves the release half-published, with the
/// crate up and no container, recoverable only by re-running the tag's workflow
/// by hand (#236).
///
/// `--locked` installs the dependency set the crate was published with, from
/// its bundled `Cargo.lock`, rather than resolving afresh — so the container
/// cannot diverge from what CI tested.
///
/// Split over `\` continuations so every line stays inside the Dockerfile line
/// limit (docker:S7020), even for a binary name far longer than this crate's.
fn render_propagation_gate(binary: &str, crate_wait: &CrateWait) -> String {
    let CrateWait { attempts, seconds } = crate_wait;
    let mut out = String::new();
    out.push_str("    && { err=/tmp/cargo-install.err; n=0; \\\n");
    out.push_str(&format!(
        "       until cargo install {binary} --locked \\\n"
    ));
    out.push_str("             --version \"${CRATE_VERSION}\" 2>\"$err\"; do \\\n");
    out.push_str("         cat \"$err\" >&2; \\\n");
    // A build failure is deterministic: retrying recompiles the whole crate and
    // buries the compiler error N repetitions deep. Only an index miss is worth
    // waiting out.
    out.push_str("         grep -q \"failed to compile\" \"$err\" \\\n");
    out.push_str("           && echo \"build failed, not an index delay\" >&2 && exit 1; \\\n");
    out.push_str("         n=$((n+1)); \\\n");
    out.push_str(&format!("         [ \"$n\" -ge {attempts} ] \\\n"));
    out.push_str("           && echo \"crates.io index never served ${CRATE_VERSION}\" >&2 \\\n");
    out.push_str("           && exit 1; \\\n");
    out.push_str("         echo \"waiting for crates.io index: ${CRATE_VERSION} (try $n)\"; \\\n");
    // Drop cargo's local index cache so the next attempt makes a full request.
    // Cheap insurance on the one layer we control; it cannot touch CDN edge
    // staleness.
    out.push_str(
        "         rm -rf \"${CARGO_HOME:-/usr/local/cargo}\"/registry/index/*/.cache; \\\n",
    );
    out.push_str(&format!("         sleep {seconds}; \\\n"));
    out.push_str("       done; }\n");
    out
}

/// Splits a `cargo_tools` entry into `(crate_name, binary_name, version)`.
///
/// A bare entry (`"cargo-audit"`) uses itself for both; `"crate:binary"`
/// (e.g. `"rsign2:rsign"`) opts in when the binary name differs. Each half
/// must follow Cargo's package name rules, since the crate half is passed
/// unquoted to `cargo binstall` and the binary half becomes a Dockerfile
/// `COPY` path.
///
/// An optional `@version` suffix (e.g. `"rsign2:rsign@2.1.0"`) pins the crate
/// to an exact version: `cargo binstall` accepts `crate@version` inline as a
/// single argument, so the version rides along with the crate half rather
/// than needing its own `--version` flag per tool. Bare entries keep floating
/// to whatever's latest on crates.io at container-build time — pinning is
/// opt-in per entry. The version is deliberately restricted to an exact pin
/// (leading digit, then alphanumeric/`.`/`-`/`+`) rather than the full range
/// of comparators `cargo binstall` itself accepts (e.g. `<=1.3.3`) — a range
/// doesn't pin anything, which defeats the point of this syntax.
pub(crate) fn split_cargo_tool_entry(entry: &str) -> anyhow::Result<(&str, &str, Option<&str>)> {
    // Cargo package name rules: leading letter or `_`, then alphanumeric/-/_.
    fn is_valid_segment(s: &str) -> bool {
        let mut chars = s.chars();
        matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    }

    // Leading digit, then alphanumeric/./-/+ — a SemVer-shaped exact version,
    // never a comparator (`<=`, `^`, `*`, ...) and never shell metacharacters.
    fn is_valid_version(s: &str) -> bool {
        let mut chars = s.chars();
        matches!(chars.next(), Some(c) if c.is_ascii_digit())
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '+')
    }

    let invalid = || {
        anyhow::anyhow!(
            "invalid cargo_tools entry {entry:?}: expected \"crate\", \"crate:binary\", \
             \"crate@version\", or \"crate:binary@version\" — crate/binary use Cargo package \
             name characters (leading letter or '_', then alphanumeric, '-', or '_'), version \
             starts with a digit (then alphanumeric, '.', '-', or '+')"
        )
    };

    // The crate/binary halves never contain '@' (is_valid_segment forbids it),
    // so a single '@' unambiguously starts the version suffix.
    let (spec, version) = match entry.matches('@').count() {
        0 => (entry, None),
        1 => {
            let (spec, version) = entry.split_once('@').expect("counted exactly one '@'");
            if !is_valid_version(version) {
                return Err(invalid());
            }
            (spec, Some(version))
        }
        _ => return Err(invalid()),
    };

    match spec.split_once(':') {
        None if is_valid_segment(spec) => Ok((spec, spec, version)),
        Some((krate, binary)) if is_valid_segment(krate) && is_valid_segment(binary) => {
            Ok((krate, binary, version))
        }
        _ => Err(invalid()),
    }
}

/// Extra tools the executor orchestrates, installed into the builder.
///
/// `cargo-binstall` fetches prebuilt binaries rather than compiling, and is
/// itself installed from crates.io rather than a `curl | bash` installer — the
/// supply chain stays on the same registry as every other dependency. One tool
/// per line, because the list grows with `[orb] cargo_tools` and would run past
/// the line limit.
///
/// `cargo binstall` takes crate specs — only the crate and version halves of
/// each triple are used here (as `crate` or `crate@version`), the binary half
/// only matters to the COPY that follows, in `render_binstall_dockerfile`.
fn render_cargo_tools_install(tools: &[(String, String, Option<String>)]) -> String {
    if tools.is_empty() {
        return String::new();
    }
    let mut out = String::from("RUN cargo install cargo-binstall --locked \\\n");
    out.push_str("    && cargo binstall --no-confirm \\\n");
    let last = tools.len() - 1;
    for (i, (krate, _, version)) in tools.iter().enumerate() {
        let cont = if i == last { "" } else { " \\" };
        match version {
            Some(v) => out.push_str(&format!("    {krate}@{v}{cont}\n")),
            None => out.push_str(&format!("    {krate}{cont}\n")),
        }
    }
    out
}

/// The stage binstall and local both end with: a slim image carrying the
/// binary, owned by an unprivileged user.
///
/// `with_cli` copies the circleci CLI in from the installer stage — both
/// callers gate that on the same condition.
fn render_runtime_stage(
    base_image: &str,
    apt_packages: &[String],
    copies: &[String],
    with_cli: bool,
) -> String {
    let mut out = format!("FROM {base_image}\n");
    out.push_str("RUN apt-get update \\\n");
    out.push_str(&render_apt_install(&sorted_packages(apt_packages)));
    out.push_str("    && rm -rf /var/lib/apt/lists/* \\\n");
    out.push_str("    && useradd -ms /bin/bash circleci\n");
    for line in copies {
        out.push_str(line);
    }
    if with_cli {
        out.push_str(&format!(
            "COPY --from=cli-installer /usr/local/bin/{CIRCLECI_CLI_BINARY} \
             /usr/local/bin/{CIRCLECI_CLI_BINARY}\n"
        ));
    }
    out.push_str("USER circleci\n");
    out.push_str("WORKDIR /home/circleci/project\n");
    out
}

/// The binary is already built and sits in the Docker build context.
fn render_local_dockerfile(binary: &str, opts: &GenerateOpts) -> String {
    // One read, as in the binstall path: the installer stage and the COPY that
    // pulls from it must not disagree.
    let cli = opts.circleci_cli_version.as_deref();

    let mut out = String::new();
    if let Some(ver) = cli {
        out.push_str(&render_cli_installer_stage(ver));
        out.push('\n');
    }
    let copies = vec![format!("COPY {binary} /usr/local/bin/{binary}\n")];
    out.push_str(&render_runtime_stage(
        &opts.base_image,
        &opts.apt_packages,
        &copies,
        cli.is_some(),
    ));
    out
}

/// The binary is packaged for apt, so one stage installs everything.
fn render_apt_dockerfile(binary: &str, opts: &GenerateOpts) -> String {
    let mut all_pkgs: Vec<&str> = vec!["git", binary];
    all_pkgs.extend(opts.apt_packages.iter().map(String::as_str));
    all_pkgs.sort_unstable();
    all_pkgs.dedup();

    let mut out = format!("FROM {}\n", opts.base_image);
    out.push_str("RUN apt-get update \\\n");
    out.push_str(&render_apt_install(&all_pkgs));
    out.push_str("    && rm -rf /var/lib/apt/lists/*\n");
    out
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

/// The job-level key to declare/forward parameter `p` under, or `None` if it
/// should be dropped entirely. A param in `skip` is dropped unless it's ALSO
/// RESTRICTED_COMMAND_PARAMS-renameable, in which case it's kept under the
/// SAME resolved key the invoked command already uses (#369) rather than the
/// bare skip-list name. Every kept param — restricted or not — resolves via
/// `resolve_param_orb_name`, so an `orb_name` override on an ordinary param
/// also stays in sync between the job's declared parameter and the command
/// it invokes (#412) — not just the restricted-rename case. Shared by
/// `build_orb_parameters` and `build_invoke_step` so the two can't drift
/// apart (a fix landing in only one would reintroduce #369 in the other).
fn resolve_job_param_key(
    sub: &SubCommand,
    effective_name: &str,
    p: &Parameter,
    skip: &[&str],
    config: Option<&OrbConfig>,
) -> Option<String> {
    if is_hardcoded_check_param(effective_name, p, config) {
        return None;
    }
    if skip.contains(&p.long_name.as_str()) {
        if RESTRICTED_COMMAND_PARAMS.contains(&p.long_name.as_str()) {
            Some(resolve_param_orb_name(
                &sub.name,
                effective_name,
                &p.long_name,
                config,
            ))
        } else {
            None
        }
    } else {
        Some(resolve_param_orb_name(
            &sub.name,
            effective_name,
            &p.long_name,
            config,
        ))
    }
}

fn build_orb_parameters(
    sub: &SubCommand,
    effective_name: &str,
    skip: &[&str],
    config: Option<&OrbConfig>,
) -> IndexMap<String, OrbParameter> {
    let mut params = IndexMap::new();
    for p in &sub.parameters {
        let Some(key) = resolve_job_param_key(sub, effective_name, p, skip, config) else {
            continue;
        };
        let (type_str, enum_vals) = match &p.param_type {
            ParamType::String => ("string".to_string(), None),
            ParamType::Boolean => ("boolean".to_string(), None),
            ParamType::Integer => ("integer".to_string(), None),
            ParamType::Enum(vals) => ("enum".to_string(), Some(vals.clone())),
        };
        let default = orb_param_default(p);
        params.insert(
            key,
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
fn build_run_step(
    sub: &SubCommand,
    effective_name: &str,
    run_name: &str,
    config: Option<&OrbConfig>,
) -> serde_yaml::Value {
    serde_yaml::Value::Mapping({
        let mut m = serde_yaml::Mapping::new();
        let mut run_map = serde_yaml::Mapping::new();
        run_map.insert(
            serde_yaml::Value::String("name".to_string()),
            serde_yaml::Value::String(run_name.to_string()),
        );
        run_map.insert(
            serde_yaml::Value::String("command".to_string()),
            serde_yaml::Value::String(format!(
                "<<include(scripts/{}.sh)>>",
                effective_name.replace('-', "_")
            )),
        );
        // Boolean params are NOT put in the environment block: a boolean
        // parameter interpolates to a YAML boolean (`true`), which CircleCI does
        // not reliably expose as the string "true" to the shell — so the script's
        // `[[ "${X}" = "true" ]]` check would fail and the flag would never be
        // added. They're set as real string env vars via `when` steps instead
        // (see build_boolean_flag_env_steps).
        let mut env_map = serde_yaml::Mapping::new();
        for p in &sub.parameters {
            if matches!(p.param_type, ParamType::Boolean) {
                continue;
            }
            let orb_name = resolve_param_orb_name(&sub.name, effective_name, &p.long_name, config);
            let env_var = env_var_name(&orb_name);
            env_map.insert(
                serde_yaml::Value::String(env_var),
                serde_yaml::Value::String(format!("<< parameters.{orb_name} >>")),
            );
        }
        if !env_map.is_empty() {
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

/// Build the command invocation step for a job. `effective_name` must match
/// the key the invoked command is actually rendered under
/// (`render_command`/`compute_effective_names`) — not necessarily
/// `sub.name`, when this subcommand's bare name collides elsewhere.
fn build_invoke_step(
    sub: &SubCommand,
    effective_name: &str,
    skip: &[&str],
    config: Option<&OrbConfig>,
) -> serde_yaml::Value {
    let mut invoke_map = serde_yaml::Mapping::new();
    for p in &sub.parameters {
        let Some(key) = resolve_job_param_key(sub, effective_name, p, skip, config) else {
            continue;
        };
        let value = format!("<< parameters.{key} >>");
        invoke_map.insert(
            serde_yaml::Value::String(key),
            serde_yaml::Value::String(value),
        );
    }
    serde_yaml::Value::Mapping({
        let mut m = serde_yaml::Mapping::new();
        m.insert(
            serde_yaml::Value::String(effective_name.replace('-', "_")),
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

/// The two job parameters a `workspace_sourced` param (`resolved_key`, its
/// already-resolved orb-facing name, e.g. `version`) gains: which variable to
/// extract, and which file to extract it from. Both empty by default — the
/// feature only activates when a consumer sets `<resolved_key>_env_var`,
/// keeping every existing literal-only consumer's job byte-for-byte
/// unchanged.
fn build_workspace_sourced_params(resolved_key: &str) -> (OrbParameter, OrbParameter) {
    let env_var = OrbParameter {
        param_type: "string".to_string(),
        description: format!(
            "Name of the variable inside <workspace_root>/versions.env-shaped source \
             file holding {resolved_key}'s real value (e.g. \"CRATE_VERSION_MYAPP\", as \
             written by circleci-toolkit's calculate_versions). Leave empty (default) to \
             use the {resolved_key} parameter's own literal value instead. Requires \
             attach_workspace."
        ),
        default: Some(serde_yaml::Value::String(String::new())),
        enum_values: None,
    };
    let source_file = OrbParameter {
        param_type: "string".to_string(),
        description: format!(
            "Path to the env-style file to resolve {resolved_key}_env_var from. Empty \
             (default) resolves to '<workspace_root>/versions.env' — calculate_versions' \
             own output path. Only used when {resolved_key}_env_var is set."
        ),
        default: Some(serde_yaml::Value::String(String::new())),
        enum_values: None,
    };
    (env_var, source_file)
}

/// Conditional step: when `<resolved_key>_env_var` is non-empty, source the
/// (attached-workspace) file it names, extract that one variable, and export
/// it to `$BASH_ENV` as `GCO_<RESOLVED_KEY>_RESOLVED` — a name distinct from
/// the literal `GCO_<RESOLVED_KEY>` so the command script's own fallback
/// logic (`render_command_script_content`) never depends on any
/// `environment:`-vs-`$BASH_ENV` precedence ordering between this step and
/// the command's own `run` step. No-op when `<resolved_key>_env_var` is
/// empty (the default): every existing literal-only consumer is unaffected.
fn build_resolve_workspace_param_step(resolved_key: &str) -> serde_yaml::Value {
    let env_var_param = format!("{resolved_key}_env_var");
    let source_file_param = format!("{resolved_key}_source_file");
    let override_var = env_var_name(&format!("{resolved_key}_resolved"));

    let mut run_map = serde_yaml::Mapping::new();
    run_map.insert(
        serde_yaml::Value::String("name".to_string()),
        serde_yaml::Value::String(format!("Resolve {resolved_key} from attached workspace")),
    );
    run_map.insert(
        serde_yaml::Value::String("command".to_string()),
        serde_yaml::Value::String("<<include(scripts/resolve_workspace_param.sh)>>".to_string()),
    );
    let mut env_map = serde_yaml::Mapping::new();
    env_map.insert(
        serde_yaml::Value::String("GCO_TARGET_ENV_VAR".to_string()),
        serde_yaml::Value::String(format!("<< parameters.{env_var_param} >>")),
    );
    env_map.insert(
        serde_yaml::Value::String("GCO_SOURCE_FILE".to_string()),
        serde_yaml::Value::String(format!("<< parameters.{source_file_param} >>")),
    );
    env_map.insert(
        serde_yaml::Value::String("GCO_WORKSPACE_ROOT".to_string()),
        serde_yaml::Value::String("<< parameters.workspace_root >>".to_string()),
    );
    env_map.insert(
        serde_yaml::Value::String("GCO_OVERRIDE_VAR".to_string()),
        serde_yaml::Value::String(override_var),
    );
    run_map.insert(
        serde_yaml::Value::String("environment".to_string()),
        serde_yaml::Value::Mapping(env_map),
    );
    let mut run_step = serde_yaml::Mapping::new();
    run_step.insert(
        serde_yaml::Value::String("run".to_string()),
        serde_yaml::Value::Mapping(run_map),
    );
    let inner_steps = serde_yaml::Value::Sequence(vec![serde_yaml::Value::Mapping(run_step)]);
    let mut when_inner = serde_yaml::Mapping::new();
    when_inner.insert(
        serde_yaml::Value::String("condition".to_string()),
        serde_yaml::Value::String(format!("<< parameters.{env_var_param} >>")),
    );
    when_inner.insert(serde_yaml::Value::String("steps".to_string()), inner_steps);
    let mut when_map = serde_yaml::Mapping::new();
    when_map.insert(
        serde_yaml::Value::String("when".to_string()),
        serde_yaml::Value::Mapping(when_inner),
    );
    serde_yaml::Value::Mapping(when_map)
}

/// The generic script `build_resolve_workspace_param_step` includes: given
/// `GCO_TARGET_ENV_VAR` (required) and `GCO_WORKSPACE_ROOT`, sources
/// `GCO_SOURCE_FILE` (default `<workspace_root>/versions.env`), extracts
/// `GCO_TARGET_ENV_VAR` via indirect expansion, and exports it to `$BASH_ENV`
/// under `GCO_OVERRIDE_VAR`'s name. Loud, specific errors — never a silent
/// empty fallback — mirroring `add-workspace-to-path.sh`'s own directness.
const RESOLVE_WORKSPACE_PARAM_SCRIPT: &str = r#"if [[ -z "${GCO_TARGET_ENV_VAR:-}" ]]; then
  echo "ERROR: GCO_TARGET_ENV_VAR must be set to use workspace-sourced resolution" >&2
  exit 1
fi

SOURCE_FILE="${GCO_SOURCE_FILE:-${GCO_WORKSPACE_ROOT}/versions.env}"

if [[ ! -f "${SOURCE_FILE}" ]]; then
  echo "ERROR: workspace source file not found: ${SOURCE_FILE}" >&2
  echo "Did you set attach_workspace: true and persist it from an earlier job?" >&2
  exit 1
fi

# shellcheck source=/dev/null
source "${SOURCE_FILE}"

RESOLVED="${!GCO_TARGET_ENV_VAR:-}"
if [[ -z "${RESOLVED}" ]]; then
  echo "ERROR: ${GCO_TARGET_ENV_VAR} not found (or empty) in ${SOURCE_FILE}" >&2
  exit 1
fi

echo "Resolved ${GCO_TARGET_ENV_VAR} from workspace: ${RESOLVED}"
echo "export ${GCO_OVERRIDE_VAR}=${RESOLVED}" >> "$BASH_ENV"
"#;

/// Boolean job parameter that toggles persisting the regenerated orb dir to the
/// workspace (Model B). Off by default; the consumer workflow sets it on the
/// regenerate-orb step so pack/review/push downstream jobs receive the orb.
fn build_persist_orb_param() -> OrbParameter {
    OrbParameter {
        param_type: "boolean".to_string(),
        description: "Persist the regenerated orb dir to the workspace so downstream \
                      jobs (pack, review, the end-of-workflow push job) operate on it \
                      without an immediate push. Default false."
            .to_string(),
        default: Some(serde_yaml::Value::Bool(false)),
        enum_values: None,
    }
}

/// Boolean job parameter (default true) gating the CI-wiring drift check.
fn build_check_ci_wiring_param() -> OrbParameter {
    OrbParameter {
        param_type: "boolean".to_string(),
        description: "Verify the consumer's CI wiring is in sync with this orb \
                      version's generated flow (runs `gen-circleci-orb update --check`); \
                      fails with upgrade instructions when out of date. Default false — \
                      opt in on the validation job; never run it at release (the \
                      freshly-built release binary is ahead of the published orb pin, \
                      so the check would deadlock the publish)."
            .to_string(),
        default: Some(serde_yaml::Value::Bool(false)),
        enum_values: None,
    }
}

/// Conditional step: when `check_ci_wiring` is true, run `gen-circleci-orb update
/// --check` so a consumer whose CI wiring has drifted from the current generator
/// flow gets a failing build with guidance. Delivered via the orb version (this
/// step is part of the orb job), so even a config on the old wiring triggers it.
fn build_check_ci_wiring_step() -> serde_yaml::Value {
    let mut run_map = serde_yaml::Mapping::new();
    run_map.insert(
        serde_yaml::Value::String("name".to_string()),
        serde_yaml::Value::String("Check the CI wiring is current".to_string()),
    );
    run_map.insert(
        serde_yaml::Value::String("command".to_string()),
        serde_yaml::Value::String("gen-circleci-orb update --check".to_string()),
    );
    let mut run_step = serde_yaml::Mapping::new();
    run_step.insert(
        serde_yaml::Value::String("run".to_string()),
        serde_yaml::Value::Mapping(run_map),
    );
    let inner_steps = serde_yaml::Value::Sequence(vec![serde_yaml::Value::Mapping(run_step)]);
    let mut when_inner = serde_yaml::Mapping::new();
    when_inner.insert(
        serde_yaml::Value::String("condition".to_string()),
        serde_yaml::Value::String("<< parameters.check_ci_wiring >>".to_string()),
    );
    when_inner.insert(serde_yaml::Value::String("steps".to_string()), inner_steps);
    let mut when_map = serde_yaml::Mapping::new();
    when_map.insert(
        serde_yaml::Value::String("when".to_string()),
        serde_yaml::Value::Mapping(when_inner),
    );
    serde_yaml::Value::Mapping(when_map)
}

/// Optional string job parameter naming a branch to switch onto before invoking
/// the command. Empty (the default) is a no-op. See
/// `build_target_branch_switch_step` for why this exists.
fn build_target_branch_param() -> OrbParameter {
    OrbParameter {
        param_type: "string".to_string(),
        description: "Branch to switch onto (fetch + checkout) right after the initial \
                      checkout, overriding CIRCLE_BRANCH to match. Empty (default) is a \
                      no-op — the job stays on whatever branch triggered the pipeline. \
                      Needed for a \"pr merged\"-triggered pipeline (gen-circleci-orb#328), \
                      where `checkout` lands on the deleted PR branch and CIRCLE_BRANCH \
                      stays stale — set this to the real target (e.g. `main`) so the \
                      generate invocation, and any auto-record push it makes, operates on \
                      the right branch."
            .to_string(),
        default: Some(serde_yaml::Value::String(String::new())),
        enum_values: None,
    }
}

/// Conditional step: when `target_branch` is non-empty, fetch and check it out,
/// then override `CIRCLE_BRANCH` in `$BASH_ENV` so tools reading it (e.g. the
/// record push) see the new branch, not the stale value `checkout` left behind.
/// Plain git — this job runs in the orb's own `default` executor, not a
/// toolkit container, so no extra tool dependency is introduced.
fn build_target_branch_switch_step() -> serde_yaml::Value {
    let mut run_map = serde_yaml::Mapping::new();
    run_map.insert(
        serde_yaml::Value::String("name".to_string()),
        serde_yaml::Value::String("Switch onto target_branch".to_string()),
    );
    run_map.insert(
        serde_yaml::Value::String("command".to_string()),
        serde_yaml::Value::String(
            "git fetch origin << parameters.target_branch >>\n\
             git checkout -B << parameters.target_branch >> origin/<< parameters.target_branch >>\n\
             echo 'export CIRCLE_BRANCH=<< parameters.target_branch >>' >> \"$BASH_ENV\"\n"
                .to_string(),
        ),
    );
    let mut run_step = serde_yaml::Mapping::new();
    run_step.insert(
        serde_yaml::Value::String("run".to_string()),
        serde_yaml::Value::Mapping(run_map),
    );
    let inner_steps = serde_yaml::Value::Sequence(vec![serde_yaml::Value::Mapping(run_step)]);
    let mut when_inner = serde_yaml::Mapping::new();
    when_inner.insert(
        serde_yaml::Value::String("condition".to_string()),
        serde_yaml::Value::String("<< parameters.target_branch >>".to_string()),
    );
    when_inner.insert(serde_yaml::Value::String("steps".to_string()), inner_steps);
    let mut when_map = serde_yaml::Mapping::new();
    when_map.insert(
        serde_yaml::Value::String("when".to_string()),
        serde_yaml::Value::Mapping(when_inner),
    );
    serde_yaml::Value::Mapping(when_map)
}

/// Optional string job parameter naming the SSH key fingerprint used to push the
/// regenerated orb with write authority. Empty (the default) means no key is
/// loaded and the push falls back to the ambient environment credentials.
///
/// This is a fingerprint *value* (a hash of the public key — not a secret), not
/// an env-var name: CircleCI resolves `add_ssh_keys` fingerprints at config-compile
/// time and cannot interpolate environment variables there.
fn build_ssh_fingerprint_param() -> OrbParameter {
    OrbParameter {
        param_type: "string".to_string(),
        description: "SSH key fingerprint (a public-key hash, not a secret) used to push the \
                      regenerated orb with write authority. When set, the key is loaded and \
                      the read-only checkout key is dropped from the agent. Empty (default) \
                      falls back to the ambient environment credentials."
            .to_string(),
        default: Some(serde_yaml::Value::String(String::new())),
        enum_values: None,
    }
}

/// Conditional step: when `ssh_fingerprint` is non-empty, `add_ssh_keys` for it
/// and remove the read-only checkout key (`~/.ssh/id_rsa.pub`) from the agent so
/// the subsequent push authenticates with the write key. Mirrors the toolkit
/// push pattern. No-op when the fingerprint is empty.
fn build_ssh_setup_step() -> serde_yaml::Value {
    let mut fingerprints = serde_yaml::Mapping::new();
    fingerprints.insert(
        serde_yaml::Value::String("fingerprints".to_string()),
        serde_yaml::Value::Sequence(vec![serde_yaml::Value::String(
            "<< parameters.ssh_fingerprint >>".to_string(),
        )]),
    );
    let mut add_keys_step = serde_yaml::Mapping::new();
    add_keys_step.insert(
        serde_yaml::Value::String("add_ssh_keys".to_string()),
        serde_yaml::Value::Mapping(fingerprints),
    );
    let mut trim_run = serde_yaml::Mapping::new();
    trim_run.insert(
        serde_yaml::Value::String("name".to_string()),
        serde_yaml::Value::String("Drop read-only checkout key from agent".to_string()),
    );
    trim_run.insert(
        serde_yaml::Value::String("command".to_string()),
        serde_yaml::Value::String("ssh-add -d ~/.ssh/id_rsa.pub || true".to_string()),
    );
    let mut trim_step = serde_yaml::Mapping::new();
    trim_step.insert(
        serde_yaml::Value::String("run".to_string()),
        serde_yaml::Value::Mapping(trim_run),
    );
    let inner_steps = serde_yaml::Value::Sequence(vec![
        serde_yaml::Value::Mapping(add_keys_step),
        serde_yaml::Value::Mapping(trim_step),
    ]);
    let mut when_inner = serde_yaml::Mapping::new();
    when_inner.insert(
        serde_yaml::Value::String("condition".to_string()),
        serde_yaml::Value::String("<< parameters.ssh_fingerprint >>".to_string()),
    );
    when_inner.insert(serde_yaml::Value::String("steps".to_string()), inner_steps);
    let mut when_map = serde_yaml::Mapping::new();
    when_map.insert(
        serde_yaml::Value::String("when".to_string()),
        serde_yaml::Value::Mapping(when_inner),
    );
    serde_yaml::Value::Mapping(when_map)
}

/// Conditional step: when `persist_orb_workspace` is true, persist the orb dir
/// (named by the `orb_dir` parameter) to the workspace, rooted at the repo so the
/// path stays `<< parameters.orb_dir >>` for downstream `attach_workspace` jobs.
fn build_persist_orb_step() -> serde_yaml::Value {
    let mut persist_map = serde_yaml::Mapping::new();
    persist_map.insert(
        serde_yaml::Value::String("root".to_string()),
        serde_yaml::Value::String(".".to_string()),
    );
    persist_map.insert(
        serde_yaml::Value::String("paths".to_string()),
        serde_yaml::Value::Sequence(vec![serde_yaml::Value::String(
            "<< parameters.orb_dir >>".to_string(),
        )]),
    );
    let mut persist_step = serde_yaml::Mapping::new();
    persist_step.insert(
        serde_yaml::Value::String("persist_to_workspace".to_string()),
        serde_yaml::Value::Mapping(persist_map),
    );
    let inner_steps = serde_yaml::Value::Sequence(vec![serde_yaml::Value::Mapping(persist_step)]);
    let mut when_inner = serde_yaml::Mapping::new();
    when_inner.insert(
        serde_yaml::Value::String("condition".to_string()),
        serde_yaml::Value::String("<< parameters.persist_orb_workspace >>".to_string()),
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
    let default = orb_param_default(p);
    OrbParameter {
        param_type: type_str,
        description: p.description.clone(),
        default,
        enum_values: enum_vals,
    }
}

/// A job-group step: the leaf plus the effective name (bare, or
/// collision-qualified) its own command/job/script are rendered under — the
/// config-section key for its param overrides (gen-circleci-orb#425).
type ResolvedStep<'a> = (&'a SubCommand, String);

/// Finds the first leaf named `name` (depth-first; job groups select steps by
/// bare name) together with its effective name from `effective_names`.
fn find_leaf_subcommand<'a>(
    cli: &'a CliDefinition,
    name: &str,
    effective_names: &HashMap<String, String>,
) -> Option<ResolvedStep<'a>> {
    fn search<'a>(
        subs: &'a [SubCommand],
        prefix: &str,
        name: &str,
        effective_names: &HashMap<String, String>,
    ) -> Option<ResolvedStep<'a>> {
        for sub in subs {
            let path = if prefix.is_empty() {
                sub.name.clone()
            } else {
                format!("{prefix}.{}", sub.name)
            };
            if sub.is_leaf && sub.name == name {
                let effective = effective_names
                    .get(&path)
                    .cloned()
                    .unwrap_or_else(|| sub.name.clone());
                return Some((sub, effective));
            }
            if let Some(found) = search(&sub.subcommands, &path, name, effective_names) {
                return Some(found);
            }
        }
        None
    }
    search(&cli.subcommands, "", name, effective_names)
}

/// Resolve the JOB-level key for an inherited param — one job value shared by
/// every step that inherits it, so it has no single subcommand to scope by
/// (gen-circleci-orb#422): bare, unless CircleCI reserves that name for a
/// job, in which case it is scoped to the job group instead.
fn resolve_job_group_param_name(group_name: &str, param_name: &str) -> String {
    prefix_if_reserved(RESERVED_JOB_PARAMS, group_name, param_name)
}

/// Where a job-level key came from. Two declarations landing on one key is
/// only legitimate when they are the same declaration: every step that
/// inherits an option shares one key by design, whereas a step's own option
/// is its own.
#[derive(Debug, Clone, PartialEq)]
struct KeyOrigin {
    id: String,
    label: String,
}

impl KeyOrigin {
    fn inherited(long_name: &str) -> Self {
        Self {
            id: format!("inherited:{long_name}"),
            label: format!("inherited '--{long_name}'"),
        }
    }

    fn step(sub: &SubCommand, long_name: &str) -> Self {
        Self {
            id: format!("step:{}:{long_name}", sub.name),
            label: format!("'--{long_name}' on step '{}'", sub.name),
        }
    }
}

/// A job group's parameter set together with the one table that says which
/// job key each step's parameter is wired to. Built once by the producer side
/// (`build_job_group_params`), so the invoke step reads its key from here
/// instead of re-deriving which naming scheme produced it — a guess that
/// depended on no two schemes ever yielding the same string.
#[derive(Default)]
struct JobGroupParams {
    params: IndexMap<String, OrbParameter>,
    /// `(step index, param long_name)` → the job key it is declared under.
    keys: HashMap<(usize, String), String>,
    origins: HashMap<String, KeyOrigin>,
    /// Two different declarations resolving to one job key.
    collisions: Vec<String>,
}

impl JobGroupParams {
    /// Wires `step_idx`'s `long_name` to `key`, declaring the job parameter
    /// the first time the key is seen. A second, DIFFERENT declaration on the
    /// same key is recorded as a collision and keeps the first.
    fn bind(
        &mut self,
        group_name: &str,
        step_idx: usize,
        p: &Parameter,
        key: String,
        origin: KeyOrigin,
    ) {
        match self.origins.get(&key) {
            None => {
                self.params.insert(key.clone(), cli_param_to_orb_param(p));
                self.origins.insert(key.clone(), origin);
            }
            Some(existing) if *existing == origin => {}
            Some(existing) => self.collisions.push(format!(
                "job_group '{group_name}': {} and {} both resolve to job parameter '{key}' — \
                 set [subcommand.<step>.param.<flag>] orb_name = \"...\" on the step's own \
                 option to give it a different key",
                existing.label, origin.label
            )),
        }
        self.keys.insert((step_idx, p.long_name.clone()), key);
    }
}

/// Only an `inherited` option (declared by an ancestor command, so genuinely
/// the same input in every step) is unified into one job parameter. A name
/// that merely two steps each declare for themselves is NOT shared — the
/// options may mean different things (`generate --output` vs `release
/// --output`), so unifying them would force one value on both
/// (gen-circleci-orb#423). Each declaring step is judged on its own: an
/// inherited declaration shares one key, a step's own declaration gets its
/// own even when another step's same-named option happens to be inherited.
fn bind_explicit_params(
    out: &mut JobGroupParams,
    group_name: &str,
    explicit: &[String],
    steps: &[ResolvedStep],
    config: Option<&OrbConfig>,
) {
    for param_name in explicit {
        for (idx, (sub, effective_name)) in steps.iter().enumerate() {
            let Some(p) = sub.parameters.iter().find(|p| &p.long_name == param_name) else {
                continue;
            };
            if p.inherited {
                let key = resolve_job_group_param_name(group_name, param_name);
                out.bind(group_name, idx, p, key, KeyOrigin::inherited(param_name));
            } else {
                let key = per_step_job_key(sub, effective_name, p, steps, config);
                out.bind(group_name, idx, p, key, KeyOrigin::step(sub, param_name));
            }
        }
    }
}

/// Default mode: the unified set is the inherited options present on every
/// step — never a same-named option the steps each declared independently.
fn bind_shared_params(out: &mut JobGroupParams, group_name: &str, steps: &[ResolvedStep]) {
    let Some(((first, _), rest)) = steps.split_first() else {
        return;
    };
    for shared in first.parameters.iter().filter(|p| p.inherited) {
        let on_every_step = rest.iter().all(|(sub, _)| {
            sub.parameters
                .iter()
                .any(|op| op.inherited && op.long_name == shared.long_name)
        });
        if !on_every_step {
            continue;
        }
        let key = resolve_job_group_param_name(group_name, &shared.long_name);
        for (idx, (sub, _)) in steps.iter().enumerate() {
            if let Some(p) = sub
                .parameters
                .iter()
                .find(|p| p.long_name == shared.long_name)
            {
                out.bind(
                    group_name,
                    idx,
                    p,
                    key.clone(),
                    KeyOrigin::inherited(&shared.long_name),
                );
            }
        }
    }
}

/// The job-level key for one step's own parameter: its resolved command-level
/// key (`resolve_param_orb_name`), prefixed `{sub}_` when another step
/// declares the same key or the key is one a JOB parameter may not use.
fn per_step_job_key(
    sub: &SubCommand,
    effective_name: &str,
    p: &Parameter,
    steps: &[ResolvedStep],
    config: Option<&OrbConfig>,
) -> String {
    let resolved = resolve_param_orb_name(&sub.name, effective_name, &p.long_name, config);
    let collides = steps.iter().any(|(other, other_effective)| {
        other.name != sub.name
            && other.parameters.iter().any(|op| {
                resolve_param_orb_name(&other.name, other_effective, &op.long_name, config)
                    == resolved
            })
    });
    // `resolved` is only renamed away from the bare CLI name when
    // RESTRICTED_COMMAND_PARAMS (just "name") applies. A JOB parameter has a
    // broader reserved set (RESERVED_JOB_PARAMS: type/filters/matrix/
    // requires/context/pre_steps/post_steps too), so a param like "type"
    // passes through unrenamed yet is still invalid as a bare job key.
    if collides || RESERVED_JOB_PARAMS.contains(&resolved.as_str()) {
        scoped(&sub.name, &resolved)
    } else {
        resolved
    }
}

/// A required, non-boolean param not already bound by shared/explicit
/// selection gets a mandatory slot here, under its own per-step key
/// (`per_step_job_key`: the restricted-name rename / `orb_name` override of
/// #369/#412, `{sub}_`-prefixed on a cross-step collision).
fn bind_mandatory_params(
    out: &mut JobGroupParams,
    group_name: &str,
    steps: &[ResolvedStep],
    config: Option<&OrbConfig>,
) {
    for (idx, (sub, effective_name)) in steps.iter().enumerate() {
        for p in &sub.parameters {
            if !p.required || matches!(p.param_type, ParamType::Boolean) {
                continue;
            }
            // An inherited param already bound under its shared key must not
            // ALSO get a per-step duplicate on top of it.
            if p.inherited && out.keys.contains_key(&(idx, p.long_name.clone())) {
                continue;
            }
            let key = per_step_job_key(sub, effective_name, p, steps, config);
            out.bind(group_name, idx, p, key, KeyOrigin::step(sub, &p.long_name));
        }
    }
}

fn build_job_group_params(
    group: &crate::orb_config::JobGroup,
    steps: &[ResolvedStep],
    config: Option<&OrbConfig>,
) -> JobGroupParams {
    let mut out = JobGroupParams::default();
    if let Some(explicit) = &group.params {
        bind_explicit_params(&mut out, &group.name, explicit, steps, config);
    } else {
        bind_shared_params(&mut out, &group.name, steps);
    }
    bind_mandatory_params(&mut out, &group.name, steps, config);
    out
}

/// Every job-group parameter-key collision across the config's simple-mode
/// groups (rich-mode groups declare their own parameters, so have nothing to
/// collide). Checked before rendering, alongside `validate_param_key_collisions`,
/// so a collision fails loudly instead of one declaration silently
/// overwriting another.
pub(crate) fn job_group_key_collisions(
    cli: &CliDefinition,
    config: Option<&OrbConfig>,
    effective_names: &HashMap<String, String>,
) -> Vec<String> {
    let mut errors = Vec::new();
    for group in config
        .and_then(|c| c.job_group.as_ref())
        .into_iter()
        .flatten()
        .filter(|g| g.step.is_none())
    {
        let steps: Vec<ResolvedStep> = group
            .steps
            .iter()
            .filter_map(|name| find_leaf_subcommand(cli, name, effective_names))
            .collect();
        errors.extend(build_job_group_params(group, &steps, config).collisions);
    }
    errors
}

/// The invoke step's key (left side) must match the SAME key the invoked
/// command itself declares that parameter under — `resolve_param_orb_name`,
/// not the bare CLI flag name (gen-circleci-orb#413) — while the value
/// (right side) is whatever job key `build_job_group_params` bound that
/// step's parameter to.
fn build_job_group_invoke_step(
    step_idx: usize,
    (sub, effective_name): &ResolvedStep,
    keys: &HashMap<(usize, String), String>,
    config: Option<&OrbConfig>,
) -> serde_yaml::Value {
    let mut invoke_map = serde_yaml::Mapping::new();
    for p in &sub.parameters {
        let Some(job_name) = keys.get(&(step_idx, p.long_name.clone())) else {
            continue;
        };
        let command_key = resolve_param_orb_name(&sub.name, effective_name, &p.long_name, config);
        invoke_map.insert(
            serde_yaml::Value::String(command_key),
            serde_yaml::Value::String(format!("<< parameters.{job_name} >>")),
        );
    }
    serde_yaml::Value::Mapping({
        let mut m = serde_yaml::Mapping::new();
        m.insert(
            serde_yaml::Value::String(effective_name.replace('-', "_")),
            serde_yaml::Value::Mapping(invoke_map),
        );
        m
    })
}

/// Coerce a rich-mode `with` value into a typed YAML scalar.
///
/// `true`/`false` become YAML booleans (orb boolean parameters reject quoted
/// string values); every other value — string literals, integers, and
/// `<< parameters.x >>` references — is emitted as a plain string.
fn with_value_to_yaml(value: &str) -> serde_yaml::Value {
    match value {
        "true" => serde_yaml::Value::Bool(true),
        "false" => serde_yaml::Value::Bool(false),
        other => serde_yaml::Value::String(other.to_string()),
    }
}

/// Build an invoke step for a tool command or third-party orb command.
///
/// With no parameter values the step is a bare list item (`- set_https_remote`);
/// otherwise it is a mapping (`- generate: {format: binary, ...}`).
fn build_with_invoke_step(key: &str, with: Option<&IndexMap<String, String>>) -> serde_yaml::Value {
    match with {
        Some(values) if !values.is_empty() => {
            let mut params_map = serde_yaml::Mapping::new();
            for (name, value) in values {
                params_map.insert(
                    serde_yaml::Value::String(name.clone()),
                    with_value_to_yaml(value),
                );
            }
            let mut m = serde_yaml::Mapping::new();
            m.insert(
                serde_yaml::Value::String(key.to_string()),
                serde_yaml::Value::Mapping(params_map),
            );
            serde_yaml::Value::Mapping(m)
        }
        _ => serde_yaml::Value::String(key.to_string()),
    }
}

/// Build a custom `run:` step (named, with inline shell body and optional env block).
/// Convert an arbitrary step name into a snake_case identifier suitable for a
/// script filename (e.g. "Set up git and environment" -> "set_up_git_and_environment").
fn snake_case(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

fn build_custom_run_step(
    name: &str,
    command: Option<&str>,
    environment: Option<&IndexMap<String, String>>,
) -> serde_yaml::Value {
    let mut run_map = serde_yaml::Mapping::new();
    run_map.insert(
        serde_yaml::Value::String("name".to_string()),
        serde_yaml::Value::String(name.to_string()),
    );
    if let Some(cmd) = command {
        run_map.insert(
            serde_yaml::Value::String("command".to_string()),
            serde_yaml::Value::String(cmd.to_string()),
        );
    }
    if let Some(env) = environment {
        let mut env_map = serde_yaml::Mapping::new();
        for (key, value) in env {
            env_map.insert(
                serde_yaml::Value::String(key.clone()),
                serde_yaml::Value::String(value.clone()),
            );
        }
        run_map.insert(
            serde_yaml::Value::String("environment".to_string()),
            serde_yaml::Value::Mapping(env_map),
        );
    }
    let mut m = serde_yaml::Mapping::new();
    m.insert(
        serde_yaml::Value::String("run".to_string()),
        serde_yaml::Value::Mapping(run_map),
    );
    serde_yaml::Value::Mapping(m)
}

/// Render a rich-mode job_group: explicit parameter declarations and an ordered,
/// heterogeneous step list (built-ins, tool commands, third-party orb steps and
/// custom run steps). The whole job is data declared in the config file.
fn render_rich_job_group(
    group: &crate::orb_config::JobGroup,
    files: &mut HashMap<PathBuf, String>,
) -> String {
    let group_snake = group.name.replace('-', "_");
    let mut parameters: IndexMap<String, OrbParameter> = IndexMap::new();
    if let Some(declared) = &group.parameter {
        for p in declared {
            let param_type = p.param_type.clone().unwrap_or_else(|| "string".to_string());
            parameters.insert(
                p.name.clone(),
                OrbParameter {
                    default: p
                        .default
                        .as_deref()
                        .map(|d| coerce_override_default(d, &param_type)),
                    param_type,
                    description: p.description.clone().unwrap_or_default(),
                    enum_values: None,
                },
            );
        }
    }

    let mut steps: Vec<serde_yaml::Value> = Vec::new();
    let mut uses_attach_workspace = false;
    for step in group.step.as_deref().unwrap_or_default() {
        if let Some(builtin) = &step.builtin {
            match builtin.as_str() {
                "attach_workspace" => {
                    steps.push(build_attach_workspace_step());
                    uses_attach_workspace = true;
                }
                // checkout and any other built-in step name emit as a bare step.
                other => steps.push(serde_yaml::Value::String(other.to_string())),
            }
        } else if let Some(run_name) = &step.run {
            // Externalize the script to a scripts/ file and reference it via
            // <<include(...)>> so the generated orb stays RC009-compliant
            // (orb-tools flags long inline run commands).
            let command = step.script.as_ref().map(|script| {
                let script_name = format!("{group_snake}_{}", snake_case(run_name));
                files.insert(
                    PathBuf::from(format!("src/scripts/{script_name}.sh")),
                    format!("{}\n", script.trim()),
                );
                format!("<<include(scripts/{script_name}.sh)>>")
            });
            steps.push(build_custom_run_step(
                run_name,
                command.as_deref(),
                step.environment.as_ref(),
            ));
        } else if let Some(orb_ref) = &step.orb {
            steps.push(build_with_invoke_step(orb_ref, step.with.as_ref()));
        } else if let Some(command) = &step.command {
            let key = command.replace('-', "_");
            steps.push(build_with_invoke_step(&key, step.with.as_ref()));
        }
    }

    if uses_attach_workspace {
        let (attach_param, root_param) = build_workspace_params();
        parameters
            .entry("attach_workspace".to_string())
            .or_insert(attach_param);
        parameters
            .entry("workspace_root".to_string())
            .or_insert(root_param);
    }

    let description = group
        .description
        .clone()
        .unwrap_or_else(|| format!("Composite job: {}.", group.name));

    let job = OrbJob {
        description,
        executor: group
            .executor
            .clone()
            .unwrap_or_else(|| "default".to_string()),
        parameters,
        steps,
    };
    serde_yaml::to_string(&job).unwrap()
}

fn render_job_group(
    group: &crate::orb_config::JobGroup,
    cli: &CliDefinition,
    config: Option<&OrbConfig>,
    effective_names: &HashMap<String, String>,
    files: &mut HashMap<PathBuf, String>,
) -> String {
    // Rich mode (explicit `step` list) takes precedence over the simple `steps` list.
    if group.step.is_some() {
        return render_rich_job_group(group, files);
    }
    let steps_resolved: Vec<ResolvedStep> = group
        .steps
        .iter()
        .filter_map(|name| find_leaf_subcommand(cli, name, effective_names))
        .collect();

    let JobGroupParams {
        params: mut parameters,
        keys,
        ..
    } = build_job_group_params(group, &steps_resolved, config);

    let (attach_param, root_param) = build_workspace_params();
    parameters.insert("attach_workspace".to_string(), attach_param);
    parameters.insert("workspace_root".to_string(), root_param);

    let mut steps = vec![
        serde_yaml::Value::String("checkout".to_string()),
        build_attach_workspace_step(),
    ];
    for (idx, step) in steps_resolved.iter().enumerate() {
        steps.push(build_job_group_invoke_step(idx, step, &keys, config));
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
    use pretty_assertions::assert_eq;

    #[test]
    fn command_param_name_snake_cases_multiword_subcommand() {
        // A restricted param under a hyphenated subcommand must be fully
        // snake_cased so the orb param key passes RC010 (no hyphens).
        assert_eq!(
            resolve_command_param_name("add-job-group", "name"),
            "add_job_group_name"
        );
        // Single-word subcommand unchanged.
        assert_eq!(
            resolve_command_param_name("generate", "name"),
            "generate_name"
        );
        // Non-restricted param passes through untouched.
        assert_eq!(
            resolve_command_param_name("add-job-group", "steps"),
            "steps"
        );
    }

    #[test]
    fn param_orb_name_falls_back_to_restricted_rename_with_no_config() {
        // No config at all, or no matching override: behaves exactly like
        // resolve_command_param_name (gen-circleci-orb#412's fallback path).
        assert_eq!(
            resolve_param_orb_name("generate", "generate", "name", None),
            "generate_name"
        );
        assert_eq!(
            resolve_param_orb_name("generate", "generate", "steps", None),
            "steps"
        );
    }

    #[test]
    fn param_orb_name_config_lookup_uses_the_effective_name_not_the_bare_one() {
        // gen-circleci-orb#425: a colliding leaf (bare `release`, effective
        // `ci_release`) must find ITS override under the qualified section,
        // and must NOT pick up a section keyed by the bare name. The
        // automatic restricted rename still uses the bare name (what the
        // consumer sees on the command line).
        let mk = |section: &str| {
            let mut overrides = IndexMap::new();
            overrides.insert(
                "name".to_string(),
                crate::orb_config::ParamOverride {
                    default: None,
                    orb_name: Some("custom_name".to_string()),
                    workspace_sourced: None,
                },
            );
            let mut subcommands = IndexMap::new();
            subcommands.insert(
                section.to_string(),
                crate::orb_config::SubcommandConfig {
                    param: Some(overrides),
                    ..Default::default()
                },
            );
            OrbConfig {
                subcommand: Some(subcommands),
                ..Default::default()
            }
        };
        assert_eq!(
            resolve_param_orb_name("release", "ci_release", "name", Some(&mk("ci_release"))),
            "custom_name"
        );
        assert_eq!(
            resolve_param_orb_name("release", "ci_release", "name", Some(&mk("release"))),
            "release_name",
            "a section keyed by the bare name must not apply to the qualified leaf"
        );
    }

    #[test]
    fn param_orb_name_honors_an_explicit_override() {
        // [subcommand.generate.param.generate_name] orb_name = "..." must win
        // over both the bare CLI flag name and automatic restricted renaming
        // -- the resolution path for a renamed-key collision (#412).
        let mut param_overrides = IndexMap::new();
        param_overrides.insert(
            "generate_name".to_string(),
            crate::orb_config::ParamOverride {
                default: None,
                orb_name: Some("generate_name_alt".to_string()),
                workspace_sourced: None,
            },
        );
        let mut subcommands = IndexMap::new();
        subcommands.insert(
            "generate".to_string(),
            crate::orb_config::SubcommandConfig {
                param: Some(param_overrides),
                ..Default::default()
            },
        );
        let config = OrbConfig {
            subcommand: Some(subcommands),
            ..Default::default()
        };

        assert_eq!(
            resolve_param_orb_name("generate", "generate", "generate_name", Some(&config)),
            "generate_name_alt"
        );
        // A restricted param under the same subcommand, with no override of
        // its own, still falls back to automatic renaming untouched.
        assert_eq!(
            resolve_param_orb_name("generate", "generate", "name", Some(&config)),
            "generate_name"
        );
    }

    #[test]
    fn enum_param_without_default_uses_first_value() {
        // An enum param with no CLI default must default to a valid enum value
        // (the first), never "" — `circleci orb validate` rejects an empty
        // default for an enum parameter.
        let p = Parameter {
            long_name: "install_method".to_string(),
            short: None,
            param_type: ParamType::Enum(vec![
                "binstall".to_string(),
                "apt".to_string(),
                "local".to_string(),
            ]),
            default: None,
            required: false,
            description: "How the binary is installed".to_string(),
            ..Default::default()
        };
        let orb = cli_param_to_orb_param(&p);
        assert_eq!(
            orb.default,
            Some(serde_yaml::Value::String("binstall".to_string()))
        );
    }

    #[test]
    fn job_and_command_builders_enum_default_is_first_value() {
        // Guards all param builders (the job builder was the one originally
        // missed, producing an invalid empty enum default in jobs/generate.yml).
        let sub = make_leaf(
            "demo",
            vec![Parameter {
                long_name: "method".to_string(),
                short: None,
                param_type: ParamType::Enum(vec!["binstall".to_string(), "apt".to_string()]),
                default: None,
                required: false,
                description: "install method".to_string(),
                ..Default::default()
            }],
        );
        let want = Some(serde_yaml::Value::String("binstall".to_string()));
        assert_eq!(
            build_orb_parameters(&sub, "demo", &[], None)["method"].default,
            want
        );
        assert_eq!(
            build_command_orb_parameters(&sub, "demo", None)["method"].default,
            want
        );
    }

    fn make_leaf(name: &str, params: Vec<Parameter>) -> SubCommand {
        SubCommand {
            name: name.to_string(),
            description: format!("Does {name} things."),
            short_about: format!("Does {name} things."),
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
            base_image: "debian:13-slim".to_string(),
            builder_image: "rust:1-slim-trixie".to_string(),
            home_url: None,
            source_url: None,
            binary_name: "mytool".to_string(),
            git_push_subcommands: vec![],
            circleci_cli_version: None,
            apt_packages: vec![],
            cargo_tools: vec![],
            crate_wait: CrateWait::default(),
        }
    }

    /// Test-only convenience: parse raw `"crate"` / `"crate:binary"` /
    /// `"crate@version"` entries into the `(crate, binary, version)` triples
    /// `GenerateOpts.cargo_tools` now holds, so tests can still write the
    /// familiar string form.
    fn parsed_cargo_tools(entries: &[&str]) -> Vec<(String, String, Option<String>)> {
        entries
            .iter()
            .map(|e| {
                let (krate, binary, version) = split_cargo_tool_entry(e).unwrap();
                (
                    krate.to_string(),
                    binary.to_string(),
                    version.map(str::to_string),
                )
            })
            .collect()
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
            content.contains("FROM rust:1-slim-trixie AS builder"),
            "should use rust:1-slim-trixie builder stage:\n{content}"
        );
        // Runtime stage is the slim Debian image
        assert!(
            content.contains("FROM debian:13-slim"),
            "should use debian:13-slim runtime stage:\n{content}"
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
    fn dockerfile_cargo_tools_installed_in_builder_and_copied_to_runtime() {
        let cli = make_cli("mytool", vec![]);
        let opts = GenerateOpts {
            cargo_tools: parsed_cargo_tools(&["cargo-audit", "cargo-deny"]),
            ..default_opts()
        };
        let files = generate(&cli, &opts, None);
        let content = &files[&PathBuf::from("Dockerfile")];
        assert!(
            content.contains("cargo install cargo-binstall --locked"),
            "builder should install cargo-binstall:\n{content}"
        );
        assert!(
            binstall_tools(content) == ["cargo-audit", "cargo-deny"],
            "builder should binstall the cargo tools:\n{content}"
        );
        assert!(
            content.contains(
                "COPY --from=builder /usr/local/cargo/bin/cargo-audit /usr/local/bin/cargo-audit"
            ),
            "runtime should copy cargo-audit:\n{content}"
        );
        assert!(
            content.contains(
                "COPY --from=builder /usr/local/cargo/bin/cargo-deny /usr/local/bin/cargo-deny"
            ),
            "runtime should copy cargo-deny:\n{content}"
        );
        assert!(
            !content.contains("| bash"),
            "must not use curl|bash:\n{content}"
        );
    }

    #[test]
    fn dockerfile_no_cargo_tools_omits_binstall_stage() {
        let cli = make_cli("mytool", vec![]);
        let content = &generate(&cli, &default_opts(), None)[&PathBuf::from("Dockerfile")];
        assert!(
            !content.contains("cargo binstall"),
            "no cargo_tools should mean no binstall line:\n{content}"
        );
        assert!(
            !content.contains("cargo-audit"),
            "no cargo_tools should mean no tool references:\n{content}"
        );
    }

    #[test]
    fn dockerfile_cargo_tools_are_sorted() {
        let cli = make_cli("mytool", vec![]);
        let opts = GenerateOpts {
            cargo_tools: parsed_cargo_tools(&["cargo-deny", "cargo-audit"]),
            ..default_opts()
        };
        let content = &generate(&cli, &opts, None)[&PathBuf::from("Dockerfile")];
        assert!(
            binstall_tools(content) == ["cargo-audit", "cargo-deny"],
            "tools should be emitted in sorted order:\n{content}"
        );
    }

    #[test]
    fn dockerfile_cargo_tools_supports_crate_binary_syntax() {
        let cli = make_cli("mytool", vec![]);
        let opts = GenerateOpts {
            cargo_tools: parsed_cargo_tools(&["cargo-audit", "rsign2:rsign"]),
            ..default_opts()
        };
        let content = &generate(&cli, &opts, None)[&PathBuf::from("Dockerfile")];
        assert!(
            binstall_tools(content) == ["cargo-audit", "rsign2"],
            "binstall should use the crate name, not the raw entry:\n{content}"
        );
        assert!(
            !content.contains("rsign2:rsign"),
            "the raw entry syntax must never reach the rendered Dockerfile:\n{content}"
        );
        assert!(
            content.contains("COPY --from=builder /usr/local/cargo/bin/rsign /usr/local/bin/rsign"),
            "runtime should copy the split binary name on both source and dest:\n{content}"
        );
        assert!(
            !content.contains("/usr/local/cargo/bin/rsign2"),
            "the crate name must never appear as a binstall output path:\n{content}"
        );
    }

    #[test]
    fn dockerfile_cargo_tools_pins_version_on_binstall_line() {
        let cli = make_cli("mytool", vec![]);
        let opts = GenerateOpts {
            cargo_tools: parsed_cargo_tools(&["cargo-audit@0.21.0", "rsign2:rsign@2.1.0"]),
            ..default_opts()
        };
        let content = &generate(&cli, &opts, None)[&PathBuf::from("Dockerfile")];
        assert!(
            binstall_tools(content) == ["cargo-audit@0.21.0", "rsign2@2.1.0"],
            "binstall should pin crate@version on the command line:\n{content}"
        );
        assert!(
            content.contains("COPY --from=builder /usr/local/cargo/bin/rsign /usr/local/bin/rsign"),
            "runtime should still copy the plain binary name, not a version-qualified path:\n{content}"
        );
    }

    #[test]
    fn dockerfile_cargo_tools_mixes_pinned_and_unpinned_entries() {
        let cli = make_cli("mytool", vec![]);
        let opts = GenerateOpts {
            cargo_tools: parsed_cargo_tools(&["cargo-audit", "cargo-deny@0.14.0"]),
            ..default_opts()
        };
        let content = &generate(&cli, &opts, None)[&PathBuf::from("Dockerfile")];
        assert!(
            binstall_tools(content) == ["cargo-audit", "cargo-deny@0.14.0"],
            "a bare entry should keep floating to latest alongside a pinned one:\n{content}"
        );
    }

    #[test]
    fn split_cargo_tool_entry_plain_name_uses_it_for_both() {
        assert_eq!(
            split_cargo_tool_entry("cargo-audit").unwrap(),
            ("cargo-audit", "cargo-audit", None)
        );
    }

    #[test]
    fn split_cargo_tool_entry_crate_colon_binary_splits() {
        assert_eq!(
            split_cargo_tool_entry("rsign2:rsign").unwrap(),
            ("rsign2", "rsign", None)
        );
    }

    #[test]
    fn split_cargo_tool_entry_crate_at_version_pins_without_alias() {
        assert_eq!(
            split_cargo_tool_entry("cargo-audit@0.21.0").unwrap(),
            ("cargo-audit", "cargo-audit", Some("0.21.0"))
        );
    }

    #[test]
    fn split_cargo_tool_entry_crate_colon_binary_at_version_pins_with_alias() {
        assert_eq!(
            split_cargo_tool_entry("rsign2:rsign@2.1.0").unwrap(),
            ("rsign2", "rsign", Some("2.1.0"))
        );
    }

    #[test]
    fn split_cargo_tool_entry_version_accepts_prerelease_and_build_metadata() {
        assert_eq!(
            split_cargo_tool_entry("cargo-audit@1.0.0-beta.1+abc123").unwrap(),
            ("cargo-audit", "cargo-audit", Some("1.0.0-beta.1+abc123"))
        );
    }

    #[test]
    fn split_cargo_tool_entry_rejects_empty_version() {
        assert!(split_cargo_tool_entry("cargo-audit@").is_err());
    }

    #[test]
    fn split_cargo_tool_entry_rejects_multiple_at_signs() {
        assert!(split_cargo_tool_entry("cargo-audit@1.0.0@2.0.0").is_err());
    }

    #[test]
    fn split_cargo_tool_entry_rejects_version_not_starting_with_digit() {
        // Real crates.io versions never carry a "v" prefix or a comparator —
        // this is an exact pin, not a range query.
        assert!(split_cargo_tool_entry("cargo-audit@v1.0.0").is_err());
        assert!(split_cargo_tool_entry("cargo-audit@<=1.3.3").is_err());
        assert!(split_cargo_tool_entry("cargo-audit@^1.0.0").is_err());
        assert!(split_cargo_tool_entry("cargo-audit@*").is_err());
    }

    #[test]
    fn split_cargo_tool_entry_rejects_shell_metacharacters_in_version() {
        for bad in [
            "crate@1.0$(id)",
            "crate@1.0`id`",
            "crate@1.0;rm -rf /",
            "crate@1.0 2.0",
        ] {
            assert!(
                split_cargo_tool_entry(bad).is_err(),
                "expected error for {bad:?}"
            );
        }
    }

    #[test]
    fn split_cargo_tool_entry_rejects_empty_entry() {
        assert!(split_cargo_tool_entry("").is_err());
    }

    #[test]
    fn split_cargo_tool_entry_rejects_empty_crate_half() {
        assert!(split_cargo_tool_entry(":rsign").is_err());
    }

    #[test]
    fn split_cargo_tool_entry_rejects_empty_binary_half() {
        assert!(split_cargo_tool_entry("rsign2:").is_err());
    }

    #[test]
    fn split_cargo_tool_entry_rejects_multiple_colons() {
        assert!(split_cargo_tool_entry("a:b:c").is_err());
    }

    #[test]
    fn split_cargo_tool_entry_rejects_whitespace_in_either_half() {
        // A stray space (e.g. "crate: binary") would otherwise pass syntax
        // validation and render a Dockerfile COPY path containing a literal
        // space, which fails the container build.
        assert!(split_cargo_tool_entry("rsign2: rsign").is_err());
        assert!(split_cargo_tool_entry(" rsign2:rsign").is_err());
        assert!(split_cargo_tool_entry("rsign2 :rsign").is_err());
        assert!(split_cargo_tool_entry("rsign2:rsign ").is_err());
        assert!(split_cargo_tool_entry("   ").is_err());
    }

    #[test]
    fn split_cargo_tool_entry_rejects_path_separator_in_either_half() {
        // cargo binstall writes a binary directly under
        // /usr/local/cargo/bin/<name>, never a nested path — a '/' in either
        // half would otherwise render a COPY source or destination path that
        // never exists.
        assert!(split_cargo_tool_entry("mycrate:bin/mybin").is_err());
        assert!(split_cargo_tool_entry("crate/name:mybin").is_err());
        assert!(split_cargo_tool_entry("crate/name").is_err());
    }

    #[test]
    fn split_cargo_tool_entry_rejects_shell_metacharacters() {
        // A crate name flows verbatim into a Dockerfile RUN instruction's
        // shell-form command line (render_cargo_tools_install); the binary
        // half flows into COPY paths. Neither is a real cargo/crates.io name
        // once it carries `$`, backticks, quotes, or a semicolon — restrict
        // both to the charset real crate/binary names actually use rather
        // than deny-listing shell metacharacters one at a time.
        for bad in [
            "crate:bin$(id)",
            "crate:bin`id`",
            "crate:bin;rm -rf /",
            "crate:bin\"x",
            "cra$te:bin",
        ] {
            assert!(
                split_cargo_tool_entry(bad).is_err(),
                "expected error for {bad:?}"
            );
        }
    }

    #[test]
    fn split_cargo_tool_entry_rejects_leading_dash() {
        // The crate half becomes a bare argument on the `cargo binstall`
        // command line; a leading '-' would make clap parse it as a flag
        // (e.g. "-h") instead of a package name.
        assert!(split_cargo_tool_entry("-h").is_err());
        assert!(split_cargo_tool_entry("--version:bin").is_err());
        assert!(split_cargo_tool_entry("crate:-bin").is_err());
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
        // Isolate the final stage (starts at "FROM debian:13-slim")
        let final_stage = content
            .find("FROM debian:13-slim")
            .map(|pos| &content[pos..])
            .expect("FROM debian:13-slim not found");
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
            cargo_tools: vec![],
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
    fn resolve_run_step_name_prefers_curated_label() {
        let sub = make_leaf("save", vec![]);
        let mut subcommands = indexmap::IndexMap::new();
        subcommands.insert(
            "save".to_string(),
            crate::orb_config::SubcommandConfig {
                label: Some("Commit back generated artifacts".to_string()),
                ..Default::default()
            },
        );
        let config = OrbConfig {
            subcommand: Some(subcommands),
            ..Default::default()
        };
        assert_eq!(
            resolve_run_step_name(&sub, Some(&config)),
            "Commit back generated artifacts"
        );
    }

    #[test]
    fn resolve_run_step_name_falls_back_to_short_about() {
        let mut sub = make_leaf("generate", vec![]);
        sub.short_about = "Generate an MCP server from an orb definition".to_string();
        assert_eq!(
            resolve_run_step_name(&sub, None),
            "Generate an MCP server from an orb definition"
        );
    }

    /// #336: step name must come from `short_about`, never `description`.
    #[test]
    fn resolve_run_step_name_uses_short_about_not_the_full_description() {
        let mut sub = make_leaf("check", vec![]);
        sub.description = "PR/dev gate: cargo-deny policy, a live cargo-audit scan, and \
            license policy. All four blocking: cargo-deny, cargo-audit, the \
            about.toml/deny.toml drift check, and the cargo-about resolution check. \
            Aggregates exit codes and surfaces stderr."
            .to_string();
        sub.short_about =
            "PR/dev gate: cargo-deny policy, a live cargo-audit scan, and license policy."
                .to_string();
        assert_eq!(
            resolve_run_step_name(&sub, None),
            "PR/dev gate: cargo-deny policy, a live cargo-audit scan, and license policy."
        );
    }

    #[test]
    fn resolve_run_step_name_falls_back_to_bare_name_when_short_about_is_empty() {
        let mut sub = make_leaf("prime", vec![]);
        sub.short_about = String::new();
        assert_eq!(resolve_run_step_name(&sub, None), "prime");
    }

    #[test]
    fn resolve_run_step_name_falls_back_to_bare_name_without_description() {
        let mut sub = make_leaf("prime", vec![]);
        sub.description = String::new();
        sub.short_about = String::new();
        assert_eq!(resolve_run_step_name(&sub, None), "prime");
    }

    #[test]
    fn curated_label_appears_as_command_run_step_name() {
        let sub = make_leaf("save", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let mut subcommands = indexmap::IndexMap::new();
        subcommands.insert(
            "save".to_string(),
            crate::orb_config::SubcommandConfig {
                label: Some("Commit back generated artifacts".to_string()),
                ..Default::default()
            },
        );
        let config = OrbConfig {
            subcommand: Some(subcommands),
            ..Default::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        let content = &files[&PathBuf::from("src/commands/save.yml")];
        assert!(
            content.contains("name: Commit back generated artifacts"),
            "curated label must be the run-step name:\n{content}"
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
            ..Default::default()
        }];
        let sub = make_leaf("generate", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let script = &files[&PathBuf::from("src/scripts/generate.sh")];
        assert!(
            script.contains("set -- \"$@\" --orb-path \"${GCO_ORB_PATH}\""),
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
            ..Default::default()
        }];
        let sub = make_leaf("generate", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let script = &files[&PathBuf::from("src/scripts/generate.sh")];
        assert!(
            script.contains("[[ -n \"${GCO_OUTPUT:-}\" ]]")
                && script.contains("--output \"${GCO_OUTPUT}\""),
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
            ..Default::default()
        }];
        let sub = make_leaf("generate", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let script = &files[&PathBuf::from("src/scripts/generate.sh")];
        assert!(
            script.contains("[[ \"${GCO_FORCE:-false}\" = \"true\" ]]")
                && script.contains("--force"),
            "boolean flag in script must use shell conditional on env var:\n{script}"
        );
    }

    fn verbose_param() -> Parameter {
        Parameter {
            long_name: "verbose".to_string(),
            short: Some('v'),
            kind: ParamKind::Long,
            param_type: ParamType::Boolean,
            description: "Increase logging verbosity".to_string(),
            repeatable: true,
            ..Default::default()
        }
    }

    fn quiet_param() -> Parameter {
        Parameter {
            long_name: "quiet".to_string(),
            short: Some('q'),
            kind: ParamKind::Long,
            param_type: ParamType::Boolean,
            description: "Decrease logging verbosity".to_string(),
            repeatable: true,
            ..Default::default()
        }
    }

    #[test]
    fn verbose_quiet_pair_merges_into_one_log_level_enum_param() {
        // #348: a subcommand's repeatable verbose/quiet pair collapses into
        // one log_level enum parameter, not two independent booleans that
        // could be set simultaneously to a self-canceling combination.
        let sub = make_leaf("release", vec![verbose_param(), quiet_param()]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let job = &files[&PathBuf::from("src/jobs/release.yml")];
        assert!(
            job.contains("log_level:") && job.contains("type: enum"),
            "merged log_level enum param must appear in generated job:\n{job}"
        );
        assert!(
            !job.contains("verbose:") && !job.contains("quiet:"),
            "verbose/quiet booleans must not appear once merged:\n{job}"
        );
        for value in ["quiet", "default", "v", "vv", "vvv", "vvvv"] {
            assert!(
                job.contains(value),
                "log_level enum must include {value:?}:\n{job}"
            );
        }
    }

    #[test]
    fn lone_repeatable_flag_without_its_pair_is_not_merged() {
        // Only verbose present, no quiet — nothing to merge into a single
        // dial, so it must be left as-is rather than guessed at.
        let sub = make_leaf("release", vec![verbose_param()]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let job = &files[&PathBuf::from("src/jobs/release.yml")];
        assert!(
            !job.contains("log_level"),
            "a lone repeatable flag without its pair must not be merged:\n{job}"
        );
        assert!(
            job.contains("verbose:"),
            "the lone verbose param must still appear:\n{job}"
        );
    }

    #[test]
    fn merge_verbosity_false_opts_a_subcommand_out_of_the_merge() {
        // A CLI whose verbose/quiet aren't clap-verbosity-flag's linked
        // counter pair can opt out of the name-based merge heuristic.
        use crate::orb_config::{OrbConfig, SubcommandConfig};

        let sub = make_leaf("release", vec![verbose_param(), quiet_param()]);
        let cli = make_cli("mytool", vec![sub]);
        let mut subcommands = IndexMap::new();
        subcommands.insert(
            "release".to_string(),
            SubcommandConfig {
                merge_verbosity: Some(false),
                ..SubcommandConfig::default()
            },
        );
        let config = OrbConfig {
            subcommand: Some(subcommands),
            ..OrbConfig::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        let job = &files[&PathBuf::from("src/jobs/release.yml")];
        assert!(
            !job.contains("log_level"),
            "merge_verbosity = false must prevent the merge:\n{job}"
        );
        assert!(
            job.contains("verbose:") && job.contains("quiet:"),
            "verbose and quiet must both still appear unmerged:\n{job}"
        );
    }

    #[test]
    fn merged_log_level_translates_to_repeated_verbose_quiet_flags_in_script() {
        let sub = make_leaf("release", vec![verbose_param(), quiet_param()]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let script = &files[&PathBuf::from("src/scripts/release.sh")];
        assert!(
            script.contains("case \"${GCO_LOG_LEVEL:-default}\" in"),
            "script must translate log_level via a case statement:\n{script}"
        );
        for (arm, flags) in [
            ("  quiet)", "--quiet"),
            ("  v)", "--verbose"),
            ("  vv)", "--verbose --verbose"),
            ("  vvv)", "--verbose --verbose --verbose"),
            ("  vvvv)", "--verbose --verbose --verbose --verbose"),
        ] {
            assert!(
                script.contains(arm) && script.contains(flags),
                "case arm {arm:?} must emit {flags:?}:\n{script}"
            );
        }
    }

    /// A positional argument carries no flag and must follow every option, in
    /// declaration order — otherwise the CLI reads an option's value as the
    /// positional (#242).
    #[test]
    fn script_passes_positional_after_the_flags() {
        let params = vec![
            Parameter {
                long_name: "version".to_string(),
                kind: ParamKind::Positional,
                param_type: ParamType::String,
                required: true,
                description: "Version to verify.".to_string(),
                ..Default::default()
            },
            Parameter {
                long_name: "advisory_db".to_string(),
                param_type: ParamType::String,
                description: "Advisory db root.".to_string(),
                ..Default::default()
            },
        ];
        let sub = make_leaf("verify", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let script = &files[&PathBuf::from("src/scripts/verify.sh")];
        let positional = script
            .find(r#"set -- "$@" "${GCO_VERSION}""#)
            .unwrap_or_else(|| panic!("positional not passed:\n{script}"));
        let flag = script
            .find("--advisory-db")
            .unwrap_or_else(|| panic!("option not passed:\n{script}"));
        assert!(
            flag < positional,
            "positional must be appended after the flags:\n{script}"
        );
        assert!(
            !script.contains("--version"),
            "a positional has no flag:\n{script}"
        );
    }

    /// gen-circleci-orb#358 redesign prerequisite: a nested subcommand's
    /// generated invocation script must run the FULL CLI path (every
    /// ancestor, not just the leaf's own bare name), or the underlying
    /// binary rejects it as an unrecognized subcommand at runtime.
    #[test]
    fn script_invokes_the_full_nested_command_path() {
        let leaf = SubCommand {
            name: "deploy".to_string(),
            description: "Deploy things.".to_string(),
            short_about: "Deploy things.".to_string(),
            is_leaf: true,
            parameters: vec![Parameter {
                long_name: "target".to_string(),
                param_type: ParamType::String,
                description: "Deploy target.".to_string(),
                ..Default::default()
            }],
            subcommands: vec![],
        };
        let group = SubCommand {
            name: "ci".to_string(),
            description: "CI-related commands.".to_string(),
            short_about: "CI-related commands.".to_string(),
            is_leaf: false,
            parameters: vec![],
            subcommands: vec![leaf],
        };
        let cli = make_cli("mytool", vec![group]);
        let files = generate(&cli, &default_opts(), None);
        let script = &files[&PathBuf::from("src/scripts/deploy.sh")];
        assert!(
            script.starts_with("set -- mytool ci deploy\n"),
            "script must invoke the full nested path 'mytool ci deploy', \
             not just the leaf's own bare name:\n{script}"
        );
    }

    /// An optional positional is only appended when a value was supplied.
    #[test]
    fn script_guards_optional_positional() {
        let params = vec![Parameter {
            long_name: "target".to_string(),
            kind: ParamKind::Positional,
            param_type: ParamType::String,
            description: "Where to write output.".to_string(),
            ..Default::default()
        }];
        let sub = make_leaf("build", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let script = &files[&PathBuf::from("src/scripts/build.sh")];
        assert!(
            script.contains(r#"[[ -n "${GCO_TARGET:-}" ]] && set -- "$@" "${GCO_TARGET}""#),
            "optional positional must be conditional:\n{script}"
        );
    }

    /// A short-only option is passed by its short flag, under the name the
    /// parameter was given (#241).
    #[test]
    fn script_passes_short_only_flag() {
        let params = vec![
            Parameter {
                long_name: "force".to_string(),
                short: Some('f'),
                kind: ParamKind::ShortOnly,
                param_type: ParamType::Boolean,
                description: "Force the operation.".to_string(),
                ..Default::default()
            },
            Parameter {
                long_name: "repeat_count".to_string(),
                short: Some('n'),
                kind: ParamKind::ShortOnly,
                param_type: ParamType::String,
                description: "How many times.".to_string(),
                ..Default::default()
            },
        ];
        let sub = make_leaf("run", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let script = &files[&PathBuf::from("src/scripts/run.sh")];
        assert!(
            script.contains(r#"[[ "${GCO_FORCE:-false}" = "true" ]] && set -- "$@" -f"#),
            "short-only boolean must be passed as -f:\n{script}"
        );
        assert!(
            script.contains(r#"set -- "$@" -n "${GCO_REPEAT_COUNT}""#),
            "short-only value option must be passed as -n <value>:\n{script}"
        );
        assert!(
            !script.contains("--force") && !script.contains("--repeat-count"),
            "a short-only option has no long form:\n{script}"
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
                ..Default::default()
            },
            Parameter {
                long_name: "force".to_string(),
                short: None,
                param_type: ParamType::Boolean,
                default: None,
                required: false,
                description: "Force.".to_string(),
                ..Default::default()
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
        // gen-circleci-orb#370: env var names are always GCO_-prefixed, never a
        // bare uppercase of the param name, so they can never collide with a
        // shell-reserved variable (PATH, HOME, IFS, ...) without needing to
        // enumerate them.
        assert!(
            content.contains("GCO_ORB_PATH: << parameters.orb_path >>"),
            "environment must map GCO_ORB_PATH:\n{content}"
        );
        assert!(
            !content.contains("\nORB_PATH:"),
            "environment must never use the bare (unprefixed) env var name:\n{content}"
        );
        // Boolean params are NOT YAML-boolean env values (unreliable at runtime);
        // they're set as strings via a `when` condition + BASH_ENV instead.
        assert!(
            !content.contains("FORCE: << parameters.force >>"),
            "boolean FORCE must not be a YAML-boolean env value:\n{content}"
        );
        assert!(
            content.contains("condition: << parameters.force >>")
                && content.contains("export GCO_FORCE=true"),
            "boolean FORCE must be gated via when + exported as a GCO_-prefixed string:\n{content}"
        );
    }

    #[test]
    fn env_var_is_prefixed_even_for_a_shell_reserved_name() {
        // gen-circleci-orb#370: a param named "path" would previously uppercase
        // to the bare env var PATH, clobbering the real PATH for the rest of
        // the script's execution. No denylist is maintained — every param's
        // env var is unconditionally GCO_-prefixed instead.
        let params = vec![Parameter {
            long_name: "path".to_string(),
            short: None,
            param_type: ParamType::String,
            default: None,
            required: true,
            description: "A path value.".to_string(),
            ..Default::default()
        }];
        let sub = make_leaf("configure", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);

        let cmd = &files[&PathBuf::from("src/commands/configure.yml")];
        assert!(
            cmd.contains("GCO_PATH: << parameters.path >>"),
            "environment must map GCO_PATH, not bare PATH:\n{cmd}"
        );
        assert!(
            !cmd.contains("\nPATH:") && !cmd.contains("  PATH:"),
            "environment must never set the bare (unprefixed, shell-reserved) PATH:\n{cmd}"
        );

        let script = &files[&PathBuf::from("src/scripts/configure.sh")];
        assert!(
            script.contains("GCO_PATH"),
            "script must reference the prefixed GCO_PATH env var:\n{script}"
        );
        assert!(
            !script.contains("\"${PATH:-}\"") && !script.contains("\"${PATH}\""),
            "script must never read/clobber the real shell PATH:\n{script}"
        );
        // The CLI flag itself is unaffected — still --path.
        assert!(
            script.contains("--path"),
            "script must still emit the original --path flag to the binary:\n{script}"
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
                ..Default::default()
            },
            Parameter {
                long_name: "optional_flag".to_string(),
                short: None,
                param_type: ParamType::Boolean,
                default: Some("false".to_string()),
                required: false,
                description: "An optional boolean.".to_string(),
                ..Default::default()
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
            ..Default::default()
        }];
        let sub = make_leaf("generate", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let script = &files[&PathBuf::from("src/scripts/generate.sh")];
        assert!(
            script.contains("set -- \"$@\" --orb-path \"${GCO_ORB_PATH}\""),
            "required param must unconditionally append via env var:\n{script}"
        );
        assert!(
            !script.contains("[ -n") || !script.contains("GCO_ORB_PATH"),
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
            ..Default::default()
        }];
        let sub = make_leaf("generate", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let script = &files[&PathBuf::from("src/scripts/generate.sh")];
        assert!(
            script.contains("[[ -n \"${GCO_OUTPUT:-}\" ]]"),
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
            ..Default::default()
        }];
        let sub = make_leaf("generate", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let script = &files[&PathBuf::from("src/scripts/generate.sh")];
        assert!(
            script.contains("[[ \"${GCO_FORCE:-false}\" = \"true\" ]]"),
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
    fn job_renames_restricted_reserved_param_instead_of_dropping_it() {
        // gen-circleci-orb#369: CircleCI reserves "name" as a job-level
        // parameter, but the invoked COMMAND already renames it to
        // "generate_name" (RESTRICTED_COMMAND_PARAMS/resolve_command_param_name)
        // rather than dropping it. The job must follow suit — declare and
        // forward the SAME renamed key — or the value has no way to reach
        // the command at all when the job is invoked from a workflow.
        let params = vec![
            Parameter {
                long_name: "name".to_string(),
                short: Some('n'),
                param_type: ParamType::String,
                default: None,
                required: false,
                description: "Name for the output.".to_string(),
                ..Default::default()
            },
            Parameter {
                long_name: "output".to_string(),
                short: Some('o'),
                param_type: ParamType::String,
                default: Some("./dist".to_string()),
                required: false,
                description: "Output dir.".to_string(),
                ..Default::default()
            },
        ];
        let sub = make_leaf("generate", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let job = &files[&PathBuf::from("src/jobs/generate.yml")];

        // Must NOT appear as the bare restricted name (CircleCI rejects it).
        assert!(
            !job.contains("\n  name:\n"),
            "job must not declare the bare restricted parameter 'name':\n{job}"
        );
        // MUST be declared under the same renamed key the command uses.
        assert!(
            job.contains("generate_name:"),
            "job must declare 'generate_name' (matching the command's own rename):\n{job}"
        );
        // MUST forward it to the command under that same renamed key, not
        // just declare it — otherwise the job still can't supply a value.
        assert!(
            job.contains("generate_name: << parameters.generate_name >>"),
            "job must forward generate_name to the command's generate_name param:\n{job}"
        );

        // Non-reserved param must still appear in the job, unchanged.
        assert!(
            job.contains("output:"),
            "job must still contain non-reserved parameter 'output':\n{job}"
        );
    }

    #[test]
    fn job_still_drops_params_reserved_only_at_job_invocation_site() {
        // Regression guard: RESERVED_JOB_PARAMS entries that are NOT also in
        // RESTRICTED_COMMAND_PARAMS (type, filters, matrix, requires,
        // context, pre_steps, post_steps) are CircleCI job-INVOCATION-site
        // keys, not something commands rename — #369's fix must not start
        // renaming these too. Combines ALL of them plus the ONE renameable
        // name ("name") in a single subcommand, so a branch-ordering bug
        // affecting only later/earlier params in the loop can't hide behind
        // testing one reserved name at a time.
        let params = vec![
            Parameter {
                long_name: "name".to_string(),
                short: None,
                param_type: ParamType::String,
                default: None,
                required: false,
                description: "Name.".to_string(),
                ..Default::default()
            },
            Parameter {
                long_name: "type".to_string(),
                short: None,
                param_type: ParamType::String,
                default: None,
                required: false,
                description: "Type.".to_string(),
                ..Default::default()
            },
            Parameter {
                long_name: "filters".to_string(),
                short: None,
                param_type: ParamType::String,
                default: None,
                required: false,
                description: "Filters.".to_string(),
                ..Default::default()
            },
            Parameter {
                long_name: "matrix".to_string(),
                short: None,
                param_type: ParamType::String,
                default: None,
                required: false,
                description: "Matrix.".to_string(),
                ..Default::default()
            },
            Parameter {
                long_name: "requires".to_string(),
                short: None,
                param_type: ParamType::String,
                default: None,
                required: false,
                description: "Requires.".to_string(),
                ..Default::default()
            },
            Parameter {
                long_name: "context".to_string(),
                short: None,
                param_type: ParamType::String,
                default: None,
                required: false,
                description: "Some context value.".to_string(),
                ..Default::default()
            },
            Parameter {
                long_name: "pre_steps".to_string(),
                short: None,
                param_type: ParamType::String,
                default: None,
                required: false,
                description: "Pre steps.".to_string(),
                ..Default::default()
            },
            Parameter {
                long_name: "post_steps".to_string(),
                short: None,
                param_type: ParamType::String,
                default: None,
                required: false,
                description: "Post steps.".to_string(),
                ..Default::default()
            },
            Parameter {
                long_name: "output".to_string(),
                short: Some('o'),
                param_type: ParamType::String,
                default: Some("./dist".to_string()),
                required: false,
                description: "Output dir.".to_string(),
                ..Default::default()
            },
        ];
        let sub = make_leaf("generate", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let job = &files[&PathBuf::from("src/jobs/generate.yml")];

        // "name" is the only renameable one — must be kept under its rename.
        assert!(
            job.contains("generate_name:"),
            "job must still rename+keep 'name':\n{job}"
        );
        // Every other reserved key stays dropped, not renamed.
        for reserved in [
            "type",
            "filters",
            "matrix",
            "requires",
            "context",
            "pre_steps",
            "post_steps",
        ] {
            assert!(
                !job.contains(&format!("\n  {reserved}:\n")),
                "job must still drop '{reserved}' (job-invocation-reserved, not renameable):\n{job}"
            );
            assert!(
                !job.contains(&format!("generate_{reserved}")),
                "job must not invent a rename for job-invocation-reserved '{reserved}':\n{job}"
            );
        }
        assert!(
            job.contains("output:"),
            "job must still contain non-reserved parameter 'output':\n{job}"
        );
    }

    #[test]
    fn job_param_override_targets_the_renamed_restricted_param() {
        // Code-review finding on #369's fix: render_job's config
        // param-default-override lookup used the raw CLI flag name
        // ("name"), but build_orb_parameters now stores a restricted+
        // renameable param under its RENAMED key ("generate_name"). An
        // override keyed by the config's bare "name" must still reach it —
        // config is written in terms of the CLI's own flag name, not the
        // internal rename.
        let params = vec![Parameter {
            long_name: "name".to_string(),
            short: Some('n'),
            param_type: ParamType::String,
            default: Some(String::new()),
            required: false,
            description: "Name for the output.".to_string(),
            ..Default::default()
        }];
        let sub = make_leaf("generate", params);
        let cli = make_cli("mytool", vec![sub]);

        let mut subcommands = IndexMap::new();
        let mut param_overrides = IndexMap::new();
        param_overrides.insert(
            "name".to_string(),
            crate::orb_config::ParamOverride {
                default: Some("my-default".to_string()),
                orb_name: None,
                workspace_sourced: None,
            },
        );
        subcommands.insert(
            "generate".to_string(),
            crate::orb_config::SubcommandConfig {
                param: Some(param_overrides),
                ..Default::default()
            },
        );
        let config = OrbConfig {
            subcommand: Some(subcommands),
            ..Default::default()
        };

        let files = generate(&cli, &default_opts(), Some(&config));
        let job = &files[&PathBuf::from("src/jobs/generate.yml")];
        assert!(
            job.contains("generate_name:") && job.contains("default: my-default"),
            "override on 'name' must apply to the renamed 'generate_name' job param:\n{job}"
        );
    }

    #[test]
    fn orb_name_override_resolves_a_renamed_key_collision() {
        // gen-circleci-orb#412: a restricted `--name` on subcommand
        // `generate` renames to `generate_name` -- which collides with an
        // unrelated, genuinely-named `--generate-name` flag on the same
        // subcommand (long_name already normalized to `generate_name`).
        // An `orb_name` override on the real flag disambiguates it, without
        // touching the restricted param's own automatic rename.
        let params = vec![
            Parameter {
                long_name: "name".to_string(),
                short: Some('n'),
                param_type: ParamType::String,
                default: Some(String::new()),
                required: false,
                description: "Name for the output.".to_string(),
                ..Default::default()
            },
            Parameter {
                long_name: "generate_name".to_string(),
                short: None,
                param_type: ParamType::Boolean,
                default: Some("false".to_string()),
                required: false,
                description: "Whether to generate a name.".to_string(),
                ..Default::default()
            },
        ];
        let sub = make_leaf("generate", params);
        let cli = make_cli("mytool", vec![sub]);

        let mut param_overrides = IndexMap::new();
        param_overrides.insert(
            "generate_name".to_string(),
            crate::orb_config::ParamOverride {
                default: None,
                orb_name: Some("generate_name_alt".to_string()),
                workspace_sourced: None,
            },
        );
        let mut subcommands = IndexMap::new();
        subcommands.insert(
            "generate".to_string(),
            crate::orb_config::SubcommandConfig {
                param: Some(param_overrides),
                ..Default::default()
            },
        );
        let config = OrbConfig {
            subcommand: Some(subcommands),
            ..Default::default()
        };

        let files = generate(&cli, &default_opts(), Some(&config));
        let command = &files[&PathBuf::from("src/commands/generate.yml")];
        let job = &files[&PathBuf::from("src/jobs/generate.yml")];
        for rendered in [command, job] {
            assert!(
                rendered.contains("generate_name:"),
                "the restricted 'name' param must still render under its \
                 automatic rename 'generate_name':\n{rendered}"
            );
            assert!(
                rendered.contains("generate_name_alt:"),
                "the genuinely-named 'generate_name' param must render under \
                 its 'orb_name' override 'generate_name_alt', not clobber \
                 'generate_name':\n{rendered}"
            );
        }
    }

    #[test]
    fn orb_name_override_on_the_restricted_param_preserves_the_real_flags_own_name() {
        // Review preference on #412: since `--name`'s own automatic rename is
        // what CREATES the collision, override the RESTRICTED param's key
        // instead of the unrelated real flag's -- one change instead of two,
        // and `--generate-name` keeps its own natural derived key untouched.
        let params = vec![
            Parameter {
                long_name: "name".to_string(),
                short: Some('n'),
                param_type: ParamType::String,
                default: Some(String::new()),
                required: false,
                description: "Name for the output.".to_string(),
                ..Default::default()
            },
            Parameter {
                long_name: "generate_name".to_string(),
                short: None,
                param_type: ParamType::Boolean,
                default: Some("false".to_string()),
                required: false,
                description: "Whether to generate a name.".to_string(),
                ..Default::default()
            },
        ];
        let sub = make_leaf("generate", params);
        let cli = make_cli("mytool", vec![sub]);

        let mut param_overrides = IndexMap::new();
        param_overrides.insert(
            "name".to_string(),
            crate::orb_config::ParamOverride {
                default: None,
                orb_name: Some("output_name".to_string()),
                workspace_sourced: None,
            },
        );
        let mut subcommands = IndexMap::new();
        subcommands.insert(
            "generate".to_string(),
            crate::orb_config::SubcommandConfig {
                param: Some(param_overrides),
                ..Default::default()
            },
        );
        let config = OrbConfig {
            subcommand: Some(subcommands),
            ..Default::default()
        };

        let files = generate(&cli, &default_opts(), Some(&config));
        let command = &files[&PathBuf::from("src/commands/generate.yml")];
        let job = &files[&PathBuf::from("src/jobs/generate.yml")];
        for rendered in [command, job] {
            assert!(
                rendered.contains("output_name:"),
                "the restricted 'name' param must render under its 'orb_name' \
                 override 'output_name':\n{rendered}"
            );
            assert!(
                rendered.contains("generate_name:"),
                "the genuinely-named 'generate_name' flag must keep its OWN \
                 natural derived key, untouched by 'name''s override:\n{rendered}"
            );
        }
    }

    #[test]
    fn orb_name_override_allows_renaming_both_colliding_params() {
        // Review follow-up on #421: the user is free to override EITHER or
        // BOTH colliding params, not steered into exactly one canonical fix
        // -- overriding both simultaneously (two changes) must work just as
        // well as overriding just one.
        let params = vec![
            Parameter {
                long_name: "name".to_string(),
                short: Some('n'),
                param_type: ParamType::String,
                default: Some(String::new()),
                required: false,
                description: "Name for the output.".to_string(),
                ..Default::default()
            },
            Parameter {
                long_name: "generate_name".to_string(),
                short: None,
                param_type: ParamType::Boolean,
                default: Some("false".to_string()),
                required: false,
                description: "Whether to generate a name.".to_string(),
                ..Default::default()
            },
        ];
        let sub = make_leaf("generate", params);
        let cli = make_cli("mytool", vec![sub]);

        let mut param_overrides = IndexMap::new();
        param_overrides.insert(
            "name".to_string(),
            crate::orb_config::ParamOverride {
                default: None,
                orb_name: Some("name_alt".to_string()),
                workspace_sourced: None,
            },
        );
        param_overrides.insert(
            "generate_name".to_string(),
            crate::orb_config::ParamOverride {
                default: None,
                orb_name: Some("generate_name_alt".to_string()),
                workspace_sourced: None,
            },
        );
        let mut subcommands = IndexMap::new();
        subcommands.insert(
            "generate".to_string(),
            crate::orb_config::SubcommandConfig {
                param: Some(param_overrides),
                ..Default::default()
            },
        );
        let config = OrbConfig {
            subcommand: Some(subcommands),
            ..Default::default()
        };

        let files = generate(&cli, &default_opts(), Some(&config));
        let command = &files[&PathBuf::from("src/commands/generate.yml")];
        let job = &files[&PathBuf::from("src/jobs/generate.yml")];
        for rendered in [command, job] {
            assert!(
                rendered.contains("name_alt:"),
                "the restricted 'name' param must render under its own \
                 override 'name_alt':\n{rendered}"
            );
            assert!(
                rendered.contains("generate_name_alt:"),
                "the genuinely-named 'generate_name' flag must ALSO render \
                 under its own override 'generate_name_alt' -- both params \
                 may be renamed at once:\n{rendered}"
            );
            assert!(
                !rendered.contains("generate_name:"),
                "neither param may keep the collision-causing bare \
                 'generate_name' key once both are overridden:\n{rendered}"
            );
        }
    }

    #[test]
    fn orb_name_override_applies_with_no_collision_present() {
        // Review follow-up on #421: `orb_name` is a general-purpose rename,
        // not gated behind "only takes effect when resolving a collision" --
        // it must apply even on a plain, non-restricted param with nothing
        // else on the subcommand for it to clash with.
        let params = vec![Parameter {
            long_name: "output".to_string(),
            short: None,
            param_type: ParamType::String,
            default: Some(String::new()),
            required: false,
            description: "Where to write the output.".to_string(),
            ..Default::default()
        }];
        let sub = make_leaf("generate", params);
        let cli = make_cli("mytool", vec![sub]);

        let mut param_overrides = IndexMap::new();
        param_overrides.insert(
            "output".to_string(),
            crate::orb_config::ParamOverride {
                default: None,
                orb_name: Some("custom_output".to_string()),
                workspace_sourced: None,
            },
        );
        let mut subcommands = IndexMap::new();
        subcommands.insert(
            "generate".to_string(),
            crate::orb_config::SubcommandConfig {
                param: Some(param_overrides),
                ..Default::default()
            },
        );
        let config = OrbConfig {
            subcommand: Some(subcommands),
            ..Default::default()
        };

        let files = generate(&cli, &default_opts(), Some(&config));
        let command = &files[&PathBuf::from("src/commands/generate.yml")];
        let job = &files[&PathBuf::from("src/jobs/generate.yml")];
        for rendered in [command, job] {
            assert!(
                rendered.contains("custom_output:"),
                "a plain, non-colliding param must still honor its own \
                 'orb_name' override:\n{rendered}"
            );
            assert!(
                !rendered.contains("\n  output:\n"),
                "the un-overridden bare 'output' key must not also appear as \
                 its own entry (substring of 'custom_output:' doesn't count):\n{rendered}"
            );
        }
    }

    #[test]
    fn orb_producing_job_gains_persist_orb_workspace() {
        // A job for an orb-producing command (one with an `orb_dir` param) must
        // gain a `persist_orb_workspace` toggle (default false) and a conditional
        // `persist_to_workspace` step for that dir, so the regenerated orb can
        // flow to downstream pack/review/push jobs via the workspace (Model B)
        // instead of relying on an immediate push.
        let params = vec![Parameter {
            long_name: "orb_dir".to_string(),
            short: None,
            param_type: ParamType::String,
            default: Some("orb".to_string()),
            required: false,
            description: "Orb output directory.".to_string(),
            ..Default::default()
        }];
        let sub = make_leaf("generate", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let job = &files[&PathBuf::from("src/jobs/generate.yml")];
        assert!(
            job.contains("persist_orb_workspace:"),
            "orb-producing job must expose persist_orb_workspace:\n{job}"
        );
        assert!(
            job.contains("persist_to_workspace:"),
            "must include a persist_to_workspace step:\n{job}"
        );
        assert!(
            job.contains("<< parameters.orb_dir >>"),
            "the persist step must reference the orb_dir parameter:\n{job}"
        );
    }

    #[test]
    fn orb_producing_job_gains_ci_wiring_check() {
        // An orb-producing job must expose a `check_ci_wiring` toggle (default
        // false — opt in on validation, never at release) and a conditional step
        // running `gen-circleci-orb update --check`. Because this lives in the orb
        // job, a consumer whose config is still on the old wiring triggers the
        // drift alert once they opt in and bump the orb version.
        let params = vec![Parameter {
            long_name: "orb_dir".to_string(),
            short: None,
            param_type: ParamType::String,
            default: Some("orb".to_string()),
            required: false,
            description: "Orb output directory.".to_string(),
            ..Default::default()
        }];
        let sub = make_leaf("generate", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let job = &files[&PathBuf::from("src/jobs/generate.yml")];
        assert!(
            job.contains("check_ci_wiring:"),
            "orb-producing job must expose check_ci_wiring:\n{job}"
        );
        assert!(
            job.contains("gen-circleci-orb update --check"),
            "must run the CI-wiring drift check:\n{job}"
        );
        assert!(
            job.contains("<< parameters.check_ci_wiring >>"),
            "the check step must be gated on the check_ci_wiring param:\n{job}"
        );
        // Default must be false: never run the wiring check unless a job opts in
        // (so it never runs at release with the ahead-of-published release binary).
        let after = &job[job.find("check_ci_wiring:").expect("check_ci_wiring param")..];
        let default_at = &after[after.find("default:").expect("param default")..];
        assert!(
            default_at.starts_with("default: false"),
            "check_ci_wiring must default to false:\n{}",
            &default_at[..default_at.len().min(20)]
        );
    }

    #[test]
    fn orb_producing_job_has_optional_ssh_fingerprint_setup() {
        // An orb-producing job must expose an optional `ssh_fingerprint` param
        // (default empty) and, gated on it, load that SSH write key + drop the
        // read-only checkout key — so the end-of-workflow push can authenticate
        // with write authority. Absent a fingerprint the step is a no-op and the
        // push falls back to the ambient environment credentials.
        let params = vec![Parameter {
            long_name: "orb_dir".to_string(),
            short: None,
            param_type: ParamType::String,
            default: Some("orb".to_string()),
            required: false,
            description: "Orb output directory.".to_string(),
            ..Default::default()
        }];
        let sub = make_leaf("generate", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let job = &files[&PathBuf::from("src/jobs/generate.yml")];
        assert!(
            job.contains("ssh_fingerprint:"),
            "orb-producing job must expose ssh_fingerprint:\n{job}"
        );
        assert!(
            job.contains("add_ssh_keys:"),
            "must conditionally add the configured SSH key:\n{job}"
        );
        assert!(
            job.contains("<< parameters.ssh_fingerprint >>"),
            "the SSH setup must be gated on / use the ssh_fingerprint param:\n{job}"
        );
        assert!(
            job.contains("ssh-add -d"),
            "must trim the read-only checkout key from the agent:\n{job}"
        );
    }

    #[test]
    fn orb_producing_job_has_optional_target_branch_switch() {
        // gen-circleci-orb#328: the relocated post-merge-regen chain runs on a
        // "pr merged" pipeline, where checkout lands on the deleted PR branch
        // and CIRCLE_BRANCH is stale. An orb-producing job must expose an
        // optional `target_branch` param (default empty — no-op) that, when
        // set, switches onto it (fetch + checkout + CIRCLE_BRANCH override)
        // right after checkout, before the generate invocation.
        let params = vec![Parameter {
            long_name: "orb_dir".to_string(),
            short: None,
            param_type: ParamType::String,
            default: Some("orb".to_string()),
            required: false,
            description: "Orb output directory.".to_string(),
            ..Default::default()
        }];
        let sub = make_leaf("generate", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let job = &files[&PathBuf::from("src/jobs/generate.yml")];
        assert!(
            job.contains("target_branch:"),
            "orb-producing job must expose target_branch:\n{job}"
        );
        assert!(
            job.contains("git fetch origin") && job.contains("git checkout"),
            "must switch onto the target branch via plain git:\n{job}"
        );
        assert!(
            job.contains("git checkout -B << parameters.target_branch >> origin/<< parameters.target_branch >>"),
            "must checkout via -B against the just-fetched remote ref, not bare `git checkout \
             <branch>` — that relies on git's remote-tracking DWIM, which can fail on a \
             shallow/single-branch clone:\n{job}"
        );
        assert!(
            job.contains("CIRCLE_BRANCH"),
            "must override CIRCLE_BRANCH so downstream tooling (e.g. record) sees the new branch:\n{job}"
        );
        assert!(
            job.contains("<< parameters.target_branch >>"),
            "the switch step must be gated on / use the target_branch param:\n{job}"
        );
        // Must run right after checkout, before the invoke step.
        let checkout_at = job.find("- checkout").expect("checkout step");
        let switch_at = job.find("target_branch >>").expect("target_branch usage");
        let invoke_at = job.find("- generate:").expect("generate invoke step");
        assert!(
            checkout_at < switch_at && switch_at < invoke_at,
            "target_branch switch must run after checkout and before generate:\n{job}"
        );
    }

    #[test]
    fn non_orb_job_has_no_target_branch_param() {
        let params = vec![Parameter {
            long_name: "output".to_string(),
            short: Some('o'),
            param_type: ParamType::String,
            default: Some(String::new()),
            required: false,
            description: "Output.".to_string(),
            ..Default::default()
        }];
        let sub = make_leaf("show", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let job = &files[&PathBuf::from("src/jobs/show.yml")];
        assert!(
            !job.contains("target_branch"),
            "non-orb job must not gain target_branch:\n{job}"
        );
    }

    #[test]
    fn non_orb_job_has_no_persist_orb_workspace() {
        // A command without an `orb_dir` param is not orb-producing and must not
        // gain the persist toggle/step (nor the ssh_fingerprint setup).
        let params = vec![Parameter {
            long_name: "output".to_string(),
            short: Some('o'),
            param_type: ParamType::String,
            default: Some(String::new()),
            required: false,
            description: "Output.".to_string(),
            ..Default::default()
        }];
        let sub = make_leaf("show", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let job = &files[&PathBuf::from("src/jobs/show.yml")];
        assert!(
            !job.contains("persist_orb_workspace"),
            "non-orb job must not gain persist_orb_workspace:\n{job}"
        );
        assert!(
            !job.contains("ssh_fingerprint"),
            "non-orb job must not gain ssh_fingerprint:\n{job}"
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
                ..Default::default()
            },
            Parameter {
                long_name: "output".to_string(),
                short: Some('o'),
                param_type: ParamType::String,
                default: Some("./dist".to_string()),
                required: false,
                description: "Output dir.".to_string(),
                ..Default::default()
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

    #[test]
    fn boolean_flag_forwarded_via_when_not_yaml_boolean_env() {
        // A boolean param must NOT be a YAML-boolean env value (CircleCI doesn't
        // reliably expose it to the shell as "true", so the flag would never be
        // passed — the bug that left consumers' no_record:true silently
        // recording). Instead a `when` condition exports it as a string to
        // BASH_ENV, which the script reads.
        let params = vec![Parameter {
            long_name: "no_record".to_string(),
            short: None,
            param_type: ParamType::Boolean,
            default: Some("false".to_string()),
            required: false,
            description: "Suppress auto-record.".to_string(),
            ..Default::default()
        }];
        let sub = make_leaf("generate", params);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let cmd = &files[&PathBuf::from("src/commands/generate.yml")];
        assert!(
            !cmd.contains("NO_RECORD: << parameters.no_record >>"),
            "boolean must not be a YAML-boolean env value:\n{cmd}"
        );
        assert!(
            cmd.contains("condition: << parameters.no_record >>"),
            "boolean flag must be gated via a when condition:\n{cmd}"
        );
        assert!(
            cmd.contains("export GCO_NO_RECORD=true"),
            "the when step must export the flag as a string to BASH_ENV:\n{cmd}"
        );
        // The script still reads the (now reliably-string) env var + emits the flag.
        let script = &files[&PathBuf::from("src/scripts/generate.sh")];
        assert!(
            script.contains(r#"[[ "${GCO_NO_RECORD:-false}" = "true" ]]"#)
                && script.contains("--no-record"),
            "script must still test GCO_NO_RECORD and emit --no-record:\n{script}"
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
    fn dockerfile_cli_installer_extracts_binary_by_name_not_strip() {
        // circleci-cli's release tarballs place the binary at the tarball
        // root, with no wrapping directory — `--strip 1` would strip the only
        // path component a top-level file has and drop it. Extracting the
        // `circleci` member by exact name works regardless of the tarball's
        // directory structure.
        let cli = make_cli("mytool", vec![]);
        let opts = GenerateOpts {
            circleci_cli_version: Some("1.0.49221".to_string()),
            ..default_opts()
        };
        let files = generate(&cli, &opts, None);
        let content = &files[&PathBuf::from("Dockerfile")];
        assert!(
            content.contains("tar -xzf \"${TARBALL}\" circleci"),
            "cli-installer must extract the circleci binary by exact member name:\n{content}"
        );
        assert!(
            !content.contains("--strip"),
            "cli-installer must not rely on --strip against the current flat tarball layout:\n{content}"
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

    /// The retry loop IS the crates.io propagation gate. Five minutes was not
    /// enough on the 0.1.4 release: the crate had published, the index had not
    /// caught up, and the release stalled half-published (#236).
    #[test]
    fn dockerfile_crate_wait_defaults_to_ten_minutes() {
        let dockerfile = render_dockerfile("mytool", &default_opts());
        assert!(
            dockerfile.contains(r#"[ "$n" -ge 40 ]"#),
            "default gate must allow 40 attempts:\n{dockerfile}"
        );
        assert!(
            dockerfile.contains("sleep 15"),
            "default gate must sleep 15s between attempts:\n{dockerfile}"
        );
        assert!(
            dockerfile.contains("crates.io index never served ${CRATE_VERSION}"),
            "the gate must still fail loudly on timeout:\n{dockerfile}"
        );
    }

    /// A consumer can widen the window without waiting for a generator release.
    #[test]
    fn dockerfile_crate_wait_is_configurable() {
        let opts = GenerateOpts {
            crate_wait: CrateWait {
                attempts: 60,
                seconds: 30,
            },
            ..default_opts()
        };
        let dockerfile = render_dockerfile("mytool", &opts);
        assert!(
            dockerfile.contains(r#"[ "$n" -ge 60 ]"#) && dockerfile.contains("sleep 30"),
            "configured attempts/interval must reach the Dockerfile:\n{dockerfile}"
        );
    }

    /// cargo caches sparse-index lookups under $CARGO_HOME. Dropping the local
    /// cache between attempts turns a conditional re-query into a full one.
    /// It has to sit inside the retry body — between the bail-out and the sleep.
    #[test]
    fn dockerfile_retry_busts_the_sparse_index_cache() {
        let dockerfile = render_dockerfile("mytool", &default_opts());
        let bust = dockerfile
            .find(r#"rm -rf "${CARGO_HOME:-/usr/local/cargo}"/registry/index/*/.cache"#)
            .unwrap_or_else(|| panic!("no index cache bust:\n{dockerfile}"));
        let fail = dockerfile
            .find("crates.io index never served")
            .expect("gate must fail loudly");
        let sleep = dockerfile.find("sleep 15").expect("gate must sleep");
        assert!(
            fail < bust && bust < sleep,
            "the cache bust belongs inside the retry body, after the bail-out and \
             before the sleep:\n{dockerfile}"
        );
    }

    /// The Dockerfile as Docker will read it: `\`-continuations joined, runs of
    /// whitespace collapsed, line structure otherwise preserved. Assertions
    /// about a *command* belong here so they survive a reflow; assertions about
    /// layout belong on the raw text.
    ///
    /// Only real continuations are joined, because an unmarked break is not one:
    /// Docker ends the instruction there and reads the next line as a new one.
    /// Leaving that newline in place is what lets an assertion tell the two
    /// apart.
    fn joined_commands(dockerfile: &str) -> String {
        dockerfile
            .replace("\\\n", " ")
            .lines()
            .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The property #262 is about, on the helper itself rather than only through
    /// the assertions that use it: a command split *without* a `\` must not read
    /// like a continued one.
    #[test]
    fn joined_commands_keeps_an_unmarked_line_break_visible() {
        let continued = "RUN foo \\\n    --bar baz\n";
        let broken = "RUN foo\n    --bar baz\n";

        assert_eq!(
            joined_commands(continued),
            "RUN foo --bar baz",
            "a real continuation is joined"
        );
        assert_eq!(
            joined_commands(broken),
            "RUN foo\n--bar baz",
            "an unmarked break stays where Docker would see it, so it cannot \
             match an assertion written for the continued form"
        );
    }

    /// The `cargo binstall` tool list as emitted, one tool per line.
    ///
    /// Reads the actual lines rather than a whitespace-flattened copy: a missing
    /// `\` continuation produces a Dockerfile that does not build, and a check
    /// that collapses newlines cannot tell the difference.
    fn binstall_tools(dockerfile: &str) -> Vec<String> {
        let mut lines = dockerfile
            .lines()
            .skip_while(|l| !l.contains("cargo binstall --no-confirm"));
        let head = lines.next().unwrap_or_default();
        assert!(
            head.trim_end().ends_with('\\'),
            "the binstall command must continue onto the tool lines:\n{head}"
        );
        let mut tools = Vec::new();
        for line in lines {
            let trimmed = line.trim();
            let cont = trimmed.ends_with('\\');
            let tool = trimmed.trim_end_matches('\\').trim();
            if tool.is_empty() {
                break;
            }
            tools.push(tool.to_string());
            if !cont {
                break;
            }
        }
        tools
    }

    /// The lines Docker reads as one `RUN` instruction — the `RUN` line and the
    /// continuations after it — each with its 1-based line number.
    ///
    /// The instruction ends at the first line not marked with a trailing `\`, so
    /// a continuation belonging to some other instruction is never picked up.
    fn run_instruction_lines(dockerfile: &str) -> Vec<(usize, &str)> {
        let mut in_run = false;
        let mut found = Vec::new();
        for (n, line) in dockerfile.lines().enumerate() {
            if line.starts_with("RUN ") {
                in_run = true;
            }
            if in_run {
                found.push((n + 1, line));
            }
            if !line.trim_end().ends_with('\\') {
                in_run = false;
            }
        }
        found
    }

    #[test]
    fn run_instruction_lines_spans_continuations_and_stops_at_the_end() {
        let dockerfile = "FROM scratch\n\
                          RUN one \\\n    two \\\n    three\nCOPY a b \\\n    c\nRUN solo\n";

        assert_eq!(
            run_instruction_lines(dockerfile),
            vec![
                (2, "RUN one \\"),
                (3, "    two \\"),
                (4, "    three"),
                (7, "RUN solo"),
            ],
            "a continued COPY is not part of a RUN, and the RUN ends at `three`"
        );
    }

    /// docker:S7020 — "Too long RUN instruction should be split into multiple
    /// lines". Scoped to `RUN`, which is what the rule covers: `COPY` and `FROM`
    /// can exceed the width without being flagged, and this repo's own
    /// digest-pinned `FROM` already does.
    ///
    /// The fixtures stress the two parts that grow with consumer config — a long
    /// binary name, and a realistic `cargo_tools` list — because a limit only
    /// holds if the test reaches it.
    #[test]
    fn every_run_instruction_line_stays_within_the_length_limit() {
        const MAX: usize = 120;
        let long_binary = "a-consumer-crate-with-a-considerably-longer-name";
        let many_tools = parsed_cargo_tools(&[
            "cargo-audit",
            "cargo-deny",
            "cargo-nextest",
            "cargo-llvm-cov",
            "cargo-msrv",
            "cargo-machete",
            "cargo-about",
            "rsign2:rsign",
        ]);

        for binary in ["mytool", long_binary] {
            for method in [
                InstallMethod::Binstall,
                InstallMethod::Local,
                InstallMethod::Apt,
            ] {
                let opts = GenerateOpts {
                    install_method: method.clone(),
                    circleci_cli_version: Some("0.1.38646".to_string()),
                    cargo_tools: if matches!(method, InstallMethod::Binstall) {
                        many_tools.clone()
                    } else {
                        vec![]
                    },
                    apt_packages: vec!["libssl-dev".to_string(), "pkg-config".to_string()],
                    ..default_opts()
                };
                let dockerfile = render_dockerfile(binary, &opts);

                for (n, line) in run_instruction_lines(&dockerfile) {
                    assert!(
                        line.chars().count() <= MAX,
                        "RUN line {n} is {} chars (limit {MAX}) for `{binary}` \
                         with {method:?}:\n{line}",
                        line.chars().count()
                    );
                }
            }
        }
    }

    /// The loop retries whatever `cargo install` failed at. A compile failure is
    /// deterministic — retrying it burns a full build per attempt and buries the
    /// compiler error N repetitions deep, so it must stop at the first one.
    #[test]
    fn dockerfile_does_not_retry_a_compile_failure() {
        let dockerfile = render_dockerfile("mytool", &default_opts());
        assert!(
            dockerfile.contains("failed to compile"),
            "the gate must recognise a build failure:\n{dockerfile}"
        );
        let detect = dockerfile.find("failed to compile").unwrap();
        let sleep = dockerfile.find("sleep 15").expect("gate must sleep");
        assert!(
            detect < sleep,
            "a build failure must bail out before the retry sleeps:\n{dockerfile}"
        );
    }

    #[test]
    fn dockerfile_binstall_builder_packages_sorted() {
        // SonarQube S7018: package lists must be sorted alphanumerically.
        let dockerfile = render_dockerfile(
            "mytool",
            &GenerateOpts {
                install_method: InstallMethod::Binstall,
                base_image: "debian:13-slim".to_string(),
                builder_image: "rust:1-slim-trixie".to_string(),
                ..default_opts()
            },
        );
        // builder stage: a self-sufficient native-build toolchain, alphabetical,
        // one package per line (S7020).
        assert!(
            dockerfile.contains(&render_apt_install(&[
                "build-essential",
                "ca-certificates",
                "clang",
                "cmake",
                "libssl-dev",
                "pkg-config",
            ])),
            "builder packages must be the sorted self-sufficient set:\n{dockerfile}"
        );
        // build the exact published dep set (Cargo.lock), not a fresh resolve.
        assert!(
            joined_commands(&dockerfile)
                .contains("cargo install mytool --locked --version \"${CRATE_VERSION}\""),
            "builder must cargo install the pinned version --locked:\n{dockerfile}"
        );
    }

    #[test]
    fn dockerfile_binstall_pins_crate_version_with_retry() {
        // #200: an unpinned `cargo install` resolves the PREVIOUS version while the
        // crates.io sparse index lags the publish API, shipping a container whose
        // binary version != its own tag. The builder must pin the exact released
        // version via a build-arg and retry until the index serves it, failing loud
        // on timeout rather than silently installing the wrong version.
        let dockerfile = render_dockerfile(
            "mytool",
            &GenerateOpts {
                install_method: InstallMethod::Binstall,
                base_image: "debian:13-slim".to_string(),
                builder_image: "rust:1-slim-trixie".to_string(),
                ..default_opts()
            },
        );
        assert!(
            dockerfile.contains("ARG CRATE_VERSION"),
            "builder must declare a CRATE_VERSION build-arg:\n{dockerfile}"
        );
        assert!(
            joined_commands(&dockerfile)
                .contains("until cargo install mytool --locked --version \"${CRATE_VERSION}\""),
            "install must be wrapped in a retry loop that waits out index lag:\n{dockerfile}"
        );
        assert!(
            dockerfile.contains("exit 1"),
            "retry loop must fail loud after a bounded number of attempts:\n{dockerfile}"
        );
        // The ARG must precede the RUN that consumes it.
        let arg_pos = dockerfile
            .find("ARG CRATE_VERSION")
            .expect("ARG CRATE_VERSION missing");
        let run_pos = dockerfile
            .find("RUN apt-get update")
            .expect("builder RUN missing");
        assert!(
            arg_pos < run_pos,
            "ARG CRATE_VERSION must precede the builder RUN:\n{dockerfile}"
        );
    }

    #[test]
    fn dockerfile_apt_install_is_one_package_per_line() {
        // docker:S7020 — a single long `apt-get install` line trips the length
        // limit. Every apt package must be on its own `\`-continued line.
        let dockerfile = render_dockerfile(
            "mytool",
            &GenerateOpts {
                install_method: InstallMethod::Binstall,
                base_image: "debian:13-slim".to_string(),
                builder_image: "rust:1-slim-trixie".to_string(),
                apt_packages: ["extra-pkg".to_string()].to_vec(),
                ..default_opts()
            },
        );
        // builder fixed toolchain: each package on its own continued line.
        for pkg in [
            "build-essential",
            "ca-certificates",
            "clang",
            "cmake",
            "libssl-dev",
            "pkg-config",
        ] {
            assert!(
                dockerfile.contains(&format!("    {pkg} \\\n")),
                "builder package `{pkg}` must be on its own continued line:\n{dockerfile}"
            );
        }
        // runtime stage extra package too.
        assert!(
            dockerfile.contains("    extra-pkg \\\n"),
            "runtime package must be on its own continued line:\n{dockerfile}"
        );
        // the multi-package list must NOT appear space-joined on one line.
        assert!(
            !dockerfile
                .contains("build-essential ca-certificates clang cmake libssl-dev pkg-config"),
            "packages must not be emitted on a single line:\n{dockerfile}"
        );
        // the install directive opens the multi-line list.
        assert!(
            dockerfile
                .contains("&& apt-get install -y --no-install-recommends \\\n    build-essential"),
            "apt-get install must open a multi-line list:\n{dockerfile}"
        );
    }

    #[test]
    fn dockerfile_apt_install_multiline_covers_local_and_apt() {
        // The one-per-line form (S7020) applies to the Local runtime stage and the
        // single-stage Apt image, not just Binstall.
        let local = render_dockerfile(
            "mytool",
            &GenerateOpts {
                install_method: InstallMethod::Local,
                base_image: "debian:13-slim".to_string(),
                builder_image: "rust:1-slim-trixie".to_string(),
                apt_packages: ["libpq-dev".to_string()].to_vec(),
                ..default_opts()
            },
        );
        assert!(
            local.contains("    libpq-dev \\\n") && local.contains("    ca-certificates \\\n"),
            "Local runtime apt list must be one package per line:\n{local}"
        );
        let apt = render_dockerfile(
            "mytool",
            &GenerateOpts {
                install_method: InstallMethod::Apt,
                base_image: "debian:13-slim".to_string(),
                builder_image: "rust:1-slim-trixie".to_string(),
                ..default_opts()
            },
        );
        assert!(
            apt.contains("    git \\\n") && apt.contains("    mytool \\\n"),
            "Apt image package list must be one package per line:\n{apt}"
        );
    }

    #[test]
    fn dockerfile_builder_stage_uses_configured_builder_image() {
        // A pinned `…@sha256:…` builder image from config must be emitted verbatim
        // on the builder FROM, so the digest survives regeneration (option 1 — the
        // generator no longer hardcodes `rust:1-slim-trixie`).
        let dockerfile = render_dockerfile(
            "mytool",
            &GenerateOpts {
                install_method: InstallMethod::Binstall,
                base_image: "debian:13-slim".to_string(),
                builder_image: "rust:1-slim-trixie@sha256:deadbeef".to_string(),
                ..default_opts()
            },
        );
        assert!(
            dockerfile.contains("FROM rust:1-slim-trixie@sha256:deadbeef AS builder"),
            "builder stage must use the configured builder_image (incl. digest):\n{dockerfile}"
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
        let dockerfile = render_dockerfile(
            "mytool",
            &GenerateOpts {
                install_method: InstallMethod::Local,
                base_image: "debian:13-slim".to_string(),
                builder_image: "rust:1-slim-trixie".to_string(),
                ..default_opts()
            },
        );
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
        let dockerfile = render_dockerfile(
            "mytool",
            &GenerateOpts {
                install_method: InstallMethod::Local,
                base_image: "debian:13-slim".to_string(),
                builder_image: "rust:1-slim-trixie".to_string(),
                ..default_opts()
            },
        );
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
        let dockerfile = render_dockerfile(
            "mytool",
            &GenerateOpts {
                install_method: InstallMethod::Local,
                base_image: "debian:13-slim".to_string(),
                builder_image: "rust:1-slim-trixie".to_string(),
                ..default_opts()
            },
        );
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
        let dockerfile = render_dockerfile(
            "mytool",
            &GenerateOpts {
                install_method: InstallMethod::Local,
                base_image: "debian:13-slim".to_string(),
                builder_image: "rust:1-slim-trixie".to_string(),
                ..default_opts()
            },
        );
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
        let dockerfile = render_dockerfile(
            "mytool",
            &GenerateOpts {
                install_method: InstallMethod::Local,
                base_image: "debian:13-slim".to_string(),
                builder_image: "rust:1-slim-trixie".to_string(),
                ..default_opts()
            },
        );
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
            &GenerateOpts {
                install_method: InstallMethod::Local,
                base_image: "debian:13-slim".to_string(),
                builder_image: "rust:1-slim-trixie".to_string(),
                circleci_cli_version: Some("0.1.36202".to_string()),
                ..default_opts()
            },
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
        // The export MUST be appended to $BASH_ENV. A bare `export PATH=...` only
        // affects its own step's shell, so the workspace binary would not be on
        // PATH for the subsequent generate step (regressed in 0.0.48 — the orb
        // failed to find the attached binary with "No such file or directory").
        assert!(
            script.contains("$BASH_ENV"),
            "add-workspace-to-path.sh must append the export to $BASH_ENV so PATH \
             persists to later steps:\n{script}"
        );
    }

    // ── workspace-sourced job parameters ─────────────────────────────────────

    /// A subcommand with one required positional `version` param, plus an
    /// `OrbConfig` flagging it `workspace_sourced = true` under
    /// `[subcommand.release-prep.param.version]` — the exact shape
    /// jci-audit's `release-prep`/`publish-record` need.
    fn workspace_sourced_fixture() -> (CliDefinition, OrbConfig) {
        let params = vec![Parameter {
            long_name: "version".to_string(),
            short: None,
            kind: ParamKind::Positional,
            param_type: ParamType::String,
            default: None,
            required: true,
            description: "The release version being validated.".to_string(),
            ..Default::default()
        }];
        let sub = make_leaf("release-prep", params);
        let cli = make_cli("mytool", vec![sub]);

        let mut param_overrides = IndexMap::new();
        param_overrides.insert(
            "version".to_string(),
            crate::orb_config::ParamOverride {
                default: None,
                orb_name: None,
                workspace_sourced: Some(true),
            },
        );
        let mut subcommands = IndexMap::new();
        subcommands.insert(
            "release-prep".to_string(),
            crate::orb_config::SubcommandConfig {
                param: Some(param_overrides),
                ..Default::default()
            },
        );
        let config = OrbConfig {
            subcommand: Some(subcommands),
            ..Default::default()
        };
        (cli, config)
    }

    #[test]
    fn workspace_sourced_param_adds_env_var_job_parameter() {
        let (cli, config) = workspace_sourced_fixture();
        let files = generate(&cli, &default_opts(), Some(&config));
        let job = &files[&PathBuf::from("src/jobs/release_prep.yml")];
        assert!(
            job.contains("version_env_var:"),
            "a workspace_sourced param must gain a '<key>_env_var' job parameter:\n{job}"
        );
    }

    #[test]
    fn workspace_sourced_param_adds_source_file_job_parameter() {
        let (cli, config) = workspace_sourced_fixture();
        let files = generate(&cli, &default_opts(), Some(&config));
        let job = &files[&PathBuf::from("src/jobs/release_prep.yml")];
        assert!(
            job.contains("version_source_file:"),
            "a workspace_sourced param must gain a '<key>_source_file' job parameter:\n{job}"
        );
    }

    #[test]
    fn workspace_sourced_param_env_var_and_source_file_default_to_empty() {
        let (cli, config) = workspace_sourced_fixture();
        let files = generate(&cli, &default_opts(), Some(&config));
        let job = &files[&PathBuf::from("src/jobs/release_prep.yml")];
        // Empty default means the feature is fully opt-in per invocation --
        // omitting both keeps the job byte-for-byte behaviourally identical
        // to a literal-only `version` consumer.
        let env_var_pos = job.find("version_env_var:").unwrap();
        let env_var_block = &job[env_var_pos..];
        assert!(
            env_var_block.contains("default: ''"),
            "version_env_var must default to empty:\n{job}"
        );
    }

    #[test]
    fn workspace_sourced_resolve_step_is_conditional_on_env_var_param() {
        let (cli, config) = workspace_sourced_fixture();
        let files = generate(&cli, &default_opts(), Some(&config));
        let job = &files[&PathBuf::from("src/jobs/release_prep.yml")];
        assert!(
            job.contains("condition: << parameters.version_env_var >>"),
            "the resolve step must be gated on version_env_var being non-empty:\n{job}"
        );
    }

    #[test]
    fn workspace_sourced_resolve_step_appears_after_attach_workspace_step() {
        let (cli, config) = workspace_sourced_fixture();
        let files = generate(&cli, &default_opts(), Some(&config));
        let job = &files[&PathBuf::from("src/jobs/release_prep.yml")];
        let attach_pos = job
            .find("condition: << parameters.attach_workspace >>")
            .expect("attach_workspace step missing");
        let resolve_pos = job
            .find("condition: << parameters.version_env_var >>")
            .expect("resolve step missing");
        let invoke_pos = job.find("release_prep:").expect("invoke step missing");
        assert!(
            attach_pos < resolve_pos && resolve_pos < invoke_pos,
            "resolve step must run after attach_workspace and before the invoke step:\n{job}"
        );
    }

    #[test]
    fn resolve_workspace_param_script_always_generated_when_used() {
        let (cli, config) = workspace_sourced_fixture();
        let files = generate(&cli, &default_opts(), Some(&config));
        assert!(
            files.contains_key(&PathBuf::from("src/scripts/resolve_workspace_param.sh")),
            "resolve_workspace_param.sh must be generated when any param is workspace_sourced"
        );
    }

    #[test]
    fn resolve_workspace_param_script_sources_file_and_uses_bash_env() {
        let (cli, config) = workspace_sourced_fixture();
        let files = generate(&cli, &default_opts(), Some(&config));
        let script = &files[&PathBuf::from("src/scripts/resolve_workspace_param.sh")];
        assert!(
            script.contains("GCO_TARGET_ENV_VAR"),
            "script must read which variable to extract:\n{script}"
        );
        assert!(
            script.contains("GCO_SOURCE_FILE"),
            "script must read which file to source:\n{script}"
        );
        assert!(
            script.contains("$BASH_ENV"),
            "resolved value must be exported via $BASH_ENV to persist to later steps:\n{script}"
        );
    }

    #[test]
    fn resolve_workspace_param_script_never_dumps_the_source_files_contents() {
        // `version_source_file` is documented as overridable to point at any
        // producer's own env-style output file -- possibly a shared file
        // carrying other, unrelated values alongside the one being
        // extracted. Printing the whole file to the (potentially more
        // widely readable) CI log on a resolution failure would leak
        // whatever else is in it; the error message already names the
        // missing variable and the file path, which is enough to debug
        // without echoing arbitrary file contents.
        let (cli, config) = workspace_sourced_fixture();
        let files = generate(&cli, &default_opts(), Some(&config));
        let script = &files[&PathBuf::from("src/scripts/resolve_workspace_param.sh")];
        assert!(
            !script.contains("cat \"${SOURCE_FILE}\""),
            "the script must not dump the source file's contents to the CI log:\n{script}"
        );
    }

    #[test]
    fn command_script_falls_back_to_resolved_env_var_when_literal_is_empty() {
        let (cli, config) = workspace_sourced_fixture();
        let files = generate(&cli, &default_opts(), Some(&config));
        let script = &files[&PathBuf::from("src/scripts/release_prep.sh")];
        assert!(
            script.contains("GCO_VERSION_RESOLVED"),
            "script must fall back to the workspace-resolved value when the literal \
             GCO_VERSION is empty:\n{script}"
        );
    }

    #[test]
    fn command_script_errors_when_neither_literal_nor_resolved_is_set() {
        let (cli, config) = workspace_sourced_fixture();
        let files = generate(&cli, &default_opts(), Some(&config));
        let script = &files[&PathBuf::from("src/scripts/release_prep.sh")];
        assert!(
            script.contains("exit 1"),
            "script must fail loudly when neither the literal nor the resolved value \
             is available:\n{script}"
        );
    }

    #[test]
    fn workspace_sourced_blank_check_treats_whitespace_only_value_as_absent() {
        // A `-z` check on the literal only rejects a genuinely zero-length
        // string -- a whitespace-only value (e.g. an upstream template
        // accidentally passing `version: " "`) would slip through as
        // "present" and get forwarded to the CLI as a literal argument,
        // producing a confusing downstream parse failure instead of this
        // script's own clear error. Both the literal-vs-resolved fallback
        // decision and the final required/optional check must treat
        // whitespace-only the same as empty.
        let (cli, config) = workspace_sourced_fixture();
        let files = generate(&cli, &default_opts(), Some(&config));
        let script = &files[&PathBuf::from("src/scripts/release_prep.sh")];
        assert!(
            script.contains("[[:space:]]"),
            "the blank check must treat a whitespace-only value as absent, not just a \
             zero-length string:\n{script}"
        );
        assert!(
            !script.contains("[[ -z \"${GCO_VERSION_VALUE}\" ]]"),
            "the final required-param check must use the whitespace-aware blank check, \
             not a bare -z:\n{script}"
        );
    }

    #[test]
    fn non_workspace_sourced_subcommand_job_is_unaffected() {
        // Regression guard: a subcommand with NO param flagged workspace_sourced
        // must not gain any of the new job parameters or steps -- the
        // overwhelming common case stays byte-for-byte unchanged.
        let sub = make_leaf("generate", vec![]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let job = &files[&PathBuf::from("src/jobs/generate.yml")];
        assert!(
            !job.contains("_env_var:") && !job.contains("_source_file:"),
            "a subcommand with no workspace_sourced param must not gain these parameters:\n{job}"
        );
        assert!(
            !files.contains_key(&PathBuf::from("src/scripts/resolve_workspace_param.sh")),
            "resolve_workspace_param.sh must not be generated when nothing uses it"
        );
    }

    #[test]
    fn workspace_sourced_required_param_job_parameter_gets_an_empty_default() {
        // A required CLI param (release-prep's positional `version`)
        // otherwise gets no `default:` at all, which makes CircleCI require
        // it at job-invocation time too -- defeating the whole point of the
        // runtime-resolution fallback, since a consumer relying on
        // version_env_var could never omit the literal `version` parameter.
        let (cli, config) = workspace_sourced_fixture();
        let files = generate(&cli, &default_opts(), Some(&config));
        let job = &files[&PathBuf::from("src/jobs/release_prep.yml")];
        let version_pos = job.find("  version:").expect("version param missing");
        let version_block = &job[version_pos..version_pos + 120];
        assert!(
            version_block.contains("default: ''"),
            "a workspace_sourced required param's own job parameter must gain an \
             empty default so it's no longer mandatory at invocation:\n{version_block}"
        );
    }

    /// A subcommand with one OPTIONAL (`required: false`) param flagged
    /// `workspace_sourced = true` — the `version` fixture is required, so
    /// this covers the other branch of `render_command_script_content`'s
    /// fallback codegen.
    fn workspace_sourced_optional_fixture() -> (CliDefinition, OrbConfig) {
        let params = vec![Parameter {
            long_name: "tag".to_string(),
            short: None,
            kind: ParamKind::Long,
            param_type: ParamType::String,
            default: Some(String::new()),
            required: false,
            description: "Optional release tag.".to_string(),
            ..Default::default()
        }];
        let sub = make_leaf("publish-record", params);
        let cli = make_cli("mytool", vec![sub]);

        let mut param_overrides = IndexMap::new();
        param_overrides.insert(
            "tag".to_string(),
            crate::orb_config::ParamOverride {
                default: None,
                orb_name: None,
                workspace_sourced: Some(true),
            },
        );
        let mut subcommands = IndexMap::new();
        subcommands.insert(
            "publish-record".to_string(),
            crate::orb_config::SubcommandConfig {
                param: Some(param_overrides),
                ..Default::default()
            },
        );
        let config = OrbConfig {
            subcommand: Some(subcommands),
            ..Default::default()
        };
        (cli, config)
    }

    #[test]
    fn workspace_sourced_optional_param_omits_flag_silently_when_unset() {
        // Regression guard: workspace_sourced must not turn an OPTIONAL
        // param mandatory. Unlike the required `version` fixture (which
        // errors loudly when neither value is available), an optional param
        // left unset by both means must simply omit the flag, exactly like
        // its pre-existing (non-workspace_sourced) optional behavior.
        let (cli, config) = workspace_sourced_optional_fixture();
        let files = generate(&cli, &default_opts(), Some(&config));
        let script = &files[&PathBuf::from("src/scripts/publish_record.sh")];
        assert!(
            !script.contains("exit 1"),
            "an optional workspace_sourced param must not hard-error when unset:\n{script}"
        );
        assert!(
            script
                .contains("[[ ! \"${GCO_TAG_VALUE}\" =~ ^[[:space:]]*$ ]] && set -- \"$@\" --tag"),
            "an optional workspace_sourced param must conditionally omit its flag, \
             matching its pre-existing optional behavior:\n{script}"
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
                ..SubcommandConfig::default()
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
                ..SubcommandConfig::default()
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

    // ── interactive (CLI-only) subcommands: full exclusion ────────────────────

    fn config_with_subcommand(name: &str, sc: crate::orb_config::SubcommandConfig) -> OrbConfig {
        use indexmap::IndexMap;
        let mut subcommands = IndexMap::new();
        subcommands.insert(name.to_string(), sc);
        OrbConfig {
            subcommand: Some(subcommands),
            ..OrbConfig::default()
        }
    }

    #[test]
    fn interactive_true_excludes_job_command_and_script() {
        use crate::orb_config::SubcommandConfig;
        let cli = make_cli("mytool", vec![make_leaf("setup", vec![])]);
        let config = config_with_subcommand(
            "setup",
            SubcommandConfig {
                interactive: Some(true),
                ..SubcommandConfig::default()
            },
        );
        let files = generate(&cli, &default_opts(), Some(&config));
        assert!(
            !files.contains_key(&PathBuf::from("src/jobs/setup.yml")),
            "no job"
        );
        assert!(
            !files.contains_key(&PathBuf::from("src/commands/setup.yml")),
            "no command"
        );
        assert!(
            !files.contains_key(&PathBuf::from("src/scripts/setup.sh")),
            "no script"
        );
    }

    #[test]
    fn interactive_true_for_init_and_false_for_config() {
        // The two default-interactive names, controlled independently: init keeps
        // the default (reserved → excluded); config is opted back into CI with
        // interactive = false (generated).
        use crate::orb_config::SubcommandConfig;
        let cli = make_cli(
            "mytool",
            vec![make_leaf("init", vec![]), make_leaf("config", vec![])],
        );
        let config = config_with_subcommand(
            "config",
            SubcommandConfig {
                interactive: Some(false),
                ..SubcommandConfig::default()
            },
        );
        let files = generate(&cli, &default_opts(), Some(&config));
        // init — default interactive → fully excluded
        assert!(
            !files.contains_key(&PathBuf::from("src/commands/init.yml")),
            "init defaults to interactive → excluded"
        );
        // config — interactive = false → generated (command + job)
        assert!(
            files.contains_key(&PathBuf::from("src/commands/config.yml")),
            "config interactive=false → command generated"
        );
        assert!(
            files.contains_key(&PathBuf::from("src/jobs/config.yml")),
            "config interactive=false → job generated"
        );
    }

    #[test]
    fn interactive_on_parent_excludes_subtree() {
        use crate::orb_config::SubcommandConfig;
        let child = make_leaf("show", vec![]);
        let parent = SubCommand {
            name: "admin".to_string(),
            description: "Admin tools".to_string(),
            short_about: "Admin tools".to_string(),
            is_leaf: false,
            parameters: vec![],
            subcommands: vec![child],
        };
        let cli = make_cli("mytool", vec![parent]);
        let config = config_with_subcommand(
            "admin",
            SubcommandConfig {
                interactive: Some(true),
                ..SubcommandConfig::default()
            },
        );
        let files = generate(&cli, &default_opts(), Some(&config));
        assert!(
            !files.contains_key(&PathBuf::from("src/commands/show.yml")),
            "subtree of an interactive parent must be excluded"
        );
    }

    #[test]
    fn interactive_takes_precedence_over_generate_job() {
        use crate::orb_config::SubcommandConfig;
        let cli = make_cli("mytool", vec![make_leaf("setup", vec![])]);
        let config = config_with_subcommand(
            "setup",
            SubcommandConfig {
                interactive: Some(true),
                generate_job: Some(true),
                ..SubcommandConfig::default()
            },
        );
        let files = generate(&cli, &default_opts(), Some(&config));
        assert!(
            !files.contains_key(&PathBuf::from("src/jobs/setup.yml")),
            "interactive=true wins over generate_job=true (no job)"
        );
        assert!(
            !files.contains_key(&PathBuf::from("src/commands/setup.yml")),
            "and no command either"
        );
    }

    #[test]
    fn orb_name_override_keyed_by_the_qualified_name_applies_to_that_leaf_only() {
        // gen-circleci-orb#425: root `release` and nested `ci release` both
        // have `--name`. An override under `[subcommand.ci_release]` must
        // rename only the nested leaf's key (command, script env var and job
        // alike); the root keeps its automatic `release_name`.
        let name_param = || {
            let mut p = make_param("name", None, false);
            p.description = "Name.".to_string();
            p
        };
        let nested = make_leaf("release", vec![name_param()]);
        let cli = CliDefinition {
            binary_name: "demo".to_string(),
            description: String::new(),
            subcommands: vec![
                make_leaf("release", vec![name_param()]),
                SubCommand {
                    name: "ci".to_string(),
                    description: String::new(),
                    short_about: String::new(),
                    is_leaf: false,
                    parameters: vec![],
                    subcommands: vec![nested],
                },
            ],
        };
        let mut overrides = IndexMap::new();
        overrides.insert(
            "name".to_string(),
            crate::orb_config::ParamOverride {
                default: None,
                orb_name: Some("custom_name".to_string()),
                workspace_sourced: None,
            },
        );
        let mut subcommands = IndexMap::new();
        subcommands.insert(
            "ci_release".to_string(),
            crate::orb_config::SubcommandConfig {
                param: Some(overrides),
                ..Default::default()
            },
        );
        let config = OrbConfig {
            subcommand: Some(subcommands),
            ..Default::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        let file = |p: &str| files[&PathBuf::from(p)].as_str();

        for path in [
            "src/commands/ci_release.yml",
            "src/jobs/ci_release.yml",
            "src/scripts/ci_release.sh",
        ] {
            assert!(
                file(path).contains("custom_name") || file(path).contains("CUSTOM_NAME"),
                "{path} must use the overridden key:\n{}",
                file(path)
            );
        }
        for path in [
            "src/commands/release.yml",
            "src/jobs/release.yml",
            "src/scripts/release.sh",
        ] {
            assert!(
                !file(path).contains("custom_name") && !file(path).contains("CUSTOM_NAME"),
                "{path} must not pick up the nested leaf's override:\n{}",
                file(path)
            );
            assert!(
                file(path).contains("release_name") || file(path).contains("RELEASE_NAME"),
                "{path} must keep the automatic restricted rename:\n{}",
                file(path)
            );
        }
    }

    #[test]
    fn job_group_uses_the_effective_name_for_a_colliding_step() {
        // gen-circleci-orb#425 review: `ci` is listed first, so the group's
        // bare step "release" resolves to `ci release` (effective name
        // `ci_release`; the root `release` keeps the bare name). Its
        // override lives under `[subcommand.ci_release]`, and the invoke
        // step must call the `ci_release` command declaring that same key.
        let name_param = || {
            let mut p = make_param("name", None, true);
            p.description = "Name.".to_string();
            p
        };
        let cli = CliDefinition {
            binary_name: "demo".to_string(),
            description: String::new(),
            subcommands: vec![
                SubCommand {
                    name: "ci".to_string(),
                    description: String::new(),
                    short_about: String::new(),
                    is_leaf: false,
                    parameters: vec![],
                    subcommands: vec![make_leaf("release", vec![name_param()])],
                },
                make_leaf("release", vec![name_param()]),
            ],
        };
        let mut overrides = IndexMap::new();
        overrides.insert(
            "name".to_string(),
            crate::orb_config::ParamOverride {
                default: None,
                orb_name: Some("custom_name".to_string()),
                workspace_sourced: None,
            },
        );
        let mut subcommands = IndexMap::new();
        subcommands.insert(
            "ci_release".to_string(),
            crate::orb_config::SubcommandConfig {
                param: Some(overrides),
                ..Default::default()
            },
        );
        let config = OrbConfig {
            subcommand: Some(subcommands),
            job_group: Some(vec![crate::orb_config::JobGroup {
                name: "sync".to_string(),
                description: None,
                steps: vec!["release".to_string()],
                params: None,
                ..Default::default()
            }]),
            ..Default::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        let job = &files[&PathBuf::from("src/jobs/sync.yml")];
        // The step's own parameter, under the overridden key its command
        // declares.
        assert!(
            job.contains("ci_release:\n    custom_name: << parameters.custom_name >>"),
            "the invoke step must call the `ci_release` command with its \
             declared (overridden) key:\n{job}"
        );
    }

    // ── compute_effective_names (#358) ──────────────────────────────────────

    #[test]
    fn compute_effective_names_qualifies_a_colliding_nested_leaf() {
        // gen-circleci-orb#358: a top-level "release" and a nested
        // "ci.release" collide. Rather than rejecting, the root occurrence
        // keeps its bare name (its own path IS "release" already) and the
        // nested one is qualified by its full underscore-joined path.
        let nested = SubCommand {
            name: "release".to_string(),
            description: String::new(),
            short_about: String::new(),
            is_leaf: true,
            parameters: vec![],
            subcommands: vec![],
        };
        let cli = CliDefinition {
            binary_name: "demo".to_string(),
            description: String::new(),
            subcommands: vec![
                make_leaf("release", vec![]),
                SubCommand {
                    name: "ci".to_string(),
                    description: String::new(),
                    short_about: String::new(),
                    is_leaf: false,
                    parameters: vec![],
                    subcommands: vec![nested],
                },
            ],
        };
        let effective = compute_effective_names(&cli, None);
        assert_eq!(
            effective.get("release").map(String::as_str),
            Some("release")
        );
        assert_eq!(
            effective.get("ci.release").map(String::as_str),
            Some("ci_release")
        );
    }

    #[test]
    fn compute_effective_names_leaves_distinct_names_bare() {
        let nested = SubCommand {
            name: "deploy".to_string(),
            description: String::new(),
            short_about: String::new(),
            is_leaf: true,
            parameters: vec![],
            subcommands: vec![],
        };
        let cli = CliDefinition {
            binary_name: "demo".to_string(),
            description: String::new(),
            subcommands: vec![
                make_leaf("release", vec![]),
                SubCommand {
                    name: "ci".to_string(),
                    description: String::new(),
                    short_about: String::new(),
                    is_leaf: false,
                    parameters: vec![],
                    subcommands: vec![nested],
                },
            ],
        };
        let effective = compute_effective_names(&cli, None);
        assert_eq!(
            effective.get("release").map(String::as_str),
            Some("release")
        );
        assert_eq!(
            effective.get("ci.deploy").map(String::as_str),
            Some("deploy")
        );
    }

    #[test]
    fn compute_effective_names_ignores_a_non_leaf_group_collision() {
        // Only LEAF subcommands ever write a generated file — a non-leaf
        // group's own name can never clobber anything, so it's absent from
        // the map (it's never looked up, since render_subcommand only
        // consults this map for leaves).
        let cli = CliDefinition {
            binary_name: "demo".to_string(),
            description: String::new(),
            subcommands: vec![
                SubCommand {
                    name: "db".to_string(),
                    description: String::new(),
                    short_about: String::new(),
                    is_leaf: false,
                    parameters: vec![],
                    subcommands: vec![make_leaf("migrate", vec![])],
                },
                SubCommand {
                    name: "other".to_string(),
                    description: String::new(),
                    short_about: String::new(),
                    is_leaf: false,
                    parameters: vec![],
                    subcommands: vec![make_leaf("db", vec![])],
                },
            ],
        };
        let effective = compute_effective_names(&cli, None);
        assert_eq!(
            effective.get("other.db").map(String::as_str),
            Some("db"),
            "the only LEAF named 'db' has no real collision to qualify against"
        );
    }

    #[test]
    fn compute_effective_names_ignores_an_interactive_excluded_collision() {
        // is_interactive gates by BARE name (not path), matching
        // render_subcommand's own gate exactly — so EVERY occurrence of an
        // interactive-reserved name (e.g. "init", DEFAULT_INTERACTIVE) is
        // excluded uniformly, regardless of nesting. Confirms the collision
        // walk correctly skips both occurrences (and their subtrees)
        // entirely, rather than reporting a false collision between two
        // subcommands that render_subcommand would never actually emit.
        let cli = CliDefinition {
            binary_name: "demo".to_string(),
            description: String::new(),
            subcommands: vec![
                make_leaf("init", vec![]), // interactive by default
                SubCommand {
                    name: "db".to_string(),
                    description: String::new(),
                    short_about: String::new(),
                    is_leaf: false,
                    parameters: vec![],
                    subcommands: vec![make_leaf("init", vec![])],
                },
            ],
        };
        let effective = compute_effective_names(&cli, None);
        assert_eq!(
            effective.get("init"),
            None,
            "the top-level 'init' is interactive-excluded, not a candidate at all"
        );
        assert_eq!(
            effective.get("db.init"),
            None,
            "the nested 'db init' is ALSO interactive-excluded (bare-name gate applies \
             to every occurrence, not just the first)"
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
                ..SubcommandConfig::default()
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
            ..Default::default()
        };
        let sub = make_leaf("generate", vec![orb_path_param]);
        let cli = make_cli("mytool", vec![sub]);

        let mut param_overrides = IndexMap::new();
        param_overrides.insert(
            "orb_path".to_string(),
            ParamOverride {
                default: Some("custom/@orb.yml".to_string()),
                orb_name: None,
                workspace_sourced: None,
            },
        );
        let mut subcommands = IndexMap::new();
        subcommands.insert(
            "generate".to_string(),
            SubcommandConfig {
                param: Some(param_overrides),
                ..SubcommandConfig::default()
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
    fn param_override_coerces_boolean_default_not_a_quoted_string() {
        use crate::help_parser::types::Parameter;
        use crate::orb_config::{OrbConfig, ParamOverride, SubcommandConfig};
        use indexmap::IndexMap;

        let check_param = Parameter {
            long_name: "check".to_string(),
            short: None,
            param_type: ParamType::Boolean,
            default: Some("false".to_string()),
            required: false,
            description: "Check only.".to_string(),
            ..Default::default()
        };
        let sub = make_leaf("wire_ci", vec![check_param]);
        let cli = make_cli("mytool", vec![sub]);

        let mut param_overrides = IndexMap::new();
        param_overrides.insert(
            "check".to_string(),
            ParamOverride {
                default: Some("true".to_string()),
                orb_name: None,
                workspace_sourced: None,
            },
        );
        let mut subcommands = IndexMap::new();
        subcommands.insert(
            "wire_ci".to_string(),
            SubcommandConfig {
                param: Some(param_overrides),
                ..SubcommandConfig::default()
            },
        );
        let config = OrbConfig {
            subcommand: Some(subcommands),
            ..OrbConfig::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        let job = &files[&PathBuf::from("src/jobs/wire_ci.yml")];
        assert!(
            job.contains("default: true"),
            "boolean override must render as an unquoted YAML boolean:\n{job}"
        );
        assert!(
            !job.contains("default: \"true\""),
            "boolean override must not render as a quoted string:\n{job}"
        );
    }

    #[test]
    fn param_override_coerces_integer_default_not_a_quoted_string() {
        use crate::help_parser::types::Parameter;
        use crate::orb_config::{OrbConfig, ParamOverride, SubcommandConfig};
        use indexmap::IndexMap;

        let retries_param = Parameter {
            long_name: "retries".to_string(),
            short: None,
            param_type: ParamType::Integer,
            default: Some("1".to_string()),
            required: false,
            description: "Retry count.".to_string(),
            ..Default::default()
        };
        let sub = make_leaf("publish", vec![retries_param]);
        let cli = make_cli("mytool", vec![sub]);

        let mut param_overrides = IndexMap::new();
        param_overrides.insert(
            "retries".to_string(),
            ParamOverride {
                default: Some("3".to_string()),
                orb_name: None,
                workspace_sourced: None,
            },
        );
        let mut subcommands = IndexMap::new();
        subcommands.insert(
            "publish".to_string(),
            SubcommandConfig {
                param: Some(param_overrides),
                ..SubcommandConfig::default()
            },
        );
        let config = OrbConfig {
            subcommand: Some(subcommands),
            ..OrbConfig::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        let job = &files[&PathBuf::from("src/jobs/publish.yml")];
        assert!(
            job.contains("default: 3"),
            "integer override must render as an unquoted YAML number:\n{job}"
        );
        assert!(
            !job.contains("default: \"3\""),
            "integer override must not render as a quoted string:\n{job}"
        );
    }

    #[test]
    fn integer_param_default_not_a_quoted_string_without_any_override() {
        // Same #347 defect class, but on the plain (non-override) default
        // path: orb_param_default has no ParamType::Integer arm, so it falls
        // through to the generic string-wrapping catch-all.
        use crate::help_parser::types::Parameter;

        let retries_param = Parameter {
            long_name: "retries".to_string(),
            short: None,
            param_type: ParamType::Integer,
            default: Some("2".to_string()),
            required: false,
            description: "Retry count.".to_string(),
            ..Default::default()
        };
        let sub = make_leaf("publish", vec![retries_param]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);
        let job = &files[&PathBuf::from("src/jobs/publish.yml")];
        assert!(
            job.contains("default: 2"),
            "integer param's plain CLI default must render as an unquoted YAML number:\n{job}"
        );
        assert!(
            !job.contains("default: \"2\""),
            "integer param's plain CLI default must not render as a quoted string:\n{job}"
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
            ..Default::default()
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
                ..Default::default()
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
                ..Default::default()
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
                ..Default::default()
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

        // gen-circleci-orb#423: only an INHERITED option (declared by an
        // ancestor, so the same input in every step) is unified.
        let mut shared_param = make_param("orb_path", Some("src/@orb.yml"), false);
        shared_param.inherited = true;
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
                ..Default::default()
            }]),
            ..OrbConfig::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        let job = &files[&PathBuf::from("src/jobs/sync.yml")];
        assert!(
            job.contains("\n  orb_path:\n"),
            "an inherited param must appear once, under its own name:\n{job}"
        );
        assert!(
            job.contains("generate:\n    orb_path: << parameters.orb_path >>")
                && job.contains("validate:\n    orb_path: << parameters.orb_path >>"),
            "the single job value must be forwarded to every step:\n{job}"
        );
    }

    #[test]
    fn job_group_explicit_param_inherited_in_one_step_only_is_split_per_declaration() {
        // gen-circleci-orb#423 review: each declaring step is judged on its
        // own. `x` is inherited in `generate` but independently declared by
        // `release`, so it must NOT be tied into one shared job value.
        use crate::orb_config::{JobGroup, OrbConfig};

        for inherited_first in [true, false] {
            let mut a = make_param("x", None, false);
            a.description = "Inherited x.".to_string();
            a.inherited = true;
            let mut b = make_param("x", None, false);
            b.description = "Release's own x.".to_string();
            let steps = if inherited_first {
                vec![("generate", a.clone()), ("release", b.clone())]
            } else {
                vec![("release", b), ("generate", a)]
            };
            let cli = make_cli(
                "mytool",
                steps
                    .iter()
                    .map(|(n, p)| make_leaf(n, vec![p.clone()]))
                    .collect(),
            );
            let config = OrbConfig {
                job_group: Some(vec![JobGroup {
                    name: "sync".to_string(),
                    description: None,
                    steps: steps.iter().map(|(n, _)| n.to_string()).collect(),
                    params: Some(vec!["x".to_string()]),
                    ..Default::default()
                }]),
                ..OrbConfig::default()
            };
            let files = generate(&cli, &default_opts(), Some(&config));
            let job = &files[&PathBuf::from("src/jobs/sync.yml")];
            assert!(
                job.contains("release:\n    x: << parameters.release_x >>")
                    && job.contains("Release's own x."),
                "release's independent option keeps its own parameter \
                 (inherited_first={inherited_first}):\n{job}"
            );
            assert!(
                job.contains("generate:\n    x: << parameters.x >>"),
                "the inherited declaration is still wired through \
                 (inherited_first={inherited_first}):\n{job}"
            );
            assert!(
                !job.contains("release:\n    x: << parameters.x >>"),
                "release must never be tied to the inherited job value \
                 (inherited_first={inherited_first}):\n{job}"
            );
        }
    }

    #[test]
    fn job_group_key_collisions_reports_two_declarations_landing_on_one_job_key() {
        // Note from the #431 review: the group-scoped key `{group}_{param}`
        // for an inherited restricted `--name` (`sync_name`) is synthesized
        // with no check against a genuinely different flag that resolves to
        // the same key -- the second insert used to silently overwrite the
        // first, and the invoke step then forwarded one value to both.
        use crate::orb_config::{JobGroup, OrbConfig};

        let mut inherited_name = make_param("name", None, true);
        inherited_name.inherited = true;
        let cli = make_cli(
            "mytool",
            vec![
                make_leaf("generate", vec![inherited_name.clone()]),
                make_leaf(
                    "release",
                    vec![inherited_name, make_param("sync_name", None, true)],
                ),
            ],
        );
        let config = OrbConfig {
            job_group: Some(vec![JobGroup {
                name: "sync".to_string(),
                description: None,
                steps: vec!["generate".to_string(), "release".to_string()],
                params: None,
                ..Default::default()
            }]),
            ..OrbConfig::default()
        };
        let errors = job_group_key_collisions(
            &cli,
            Some(&config),
            &compute_effective_names(&cli, Some(&config)),
        );
        assert_eq!(errors.len(), 1, "got: {errors:?}");
        assert!(
            errors[0].contains("job_group 'sync'")
                && errors[0].contains("'sync_name'")
                && errors[0].contains("orb_name"),
            "the message names the group, the key and the fix: {errors:?}"
        );
    }

    #[test]
    fn job_group_key_collisions_accepts_independent_and_shared_declarations() {
        use crate::orb_config::{JobGroup, OrbConfig};

        let mut inherited = make_param("config", None, false);
        inherited.inherited = true;
        let cli = make_cli(
            "mytool",
            vec![
                make_leaf(
                    "generate",
                    vec![inherited.clone(), make_param("output", None, true)],
                ),
                make_leaf("release", vec![inherited, make_param("output", None, true)]),
            ],
        );
        let config = OrbConfig {
            job_group: Some(vec![JobGroup {
                name: "sync".to_string(),
                description: None,
                steps: vec!["generate".to_string(), "release".to_string()],
                params: None,
                ..Default::default()
            }]),
            ..OrbConfig::default()
        };
        let errors = job_group_key_collisions(
            &cli,
            Some(&config),
            &compute_effective_names(&cli, Some(&config)),
        );
        assert!(errors.is_empty(), "got: {errors:?}");
    }

    #[test]
    fn job_group_step_prefix_is_snake_cased_for_a_hyphenated_step_name() {
        // Orb parameter keys must be snake_case (RC010): a step named
        // `add-thing` prefixes its per-step key `add_thing_`, never
        // `add-thing_`, exactly as `resolve_command_param_name` does.
        use crate::orb_config::{JobGroup, OrbConfig};

        let cli = make_cli(
            "mytool",
            vec![
                make_leaf("add-thing", vec![make_param("path", None, true)]),
                make_leaf("remove-thing", vec![make_param("path", None, true)]),
            ],
        );
        let config = OrbConfig {
            job_group: Some(vec![JobGroup {
                name: "sync".to_string(),
                description: None,
                steps: vec!["add-thing".to_string(), "remove-thing".to_string()],
                params: None,
                ..Default::default()
            }]),
            ..OrbConfig::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        let job = &files[&PathBuf::from("src/jobs/sync.yml")];
        assert!(
            job.contains("\n  add_thing_path:\n") && job.contains("\n  remove_thing_path:\n"),
            "per-step keys must be snake_case:\n{job}"
        );
        assert!(
            !job.contains("add-thing_path") && !job.contains("remove-thing_path"),
            "a hyphen must never leak into a parameter key:\n{job}"
        );
    }

    #[test]
    fn job_group_never_unifies_same_named_params_the_steps_each_declare() {
        // gen-circleci-orb#423: `generate --output` and `release --output`
        // share only a name -- each subcommand declared its own, and they may
        // mean different things (two different files). Listing `output` in
        // `params` must expose one INDEPENDENT job parameter per step, never
        // a single one forwarded to both.
        use crate::orb_config::{JobGroup, OrbConfig};

        let mut generate_output = make_param("output", None, false);
        generate_output.description = "Where generate writes its report.".to_string();
        let mut release_output = make_param("output", None, false);
        release_output.description = "Where release writes its archive.".to_string();
        let cli = make_cli(
            "mytool",
            vec![
                make_leaf("generate", vec![generate_output]),
                make_leaf("release", vec![release_output]),
            ],
        );
        for params in [Some(vec!["output".to_string()]), None] {
            let explicit = params.is_some();
            let config = OrbConfig {
                job_group: Some(vec![JobGroup {
                    name: "sync".to_string(),
                    description: None,
                    steps: vec!["generate".to_string(), "release".to_string()],
                    params,
                    ..Default::default()
                }]),
                ..OrbConfig::default()
            };
            let files = generate(&cli, &default_opts(), Some(&config));
            let job = &files[&PathBuf::from("src/jobs/sync.yml")];
            assert!(
                !job.contains("<< parameters.output >>") && !job.contains("\n  output:\n"),
                "same-named options must never be unified into one job \
                 parameter (explicit={explicit}):\n{job}"
            );
            if explicit {
                assert!(
                    job.contains("Where generate writes its report.")
                        && job.contains("Where release writes its archive."),
                    "each step keeps its OWN description:\n{job}"
                );
                assert!(
                    job.contains("output: << parameters.generate_output >>")
                        && job.contains("output: << parameters.release_output >>"),
                    "each step forwards its own independent parameter:\n{job}"
                );
            }
        }
    }

    #[test]
    fn job_group_renames_a_restricted_mandatory_param_instead_of_dropping_it() {
        // gen-circleci-orb#413: the job-group path (add_mandatory_params /
        // build_job_group_invoke_step) never applied #369's restricted-param
        // rename. A required `--name` on "release" (not in the shared/explicit
        // set, since "generate" has no "name" param) must surface as
        // "release_name" in both the job's declared parameters and its
        // invoke step -- not the bare "name" key, which collides with
        // CircleCI's own reserved job "name" field.
        let name_param = Parameter {
            long_name: "name".to_string(),
            short: None,
            param_type: ParamType::String,
            default: None,
            required: true,
            description: "Name for the release.".to_string(),
            ..Default::default()
        };
        let subs = vec![
            make_leaf("generate", vec![]),
            make_leaf("release", vec![name_param]),
        ];
        let cli = make_cli("mytool", subs);
        let config = crate::orb_config::OrbConfig {
            job_group: Some(vec![crate::orb_config::JobGroup {
                name: "sync".to_string(),
                description: None,
                steps: vec!["generate".to_string(), "release".to_string()],
                params: None,
                ..Default::default()
            }]),
            ..crate::orb_config::OrbConfig::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        let job = &files[&PathBuf::from("src/jobs/sync.yml")];
        assert!(
            job.contains("release_name:"),
            "the restricted 'name' param must be renamed to 'release_name', \
             not dropped or left bare (colliding with the reserved job \
             'name' field):\n{job}"
        );
        assert!(
            !job.contains("\n  name:\n"),
            "the bare, CircleCI-reserved 'name' key must never appear as a \
             job parameter:\n{job}"
        );
        assert!(
            job.contains("release_name: << parameters.release_name >>"),
            "the invoke step must forward under the SAME key the 'release' \
             command itself declares -- 'release_name' on both sides, since \
             #412 already renames the restricted param at the command \
             level too, not the bare CLI flag name 'name':\n{job}"
        );
    }

    #[test]
    fn job_group_renames_a_mandatory_param_reserved_only_at_the_job_level() {
        // Code-review finding on #422: RESTRICTED_COMMAND_PARAMS (just
        // "name") is narrower than RESERVED_JOB_PARAMS (also type/filters/
        // matrix/requires/context/pre_steps/post_steps) -- a param like
        // "type" passes through resolve_param_orb_name unrenamed (valid as
        // a COMMAND param) but is still invalid as a bare JOB parameter key
        // (collides with CircleCI's own reserved job "type" field). Must be
        // sub-prefixed the same way a genuine cross-subcommand collision is,
        // even with no other subcommand declaring "type" at all.
        let type_param = Parameter {
            long_name: "type".to_string(),
            short: None,
            param_type: ParamType::String,
            default: None,
            required: true,
            description: "Type of the release.".to_string(),
            ..Default::default()
        };
        let subs = vec![
            make_leaf("generate", vec![]),
            make_leaf("release", vec![type_param]),
        ];
        let cli = make_cli("mytool", subs);
        let config = crate::orb_config::OrbConfig {
            job_group: Some(vec![crate::orb_config::JobGroup {
                name: "sync".to_string(),
                description: None,
                steps: vec!["generate".to_string(), "release".to_string()],
                params: None,
                ..Default::default()
            }]),
            ..crate::orb_config::OrbConfig::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        let job = &files[&PathBuf::from("src/jobs/sync.yml")];
        assert!(
            job.contains("\n  release_type:\n"),
            "the job-level-only-reserved 'type' param must be renamed to \
             'release_type', not left bare (colliding with the reserved job \
             'type' field):\n{job}"
        );
        assert!(
            !job.contains("\n  type:\n"),
            "the bare, CircleCI-reserved 'type' key must never appear as a \
             job parameter:\n{job}"
        );
        assert!(
            job.contains("type: << parameters.release_type >>"),
            "the invoke step must forward the job's 'release_type' value to \
             the command's own (unrenamed, since \"type\" is not restricted \
             at the command level) key 'type':\n{job}"
        );
    }

    #[test]
    fn job_group_explicit_restricted_param_still_reaches_the_command() {
        // gen-circleci-orb#422/#423: an explicitly job_group.params-selected
        // restricted param ("name") must never keep the bare, CircleCI-
        // reserved job key. Declared by a single step it is that step's own
        // parameter, under the same resolved key its command declares
        // ("release_name") -- not unified with anything.
        use crate::orb_config::{JobGroup, OrbConfig};

        let name_param = Parameter {
            long_name: "name".to_string(),
            short: None,
            param_type: ParamType::String,
            default: None,
            required: false,
            description: "Name for the release.".to_string(),
            ..Default::default()
        };
        let subs = vec![
            make_leaf("generate", vec![]),
            make_leaf("release", vec![name_param]),
        ];
        let cli = make_cli("mytool", subs);
        let config = OrbConfig {
            job_group: Some(vec![JobGroup {
                name: "sync".to_string(),
                description: None,
                steps: vec!["generate".to_string(), "release".to_string()],
                params: Some(vec!["name".to_string()]),
                ..Default::default()
            }]),
            ..OrbConfig::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        let job = &files[&PathBuf::from("src/jobs/sync.yml")];
        assert!(
            job.contains("\n  release_name:\n"),
            "an explicitly selected restricted param, declared by one step \
             only, is that step's own parameter under its resolved key \
             ('release_name'), not the bare, CircleCI-reserved 'name':\n{job}"
        );
        assert!(
            !job.contains("\n  name:\n"),
            "the bare, CircleCI-reserved 'name' key must never appear as a \
             job parameter:\n{job}"
        );
        assert!(
            job.contains("release_name: << parameters.release_name >>"),
            "the invoke step must forward the job's value to the command's \
             own resolved key 'release_name':\n{job}"
        );
    }

    #[test]
    fn job_group_shared_restricted_required_param_is_not_duplicated() {
        // gen-circleci-orb#422: an INHERITED restricted param ("name", the
        // same input in every step) is declared once under a group-scoped
        // job key ("sync_name") and forwarded from there to each command's
        // own resolved key -- add_mandatory_params must not ALSO add
        // per-subcommand duplicates on top of it.
        use crate::orb_config::{JobGroup, OrbConfig};

        let make_name_param = || Parameter {
            long_name: "name".to_string(),
            short: None,
            param_type: ParamType::String,
            default: None,
            required: true,
            description: "Name for the thing.".to_string(),
            inherited: true,
            ..Default::default()
        };
        let subs = vec![
            make_leaf("generate", vec![make_name_param()]),
            make_leaf("release", vec![make_name_param()]),
        ];
        let cli = make_cli("mytool", subs);
        let config = OrbConfig {
            job_group: Some(vec![JobGroup {
                name: "sync".to_string(),
                description: None,
                steps: vec!["generate".to_string(), "release".to_string()],
                params: None,
                ..Default::default()
            }]),
            ..OrbConfig::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        let job = &files[&PathBuf::from("src/jobs/sync.yml")];
        assert!(
            job.contains("\n  sync_name:\n"),
            "the inherited restricted param is declared once, group-scoped:\n{job}"
        );
        assert!(
            !job.contains("\n  name:\n")
                && !job.contains("\n  generate_name:\n")
                && !job.contains("\n  release_name:\n"),
            "no bare or per-subcommand duplicate may be DECLARED:\n{job}"
        );
        assert!(
            job.contains("generate_name: << parameters.sync_name >>")
                && job.contains("release_name: << parameters.sync_name >>"),
            "the single job value is forwarded to each command's own key:\n{job}"
        );
    }

    #[test]
    fn job_group_keeps_same_named_required_restricted_params_independent() {
        // gen-circleci-orb#423: two steps each declaring their own required
        // `--name` are independent inputs, not one shared value.
        use crate::orb_config::{JobGroup, OrbConfig};

        let make_name_param = || Parameter {
            long_name: "name".to_string(),
            required: true,
            description: "Name for the thing.".to_string(),
            ..Default::default()
        };
        let cli = make_cli(
            "mytool",
            vec![
                make_leaf("generate", vec![make_name_param()]),
                make_leaf("release", vec![make_name_param()]),
            ],
        );
        let config = OrbConfig {
            job_group: Some(vec![JobGroup {
                name: "sync".to_string(),
                description: None,
                steps: vec!["generate".to_string(), "release".to_string()],
                params: None,
                ..Default::default()
            }]),
            ..OrbConfig::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        let job = &files[&PathBuf::from("src/jobs/sync.yml")];
        assert!(
            job.contains("\n  generate_name:\n") && job.contains("\n  release_name:\n"),
            "each step gets its own independently-settable parameter:\n{job}"
        );
        assert!(
            !job.contains("sync_name"),
            "nothing is unified under a group-scoped key:\n{job}"
        );
        assert!(
            job.contains("generate_name: << parameters.generate_name >>")
                && job.contains("release_name: << parameters.release_name >>"),
            "each step forwards its own value:\n{job}"
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
                ..Default::default()
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

    // ── Rich job_group composite builder ───────────────────────────────────

    fn rich_build_group() -> crate::orb_config::JobGroup {
        use crate::orb_config::{JobGroup, JobGroupParam, JobGroupStep};
        let mut gen_with = IndexMap::new();
        gen_with.insert("format".to_string(), "binary".to_string());
        gen_with.insert(
            "orb_path".to_string(),
            "<< parameters.orb_path >>".to_string(),
        );
        gen_with.insert("force".to_string(), "true".to_string());
        let mut env = IndexMap::new();
        env.insert(
            "TAG_PREFIX".to_string(),
            "<< parameters.tag_prefix >>".to_string(),
        );
        JobGroup {
            name: "sync_and_publish".to_string(),
            description: Some("Goal-oriented composite job.".to_string()),
            parameter: Some(vec![
                JobGroupParam {
                    name: "binary_name".to_string(),
                    description: Some("Consumer binary name.".to_string()),
                    ..Default::default()
                },
                JobGroupParam {
                    name: "tag_prefix".to_string(),
                    param_type: Some("string".to_string()),
                    default: Some("v".to_string()),
                    ..Default::default()
                },
            ]),
            step: Some(vec![
                JobGroupStep {
                    builtin: Some("checkout".to_string()),
                    ..Default::default()
                },
                JobGroupStep {
                    command: Some("set_https_remote".to_string()),
                    ..Default::default()
                },
                JobGroupStep {
                    run: Some("Set up git and environment".to_string()),
                    script: Some("git fetch origin main".to_string()),
                    environment: Some(env),
                    ..Default::default()
                },
                JobGroupStep {
                    command: Some("generate".to_string()),
                    with: Some(gen_with),
                    ..Default::default()
                },
                JobGroupStep {
                    orb: Some("toolkit/setup".to_string()),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        }
    }

    fn render_rich(group: crate::orb_config::JobGroup) -> String {
        use crate::orb_config::OrbConfig;
        // A `generate` leaf exists in the CLI but rich mode must not depend on it.
        let cli = make_cli("mytool", vec![make_leaf("generate", vec![])]);
        let config = OrbConfig {
            job_group: Some(vec![group]),
            ..OrbConfig::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        files[&PathBuf::from("src/jobs/sync_and_publish.yml")].clone()
    }

    #[test]
    fn rich_job_group_declares_explicit_parameters() {
        let job = render_rich(rich_build_group());
        assert!(
            job.contains("binary_name:"),
            "declared param binary_name must appear:\n{job}"
        );
        assert!(
            job.contains("tag_prefix:") && job.contains("default: v"),
            "tag_prefix with default must appear:\n{job}"
        );
    }

    #[test]
    fn rich_job_group_boolean_and_integer_param_defaults_not_quoted_strings() {
        // Same #347 defect class as the subcommand param-override path:
        // render_rich_job_group unconditionally wrapped a declared param's
        // default in Value::String regardless of its declared type.
        use crate::orb_config::JobGroupParam;

        let mut group = rich_build_group();
        group.parameter.as_mut().unwrap().push(JobGroupParam {
            name: "force".to_string(),
            param_type: Some("boolean".to_string()),
            default: Some("true".to_string()),
            ..Default::default()
        });
        group.parameter.as_mut().unwrap().push(JobGroupParam {
            name: "attempts".to_string(),
            param_type: Some("integer".to_string()),
            default: Some("3".to_string()),
            ..Default::default()
        });
        let job = render_rich(group);
        assert!(
            job.contains("default: true"),
            "boolean job_group param default must be unquoted:\n{job}"
        );
        assert!(
            job.contains("default: 3"),
            "integer job_group param default must be unquoted:\n{job}"
        );
        assert!(
            !job.contains("default: \"true\"") && !job.contains("default: \"3\""),
            "job_group param defaults must not render as quoted strings:\n{job}"
        );
    }

    #[test]
    fn rich_job_group_steps_render_in_declared_order() {
        let job = render_rich(rich_build_group());
        let checkout = job.find("checkout").expect("checkout missing");
        let https = job
            .find("set_https_remote")
            .expect("set_https_remote missing");
        let setup = job.find("Set up git").expect("run step missing");
        let generate = job.find("generate:").expect("generate invoke missing");
        let orb = job.find("toolkit/setup").expect("orb step missing");
        assert!(
            checkout < https && https < setup && setup < generate && generate < orb,
            "steps must render in declared order:\n{job}"
        );
    }

    #[test]
    fn rich_job_group_command_with_emits_literal_and_ref_values() {
        let job = render_rich(rich_build_group());
        assert!(
            job.contains("format: binary"),
            "literal value must render unquoted:\n{job}"
        );
        assert!(
            job.contains("orb_path: << parameters.orb_path >>"),
            "parameter ref must render:\n{job}"
        );
        assert!(
            job.contains("force: true"),
            "boolean literal must coerce to YAML bool:\n{job}"
        );
    }

    #[test]
    fn rich_job_group_run_step_externalizes_script_to_include() {
        use crate::orb_config::OrbConfig;
        let cli = make_cli("mytool", vec![make_leaf("generate", vec![])]);
        let config = OrbConfig {
            job_group: Some(vec![rich_build_group()]),
            ..OrbConfig::default()
        };
        let files = generate(&cli, &default_opts(), Some(&config));
        let job = &files[&PathBuf::from("src/jobs/sync_and_publish.yml")];

        // The run step keeps its name + environment, but the (potentially long)
        // script is externalized to a scripts file and referenced via include
        // so the generated orb stays RC009-compliant.
        assert!(
            job.contains("name: Set up git and environment"),
            "run step name must appear:\n{job}"
        );
        assert!(
            job.contains("TAG_PREFIX: << parameters.tag_prefix >>"),
            "run step environment must appear:\n{job}"
        );
        assert!(
            job.contains("<<include(scripts/sync_and_publish_set_up_git_and_environment.sh)>>"),
            "run command must be an <<include(...)>>, not inline:\n{job}"
        );
        assert!(
            !job.contains("git fetch origin main"),
            "script body must NOT be inlined into the job:\n{job}"
        );

        // The script body lives in its own file.
        let script = files
            .get(&PathBuf::from(
                "src/scripts/sync_and_publish_set_up_git_and_environment.sh",
            ))
            .expect("externalized run-step script file must be generated");
        assert!(
            script.contains("git fetch origin main"),
            "externalized script must contain the body:\n{script}"
        );
    }

    #[test]
    fn rich_job_group_set_https_remote_renders_as_bare_step() {
        let job = render_rich(rich_build_group());
        // No-arg command must be a bare list item, not a mapping with params.
        assert!(
            job.contains("- set_https_remote\n"),
            "set_https_remote must render as a bare step:\n{job}"
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

    // ── hardcode_check: generalized check-only baked-in flag (#350) ──────────

    fn check_param() -> Parameter {
        Parameter {
            long_name: "check".to_string(),
            short: None,
            param_type: ParamType::Boolean,
            default: None,
            required: false,
            description: "Check only, do not write.".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn hardcode_check_bakes_the_flag_in_and_drops_the_parameter() {
        let sub = make_leaf("wire-ci", vec![check_param()]);
        let cli = make_cli("mytool", vec![sub]);
        let config = config_with_subcommand(
            "wire-ci",
            crate::orb_config::SubcommandConfig {
                hardcode_check: Some(true),
                ..crate::orb_config::SubcommandConfig::default()
            },
        );
        let files = generate(&cli, &default_opts(), Some(&config));

        let command = &files[&PathBuf::from("src/commands/wire_ci.yml")];
        assert!(
            !command.contains("check:"),
            "hardcode_check must drop 'check' as a forwarded command \
             parameter entirely:\n{command}"
        );

        let script = &files[&PathBuf::from("src/scripts/wire_ci.sh")];
        assert!(
            script.contains("set -- \"$@\" --check"),
            "hardcode_check must bake '--check' into the script as a \
             literal, unconditional flag:\n{script}"
        );
        assert!(
            !script.contains("GCO_CHECK"),
            "a baked-in flag must not also read an env var (that would \
             make it consumer-controlled again):\n{script}"
        );

        let job = &files[&PathBuf::from("src/jobs/wire_ci.yml")];
        assert!(
            !job.contains("check:"),
            "hardcode_check must drop 'check' as a forwarded job parameter \
             too:\n{job}"
        );
    }

    #[test]
    fn hardcode_check_false_leaves_check_as_a_normal_forwarded_param() {
        // Regression: without hardcode_check, a `check` flag behaves like any
        // other boolean parameter — forwarded, consumer-settable, read via
        // its own env var.
        let sub = make_leaf("wire-ci", vec![check_param()]);
        let cli = make_cli("mytool", vec![sub]);
        let files = generate(&cli, &default_opts(), None);

        let command = &files[&PathBuf::from("src/commands/wire_ci.yml")];
        assert!(
            command.contains("check:"),
            "without hardcode_check, 'check' must remain a normal forwarded \
             parameter:\n{command}"
        );

        let script = &files[&PathBuf::from("src/scripts/wire_ci.sh")];
        assert!(
            script.contains("GCO_CHECK"),
            "without hardcode_check, 'check' must be read via its own env \
             var like any other boolean flag:\n{script}"
        );
    }

    /// Code review on gen-circleci-orb#350's PR: two leaves share a bare
    /// name (root `release` and nested `ci.release`), so `hardcode_check`
    /// keyed off the bare name would bleed a config section meant only for
    /// the root onto the nested leaf too, even though `compute_effective_names`
    /// qualifies the nested one to `ci_release` — the same collision class
    /// #418/#425 already fixed for other `[subcommand.<name>]` lookups.
    #[test]
    fn hardcode_check_is_scoped_to_the_intended_occurrence_only() {
        let root = make_leaf("release", vec![check_param()]);
        let nested = make_leaf("release", vec![check_param()]);
        let group = SubCommand {
            name: "ci".to_string(),
            description: "CI commands.".to_string(),
            short_about: "CI commands.".to_string(),
            is_leaf: false,
            parameters: vec![],
            subcommands: vec![nested],
        };
        let cli = make_cli("mytool", vec![root, group]);
        let config = config_with_subcommand(
            "release",
            crate::orb_config::SubcommandConfig {
                hardcode_check: Some(true),
                ..crate::orb_config::SubcommandConfig::default()
            },
        );
        let files = generate(&cli, &default_opts(), Some(&config));

        let root_command = &files[&PathBuf::from("src/commands/release.yml")];
        assert!(
            !root_command.contains("check:"),
            "root 'release' IS named by the config, so its 'check' param \
             must be baked in and dropped:\n{root_command}"
        );

        let nested_command = &files[&PathBuf::from("src/commands/ci_release.yml")];
        assert!(
            nested_command.contains("check:"),
            "nested 'ci_release' is a DIFFERENT effective name, never \
             targeted by '[subcommand.release] hardcode_check', so its own \
             'check' param must stay a normal forwarded parameter:\n{nested_command}"
        );
    }
}
