use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::commands::init::{
    DEFAULT_DOCKER_CONTEXT, DEFAULT_DOCKER_ORB_VERSION, DEFAULT_GEN_ORB_MCP_ORB_VERSION,
    DEFAULT_MCP_EARLIEST_VERSION, DEFAULT_ORB_CONTEXT,
};
use crate::{ci_patcher, orb_config};

/// Re-sync an existing consumer's orb-managed CI wiring to the current generator
/// flow.
///
/// Reads the committed `gen-circleci-orb.toml` (never overwrites it) and rewrites
/// only the gen-circleci-orb-managed blocks in `.circleci/config.yml`, preserving
/// the consumer's own jobs and customizations. Run with `--check` in CI to fail
/// when the wiring is out of date. It also checks the arguments of every
/// gen-circleci-orb job invocation in the CI files against the orb's job
/// parameters, removing arguments a job no longer declares.
#[derive(Debug, clap::Args)]
pub struct Update {
    /// Path to gen-circleci-orb.toml.
    #[arg(long, default_value = "gen-circleci-orb.toml")]
    pub config: PathBuf,

    /// Path to the .circleci/ directory.
    #[arg(long, default_value = ".circleci")]
    pub ci_dir: PathBuf,

    /// Verify mode: write nothing and exit non-zero when the CI wiring is out of date.
    ///
    /// Prints a diff and upgrade guidance. For use in CI.
    #[arg(long)]
    pub check: bool,
}

impl Update {
    pub fn run(&self) -> Result<()> {
        let config = orb_config::load_config(&self.config)
            .with_context(|| format!("reading {}", self.config.display()))?;
        // `update` relies on init-captured config; it must not guess. Fail on a
        // missing required section, warn on present-but-empty required fields.
        for w in validate_config_completeness(&config)? {
            eprintln!("warning: {w}");
        }
        let opts = opts_from_config(&config);
        // Decided once per run: honours `NO_COLOR` and a non-tty stderr (e.g.
        // output piped to a log file), and stays consistent across every
        // message this run prints.
        let color = console::colors_enabled_stderr();
        self.warn_pin_mismatches(&opts, color)?;

        let mut drifted: Vec<String> = Vec::new();
        for (filename, resync_fn) in resync_targets(&opts) {
            let path = self.ci_dir.join(&filename);
            // The post_merge_ci_file is the one target most consumers won't
            // already have (see docs/post-merge-regeneration.md): its
            // absence means "create it," not a real missing-file error —
            // unlike config.yml, which every consumer already has.
            let is_post_merge_target = !opts.post_merge_branch_patterns.is_empty()
                && filename == opts.post_merge_ci_file
                && filename != "config.yml";
            let current = if is_post_merge_target && !path.exists() {
                String::new()
            } else {
                std::fs::read_to_string(&path)
                    .with_context(|| format!("reading {}", path.display()))?
            };
            let (resynced, report) = resync_fn(&current, &opts);

            // Content the strip kept because it was not recognised as ours, yet
            // sat inside a managed-marker region: preserved, but worth a human's
            // eyes.
            if !report.warnings.is_empty() {
                eprintln!(
                    "warning: {} item(s) inside a gen-circleci-orb managed region in {} \
                     were not recognised and have been preserved — review them (a marker \
                     may be damaged, or custom content was added inside a managed block):",
                    report.warnings.len(),
                    filename,
                );
                for w in &report.warnings {
                    eprintln!("  - {w}");
                }
            }

            if self.check {
                if resynced != current {
                    eprintln!(
                        "{}",
                        drift_message(
                            &opts.gen_circleci_orb_version,
                            &filename,
                            &current,
                            &resynced,
                            color,
                        )
                    );
                    drifted.push(filename.clone());
                }
                continue;
            }

            if resynced != current {
                // The consumer's own jobs and comments live in this file and are
                // rewritten wholesale, so it is replaced rather than truncated.
                crate::fs_atomic::write_atomically(&path, &resynced)
                    .with_context(|| format!("writing {}", path.display()))?;
                println!("Re-synced CI wiring in {}", path.display());
            } else {
                println!("{} CI wiring already up to date.", path.display());
            }
        }

        let arg_problems = self.validate_orb_arguments(color)?;

        let mut failures = Vec::new();
        if !drifted.is_empty() {
            failures.push(format!(
                "CI wiring is out of date in {} — run `gen-circleci-orb update`",
                drifted.join(", ")
            ));
        }
        if !arg_problems.is_empty() {
            failures.push(format!(
                "invalid orb job arguments:\n\n{}",
                arg_problems.join("\n\n")
            ));
        }
        if !failures.is_empty() {
            anyhow::bail!(failures.join("\n"));
        }
        if self.check {
            println!("CI wiring is up to date.");
        }
        Ok(())
    }

    /// Warn about every CI file that pins the orb at a version other than this
    /// binary's, before any drift is reported, so the cause reads first.
    fn warn_pin_mismatches(&self, opts: &ci_patcher::PatchOpts, color: bool) -> Result<()> {
        let managed: Vec<String> = resync_targets(opts).into_iter().map(|(f, _)| f).collect();
        for path in ci_yaml_files(&self.ci_dir)? {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            let is_managed = path
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|n| managed.iter().any(|m| m == n));
            let pin = crate::orb_wiring::orb_pin(&content);
            if let Some(w) = pin_warning(&path, pin.as_deref(), is_managed, color) {
                eprintln!("warning: {w}");
            }
        }
        Ok(())
    }

    /// Check the orb job arguments in every CI file under `ci_dir` that
    /// imports this orb. Managed blocks are valid by construction; this covers
    /// hand-authored invocations (e.g. in `release.yml`). In write mode,
    /// arguments the job no longer declares are removed; anything that cannot be
    /// fixed mechanically is returned as one message per problem.
    fn validate_orb_arguments(&self, color: bool) -> Result<Vec<String>> {
        let schema = crate::orb_wiring::schema()?;
        let mut problems = Vec::new();
        for path in ci_yaml_files(&self.ci_dir)? {
            let mut content = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            let mut findings = crate::orb_wiring::validate(&content, &schema);
            // The schema is this binary's version of the orb. A file pinned to
            // another version is never rewritten, since it may legitimately use
            // other arguments.
            let pin = crate::orb_wiring::orb_pin(&content);
            let pin_matches = pin.as_deref() == Some(env!("CARGO_PKG_VERSION"));
            if !self.check && pin_matches && !findings.is_empty() {
                let stripped = crate::orb_wiring::strip_unexpected(&content, &findings);
                if stripped != content {
                    crate::fs_atomic::write_atomically(&path, &stripped)
                        .with_context(|| format!("writing {}", path.display()))?;
                    println!("Removed stale orb job arguments from {}", path.display());
                    content = stripped;
                    findings = crate::orb_wiring::validate(&content, &schema);
                }
            }
            if !findings.is_empty() {
                problems.push(argument_message(
                    &path,
                    &content,
                    &findings,
                    pin.as_deref(),
                    pin_matches,
                    color,
                ));
            }
        }
        Ok(problems)
    }
}

/// The file name highlighted for a terminal — a user scanning a report of
/// several problems needs to see which file each one is in at a glance. Plain
/// text when `color` is false (`Update::run` decides this once, via
/// `console::colors_enabled_stderr()`, honouring `NO_COLOR` and a non-tty
/// stderr).
fn styled_file(file: impl std::fmt::Display, color: bool) -> String {
    if color {
        console::style(file).cyan().force_styling(true).to_string()
    } else {
        file.to_string()
    }
}

/// A warning when `path` pins the orb at a version other than this binary's,
/// saying what `update` will and will not do about it. `managed` is whether
/// `update` regenerates this file's orb pin. `None` when the versions agree or
/// the file does not import the orb.
fn pin_warning(
    path: &std::path::Path,
    pin: Option<&str>,
    managed: bool,
    color: bool,
) -> Option<String> {
    let version = env!("CARGO_PKG_VERSION");
    let pin = pin.filter(|p| *p != version)?;
    let action = if managed {
        format!("`update` will set this pin to {version}")
    } else {
        format!(
            "`update` does not change this pin (bump it to {version}, or use the CLI \
             matching the pin); orb job arguments are checked against {version} and \
             this file is not rewritten"
        )
    };
    Some(format!(
        "{} pins gen-circleci-orb@{pin} but the binary is {version}; {action}",
        styled_file(path.display(), color)
    ))
}

/// Operator-facing message for one file's invalid orb job arguments. Mirrors
/// `drift_message`'s shape (numbered steps, then a diff summary) for the
/// arguments `update` can fix by removing them; the file is otherwise
/// untouched, so the message says what to do by hand instead.
fn argument_message(
    path: &std::path::Path,
    content: &str,
    findings: &[crate::orb_wiring::Finding],
    pin: Option<&str>,
    pin_matches: bool,
    color: bool,
) -> String {
    use crate::orb_wiring::FindingKind;
    let version = env!("CARGO_PKG_VERSION");
    let mut out = format!(
        "{}: invalid orb job argument(s) for gen-circleci-orb@{version}.\n",
        styled_file(path.display(), color)
    );
    for f in findings {
        out.push_str(&format!("  - {f}\n"));
    }

    let fixable: Vec<crate::orb_wiring::Finding> = findings
        .iter()
        .filter(|f| matches!(f.kind, FindingKind::UnexpectedArg(_)))
        .cloned()
        .collect();
    let has_unfixable = fixable.len() != findings.len();

    if !fixable.is_empty() {
        if pin_matches {
            let stripped = crate::orb_wiring::strip_unexpected(content, &fixable);
            out.push_str(&format!(
                "  1. Run `gen-circleci-orb update` to remove the unexpected argument(s).\n\
                 \x20 2. Commit + push the config change.\n\
                 Summary of the change (run `gen-circleci-orb update` then `git diff` for \
                 the exact diff):\n{}\n",
                line_diff(content, &stripped)
            ));
        } else {
            out.push_str(&format!(
                "  1. This file pins gen-circleci-orb@{} but the binary is {version}: use \
                 the CLI matching the pin, or bump the pin.\n\
                 \x20 2. Re-sync the wiring:               gen-circleci-orb update\n",
                pin.unwrap_or("?")
            ));
        }
    }
    if has_unfixable {
        out.push_str(
            "  Edit the file by hand — `update` cannot add a missing argument or \
             rename a job.\n",
        );
    }
    out.trim_end().to_string()
}

/// `*.yml` / `*.yaml` files directly under `dir`, in name order.
fn ci_yaml_files(dir: &std::path::Path) -> Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "yml" || x == "yaml"))
        .collect();
    files.sort();
    Ok(files)
}

/// Files (relative to `ci_dir`) `update` resyncs, and the function that
/// resyncs each — always `config.yml`, plus `[post_merge_regen]`'s named file
/// when it differs from `config.yml` (the same-file case is folded into
/// `config.yml`'s own resync via `resync_build_composed`).
fn resync_targets(opts: &ci_patcher::PatchOpts) -> Vec<(String, ci_patcher::PatchFn)> {
    let mut targets: Vec<(String, ci_patcher::PatchFn)> =
        vec![("config.yml".to_string(), ci_patcher::resync_build_composed)];
    if !opts.post_merge_branch_patterns.is_empty() && opts.post_merge_ci_file != "config.yml" {
        targets.push((
            opts.post_merge_ci_file.clone(),
            ci_patcher::resync_post_merge_regen,
        ));
    }
    targets
}

/// Validate that the loaded config carries the sections/fields `update` needs to
/// regenerate stable CI across tool upgrades. `init` captures these interactively;
/// `update` is non-interactive and must never guess or emit a construct whose
/// required config value is absent. Returns an error (pointing at `init`) when a
/// required section is missing, and warnings for present-but-empty required fields.
fn validate_config_completeness(config: &orb_config::OrbConfig) -> Result<Vec<String>> {
    let is_blank = |v: &Option<String>| v.as_deref().unwrap_or_default().trim().is_empty();

    // [orb] — the binary name + namespaces underpin every generated job.
    let Some(orb) = config.orb.as_ref() else {
        anyhow::bail!(
            "gen-circleci-orb.toml has no [orb] section — the binary name and \
             namespaces it provides underpin every generated job, and update must \
             not guess. Run `gen-circleci-orb init` to configure it."
        );
    };
    if is_blank(&orb.binary) {
        anyhow::bail!(
            "[orb].binary is empty — it names the generated jobs, executor and MCP \
             server; update cannot proceed without it. Run `gen-circleci-orb init`."
        );
    }

    // [ci] — configures the orb-release / publish / MCP wiring.
    let Some(ci) = config.ci.as_ref() else {
        anyhow::bail!(
            "gen-circleci-orb.toml has no [ci] section — the generated orb-release \
             wiring (container build, orb publish, MCP build) cannot be produced \
             without it, and update must not guess. Run `gen-circleci-orb init` to \
             configure it."
        );
    };

    let mut warnings = Vec::new();

    if orb.namespaces.as_deref().unwrap_or_default().is_empty() {
        warnings.push(
            "[orb].namespaces is empty — the ensure_orb_registered and orb publish \
             steps have no target namespace; set it or re-run `gen-circleci-orb init`"
                .to_string(),
        );
    }

    // [record] is optional (its absence disables auto-record), so warn rather than
    // fail — this surfaces an accidental loss (e.g. a corrupted config) while a
    // deliberate `[record]` opt-out (even `enabled = false`) silences it.
    if config.record.is_none() {
        warnings.push(
            "gen-circleci-orb.toml has no [record] section — auto-record (the signed \
             commit-back that keeps the published orb in sync with the CLI) is \
             disabled. If intentional, add a `[record]` section with `enabled = false` \
             to silence this; if unexpected, the config may be corrupted — run \
             `gen-circleci-orb init` to restore it."
                .to_string(),
        );
    }

    if is_blank(&ci.crate_tag_prefix) {
        warnings.push(
            "[ci].crate_tag_prefix is empty — the orb-release tag filter and \
             CIRCLE_TAG normalisation will be malformed; set it or re-run \
             `gen-circleci-orb init`"
                .to_string(),
        );
    }
    if is_blank(&ci.docker_namespace) {
        warnings.push(
            "[ci].docker_namespace is empty — the build_container step will push to \
             an invalid image name; set it or re-run `gen-circleci-orb init`"
                .to_string(),
        );
    }
    if let Some(post_merge_regen) = config.post_merge_regen.as_ref() {
        let record_enabled = config.record.as_ref().is_some_and(|r| r.enabled);
        if !record_enabled {
            anyhow::bail!(
                "gen-circleci-orb.toml has a [post_merge_regen] section but \
                 [record].enabled is not true — [post_merge_regen] relocates \
                 regen+record, and there is nothing to relocate without \
                 [record] enabled. Either enable [record] or remove \
                 [post_merge_regen]."
            );
        }
        if post_merge_regen
            .requires
            .iter()
            .any(|r| r.trim().is_empty())
        {
            anyhow::bail!(
                "[post_merge_regen].requires has a blank entry — every job \
                 name in the list must be non-empty. Remove the blank entry \
                 or fix the typo (e.g. a trailing comma)."
            );
        }
    }

    if ci.mcp.unwrap_or(false) {
        if is_blank(&ci.mcp_earliest_version) {
            warnings.push(
                "[ci].mcp_earliest_version is empty — build_mcp_server will prime \
                 from an unset earliest version; set it or re-run `gen-circleci-orb init`"
                    .to_string(),
            );
        }
        if ci.mcp_context.as_deref().unwrap_or_default().is_empty() {
            warnings.push(
                "[ci].mcp_context is empty — build_mcp_server will attach no context \
                 (no signing/push credentials); set it or re-run `gen-circleci-orb init`"
                    .to_string(),
            );
        }
    }
    Ok(warnings)
}

/// Build `ci_patcher::PatchOpts` from the committed gen-circleci-orb.toml. Fields
/// not stored in the toml fall back to the same defaults `init` uses; the orb
/// version pin is this binary's own version (orb + crate release together).
fn opts_from_config(config: &orb_config::OrbConfig) -> ci_patcher::PatchOpts {
    let orb = config.orb.as_ref();
    let ci = config.ci.as_ref();
    let record = config.record.as_ref();
    let post_merge_regen = config.post_merge_regen.as_ref();
    ci_patcher::PatchOpts {
        binary: orb.and_then(|o| o.binary.clone()).unwrap_or_default(),
        build_executor: ci
            .and_then(|c| c.build_executor.clone())
            .unwrap_or_default(),
        namespaces: orb.and_then(|o| o.namespaces.clone()).unwrap_or_default(),
        orb_dir: crate::orb_config::non_empty(orb.map(|o| o.orb_dir.clone()))
            .unwrap_or_else(|| crate::orb_config::DEFAULT_ORB_DIR.to_string()),
        docker_namespace: ci
            .and_then(|c| c.docker_namespace.clone())
            .unwrap_or_default(),
        build_workflow: ci
            .and_then(|c| c.build_workflow.clone())
            .unwrap_or_else(|| "validation".to_string()),
        release_workflow: ci
            .and_then(|c| c.release_workflow.clone())
            .unwrap_or_else(|| "release".to_string()),
        requires_job: ci.and_then(|c| c.requires_job.clone()),
        crate_tag_prefix: ci
            .and_then(|c| c.crate_tag_prefix.clone())
            .unwrap_or_default(),
        release_after_job: ci
            .and_then(|c| c.release_after_job.clone())
            .unwrap_or_default(),
        orb_tools_version: "12.3.3".to_string(),
        docker_orb_version: DEFAULT_DOCKER_ORB_VERSION.to_string(),
        docker_context: ci
            .and_then(|c| c.docker_context.clone())
            .unwrap_or_else(|| DEFAULT_DOCKER_CONTEXT.to_string()),
        orb_context: ci
            .and_then(|c| c.orb_context.clone())
            .unwrap_or_else(|| DEFAULT_ORB_CONTEXT.to_string()),
        private_namespaces: vec![],
        gen_circleci_orb_version: env!("CARGO_PKG_VERSION").to_string(),
        mcp: ci.and_then(|c| c.mcp).unwrap_or(false),
        mcp_earliest_version: ci
            .and_then(|c| c.mcp_earliest_version.clone())
            .unwrap_or_else(|| DEFAULT_MCP_EARLIEST_VERSION.to_string()),
        mcp_context: ci.and_then(|c| c.mcp_context.clone()).unwrap_or_default(),
        gen_orb_mcp_orb_version: ci
            .and_then(|c| c.gen_orb_mcp_orb_version.clone())
            .unwrap_or_else(|| DEFAULT_GEN_ORB_MCP_ORB_VERSION.to_string()),
        record_contexts: record.map(|r| r.contexts.clone()).unwrap_or_default(),
        record_push_ssh_fingerprint: record
            .map(|r| r.push_ssh_fingerprint.clone())
            .unwrap_or_default(),
        test_generation: ci.and_then(|c| c.test_generation).unwrap_or(true),
        post_merge_branch_patterns: post_merge_regen
            .map(|p| p.branch_patterns.clone())
            .unwrap_or_default(),
        post_merge_workflow: post_merge_regen
            .map(|p| p.workflow.clone())
            .unwrap_or_default(),
        post_merge_ci_file: post_merge_regen.map(|p| p.file.clone()).unwrap_or_default(),
        post_merge_requires: post_merge_regen
            .map(|p| p.requires.clone())
            .unwrap_or_default(),
    }
}

/// Operator-facing message when `--check` finds the wiring out of date. The local
/// CLI must be upgraded to the pinned version FIRST, or `update` reproduces the
/// old wiring.
fn drift_message(version: &str, file: &str, current: &str, would_be: &str, color: bool) -> String {
    format!(
        "{}: CI wiring is out of date for gen-circleci-orb@{version}.\n\
         \x20 1. Upgrade your local CLI to match:  cargo binstall gen-circleci-orb@{version}\n\
         \x20    (an older CLI would re-create the OLD wiring)\n\
         \x20 2. Re-sync the wiring:               gen-circleci-orb update\n\
         \x20 3. Commit + push the config change.\n\
         Summary of the change (run `gen-circleci-orb update` then `git diff` for the exact diff):\n{}",
        styled_file(file, color),
        line_diff(current, would_be)
    )
}

/// Minimal line diff for the alert. Not a true unified diff (the authoritative
/// diff is `git diff` after running `update`); removed lines are shown with
/// `-`, added lines with `+`. A real (Myers) line diff, not a set-membership
/// comparison — gen-circleci-orb#409: `current`/`would_be` can differ only by
/// the POSITION of otherwise-identical lines (e.g. a hand-added job moving
/// relative to the managed block) or by a dropped DUPLICATE of a line that
/// still appears elsewhere, and a set-based comparison shows neither as a
/// change even though `resynced != current` is true.
fn line_diff(current: &str, would_be: &str) -> String {
    use similar::{ChangeTag, TextDiff};
    let diff = TextDiff::from_lines(current, would_be);
    let mut out = String::new();
    for change in diff.iter_all_changes() {
        let prefix = match change.tag() {
            ChangeTag::Delete => "- ",
            ChangeTag::Insert => "+ ",
            ChangeTag::Equal => continue,
        };
        out.push_str(prefix);
        out.push_str(change.value());
        if !change.value().ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orb_config::{CiSection, OrbConfig, OrbSection, PostMergeRegenConfig, RecordConfig};
    use pretty_assertions::assert_eq;

    // ── config completeness (#155): update must rely on init-captured config ──

    /// A fully-valid config: [orb] (binary + namespaces), [ci] (tag prefix +
    /// docker namespace) and [record] all present. Each test removes/blanks one
    /// piece to isolate its effect.
    fn complete_config() -> OrbConfig {
        OrbConfig {
            orb: Some(OrbSection {
                binary: Some("mytool".to_string()),
                namespaces: Some(vec!["my-org".to_string()]),
                ..OrbSection::default()
            }),
            ci: Some(CiSection {
                crate_tag_prefix: Some("mytool-v".to_string()),
                docker_namespace: Some("my-docker-org".to_string()),
                ..CiSection::default()
            }),
            record: Some(RecordConfig {
                enabled: true,
                ..RecordConfig::default()
            }),
            ..OrbConfig::default()
        }
    }

    #[test]
    fn validate_passes_for_complete_config() {
        let warnings = validate_config_completeness(&complete_config()).unwrap();
        assert!(
            warnings.is_empty(),
            "a complete config must produce no warnings: {warnings:?}"
        );
    }

    #[test]
    fn validate_fails_when_ci_section_missing() {
        let config = OrbConfig {
            ci: None,
            ..complete_config()
        };
        let msg = validate_config_completeness(&config)
            .unwrap_err()
            .to_string();
        assert!(msg.contains("[ci]"), "error must name the section: {msg}");
        assert!(msg.contains("init"), "error must direct to `init`: {msg}");
    }

    #[test]
    fn validate_fails_when_orb_section_missing() {
        let config = OrbConfig {
            orb: None,
            ..complete_config()
        };
        let msg = validate_config_completeness(&config)
            .unwrap_err()
            .to_string();
        assert!(msg.contains("[orb]"), "error must name the section: {msg}");
        assert!(msg.contains("init"), "error must direct to `init`: {msg}");
    }

    #[test]
    fn validate_fails_when_orb_binary_empty() {
        let mut config = complete_config();
        config.orb = Some(OrbSection {
            binary: Some(String::new()),
            namespaces: Some(vec!["my-org".to_string()]),
            ..OrbSection::default()
        });
        let msg = validate_config_completeness(&config)
            .unwrap_err()
            .to_string();
        assert!(
            msg.contains("[orb].binary"),
            "error must name the empty binary: {msg}"
        );
    }

    #[test]
    fn validate_warns_on_empty_required_ci_fields() {
        let config = OrbConfig {
            ci: Some(CiSection::default()),
            ..complete_config()
        };
        let warnings = validate_config_completeness(&config).unwrap();
        assert!(
            warnings.iter().any(|w| w.contains("crate_tag_prefix")),
            "must warn on empty crate_tag_prefix: {warnings:?}"
        );
        assert!(
            warnings.iter().any(|w| w.contains("docker_namespace")),
            "must warn on empty docker_namespace: {warnings:?}"
        );
    }

    #[test]
    fn validate_warns_on_empty_orb_namespaces() {
        let config = OrbConfig {
            orb: Some(OrbSection {
                binary: Some("mytool".to_string()),
                namespaces: Some(vec![]),
                ..OrbSection::default()
            }),
            ..complete_config()
        };
        let warnings = validate_config_completeness(&config).unwrap();
        assert!(
            warnings.iter().any(|w| w.contains("namespaces")),
            "must warn on empty [orb].namespaces: {warnings:?}"
        );
    }

    #[test]
    fn validate_warns_when_record_section_missing() {
        // [record] is optional (its absence disables auto-record), but warn so an
        // accidental loss is surfaced — with a hint on how to opt out silently.
        let config = OrbConfig {
            record: None,
            ..complete_config()
        };
        let warnings = validate_config_completeness(&config).unwrap();
        let w = warnings
            .iter()
            .find(|w| w.contains("[record]"))
            .expect("expected a [record] warning");
        assert!(
            w.contains("enabled = false"),
            "warning must hint how to silence it: {w}"
        );
    }

    #[test]
    fn validate_fails_when_post_merge_regen_present_without_record_enabled() {
        // [post_merge_regen] relocates regen+record — nothing to relocate
        // without [record].enabled = true. gen-circleci-orb#328.
        let config = OrbConfig {
            record: None,
            post_merge_regen: Some(PostMergeRegenConfig {
                branch_patterns: vec!["renovate/*".to_string()],
                workflow: "update_prlog".to_string(),
                file: "update_prlog.yml".to_string(),
                ..PostMergeRegenConfig::default()
            }),
            ..complete_config()
        };
        let err = validate_config_completeness(&config).unwrap_err();
        assert!(
            err.to_string().contains("[post_merge_regen]") && err.to_string().contains("[record]"),
            "error must name both sections: {err}"
        );
    }

    #[test]
    fn validate_fails_when_post_merge_regen_present_with_record_disabled() {
        let config = OrbConfig {
            record: Some(RecordConfig {
                enabled: false,
                ..RecordConfig::default()
            }),
            post_merge_regen: Some(PostMergeRegenConfig {
                branch_patterns: vec!["renovate/*".to_string()],
                workflow: "update_prlog".to_string(),
                file: "update_prlog.yml".to_string(),
                ..PostMergeRegenConfig::default()
            }),
            ..complete_config()
        };
        let err = validate_config_completeness(&config).unwrap_err();
        assert!(err.to_string().contains("[post_merge_regen]"));
    }

    #[test]
    fn validate_passes_when_post_merge_regen_present_with_record_enabled() {
        let config = OrbConfig {
            post_merge_regen: Some(PostMergeRegenConfig {
                branch_patterns: vec!["renovate/*".to_string()],
                workflow: "update_prlog".to_string(),
                file: "update_prlog.yml".to_string(),
                ..PostMergeRegenConfig::default()
            }),
            ..complete_config()
        };
        let warnings = validate_config_completeness(&config).unwrap();
        assert!(
            warnings.is_empty(),
            "a complete config with valid post_merge_regen must produce no warnings: {warnings:?}"
        );
    }

    #[test]
    fn validate_fails_when_post_merge_regen_requires_has_a_blank_entry() {
        // Code-review finding: a stray empty-string entry (e.g. a trailing-
        // comma typo) would otherwise flow straight through to `requires: []`
        // in the generated YAML with no diagnostic. Catch it at config-load
        // time instead, matching this function's fail-loudly convention.
        let config = OrbConfig {
            post_merge_regen: Some(PostMergeRegenConfig {
                requires: vec!["update-prlog-on-main".to_string(), String::new()],
                ..PostMergeRegenConfig::default()
            }),
            ..complete_config()
        };
        let err = validate_config_completeness(&config).unwrap_err();
        assert!(
            err.to_string().contains("[post_merge_regen].requires"),
            "error must name the offending field: {err}"
        );
    }

    #[test]
    fn validate_no_record_warning_when_record_present_even_if_disabled() {
        let config = OrbConfig {
            record: Some(RecordConfig {
                enabled: false,
                ..RecordConfig::default()
            }),
            ..complete_config()
        };
        let warnings = validate_config_completeness(&config).unwrap();
        assert!(
            !warnings.iter().any(|w| w.contains("[record]")),
            "an explicit [record] opt-out must silence the warning: {warnings:?}"
        );
    }

    use std::fs;
    use tempfile::TempDir;

    fn write_repo(dir: &TempDir, toml: &str, config_yml: &str) -> (PathBuf, PathBuf) {
        let toml_path = dir.path().join("gen-circleci-orb.toml");
        let ci_dir = dir.path().join(".circleci");
        fs::create_dir_all(&ci_dir).unwrap();
        fs::write(&toml_path, toml).unwrap();
        fs::write(ci_dir.join("config.yml"), config_yml).unwrap();
        (toml_path, ci_dir)
    }

    const TOML: &str = "\
[orb]
binary = \"mytool\"
namespaces = [\"my-org\"]
orb_dir = \"orb\"

[ci]
build_workflow = \"validation\"
requires_job = \"toolkit/common_tests\"
crate_tag_prefix = \"mytool-v\"
docker_namespace = \"my-docker-org\"
";

    // An unmarked, old-flow consumer config (build-binary serial; push-orb at end).
    const OLD_CONFIG: &str = "\
version: 2.1

orbs:
  toolkit: jerus-org/circleci-toolkit@6.0.0
  gen-circleci-orb: jerus-org/gen-circleci-orb@0.0.1
  orb-tools: circleci/orb-tools@12.3.3

workflows:
  validation:
    jobs:
      - toolkit/common_tests
      - gen-circleci-orb/build_rust_binary:
          name: build-binary
          package: mytool
          requires: [toolkit/common_tests]
      - gen-circleci-orb/generate:
          name: regenerate-orb
          binary: mytool
          orb_dir: orb
          no_record: true
          requires: [build-binary]
      - orb-tools/pack:
          name: pack-orb
          requires: [regenerate-orb]
      - orb-tools/review:
          name: review-orb
          requires: [pack-orb]
      - gen-circleci-orb/generate:
          name: push-orb
          binary: mytool
          requires: [pack-orb, review-orb]

  orb-release:
    jobs:
      - gen-circleci-orb/build_rust_binary:
          name: orb-release-binary
          package: mytool
";

    #[test]
    fn opts_from_config_maps_toml_fields() {
        let config: orb_config::OrbConfig = toml::from_str(TOML).unwrap();
        let opts = opts_from_config(&config);
        assert_eq!(opts.binary, "mytool");
        assert_eq!(opts.namespaces, vec!["my-org".to_string()]);
        assert_eq!(opts.requires_job.as_deref(), Some("toolkit/common_tests"));
        assert_eq!(opts.crate_tag_prefix, "mytool-v");
        // version pin is this binary's own version
        assert_eq!(opts.gen_circleci_orb_version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn opts_from_config_test_generation_defaults_true_when_unset() {
        // TOML has no [ci].test_generation — every existing consumer's committed
        // config must keep today's live-dogfood behavior with zero action.
        let config: orb_config::OrbConfig = toml::from_str(TOML).unwrap();
        let opts = opts_from_config(&config);
        assert!(opts.test_generation);
    }

    #[test]
    fn opts_from_config_test_generation_honours_explicit_false() {
        let toml_with_opt_out = format!("{TOML}test_generation = false\n");
        let config: orb_config::OrbConfig = toml::from_str(&toml_with_opt_out).unwrap();
        let opts = opts_from_config(&config);
        assert!(!opts.test_generation);
    }

    #[test]
    fn opts_from_config_build_executor_defaults_empty_when_unset() {
        // TOML has no [ci].build_executor — the job falls back to its own
        // bundled default executor.
        let config: orb_config::OrbConfig = toml::from_str(TOML).unwrap();
        let opts = opts_from_config(&config);
        assert!(opts.build_executor.is_empty());
    }

    #[test]
    fn opts_from_config_build_executor_honours_explicit_value() {
        let toml_with_executor = format!("{TOML}build_executor = \"toolkit/rust_env_rolling\"\n");
        let config: orb_config::OrbConfig = toml::from_str(&toml_with_executor).unwrap();
        let opts = opts_from_config(&config);
        assert_eq!(opts.build_executor, "toolkit/rust_env_rolling");
    }

    #[test]
    fn opts_from_config_post_merge_regen_defaults_empty_when_unset() {
        // TOML has no [post_merge_regen] — the feature is off, zero behavior
        // change for every existing consumer.
        let config: orb_config::OrbConfig = toml::from_str(TOML).unwrap();
        let opts = opts_from_config(&config);
        assert!(opts.post_merge_branch_patterns.is_empty());
        assert_eq!(opts.post_merge_workflow, "");
        assert_eq!(opts.post_merge_ci_file, "");
    }

    #[test]
    fn opts_from_config_maps_post_merge_regen_section() {
        let toml_with_pmr = format!(
            "{TOML}\n[post_merge_regen]\nbranch_patterns = [\"renovate/*\"]\nworkflow = \"update_prlog\"\nfile = \"update_prlog.yml\"\n"
        );
        let config: orb_config::OrbConfig = toml::from_str(&toml_with_pmr).unwrap();
        let opts = opts_from_config(&config);
        assert_eq!(
            opts.post_merge_branch_patterns,
            vec!["renovate/*".to_string()]
        );
        assert_eq!(opts.post_merge_workflow, "update_prlog");
        assert_eq!(opts.post_merge_ci_file, "update_prlog.yml");
    }

    #[test]
    fn opts_from_config_maps_post_merge_regen_requires() {
        let toml_with_pmr = format!(
            "{TOML}\n[post_merge_regen]\nbranch_patterns = [\"renovate/*\"]\nworkflow = \"update_prlog\"\nfile = \"update_prlog.yml\"\nrequires = [\"update-prlog-on-main\"]\n"
        );
        let config: orb_config::OrbConfig = toml::from_str(&toml_with_pmr).unwrap();
        let opts = opts_from_config(&config);
        assert_eq!(
            opts.post_merge_requires,
            vec!["update-prlog-on-main".to_string()]
        );
    }

    #[test]
    fn opts_from_config_post_merge_regen_requires_defaults_empty() {
        let toml_with_pmr = format!(
            "{TOML}\n[post_merge_regen]\nbranch_patterns = [\"renovate/*\"]\nworkflow = \"update_prlog\"\nfile = \"update_prlog.yml\"\n"
        );
        let config: orb_config::OrbConfig = toml::from_str(&toml_with_pmr).unwrap();
        let opts = opts_from_config(&config);
        assert!(opts.post_merge_requires.is_empty());
    }

    #[test]
    fn update_check_fails_on_drift_and_writes_nothing() {
        let dir = TempDir::new().unwrap();
        let (toml, ci_dir) = write_repo(&dir, TOML, OLD_CONFIG);
        let before = fs::read_to_string(ci_dir.join("config.yml")).unwrap();
        let cmd = Update {
            config: toml,
            ci_dir: ci_dir.clone(),
            check: true,
        };
        let err = cmd.run().unwrap_err().to_string();
        assert!(
            err.contains("out of date"),
            "check must report drift: {err}"
        );
        // --check must not modify the file.
        assert_eq!(
            fs::read_to_string(ci_dir.join("config.yml")).unwrap(),
            before,
            "--check must not write the config"
        );
    }

    fn stale_release_pinned(pin: &str) -> String {
        format!(
            "\
version: 2.1
orbs:
  gen-circleci-orb: jerus-org/gen-circleci-orb@{pin}
workflows:
  release:
    jobs:
      # keep this comment
      - gen-circleci-orb/build_rust_binary:
          name: build-binary
          package: mytool
          rust_image: old-image
          requires: [approve-release]
"
        )
    }

    /// A stale `release.yml` pinned at this binary's own orb version.
    fn stale_release() -> String {
        stale_release_pinned(env!("CARGO_PKG_VERSION"))
    }

    /// A repo whose `config.yml` is already in sync, plus a `release.yml`.
    fn synced_repo_with_release(release: &str) -> (TempDir, PathBuf, PathBuf) {
        let dir = TempDir::new().unwrap();
        let (toml, ci_dir) = write_repo(&dir, TOML, OLD_CONFIG);
        Update {
            config: toml.clone(),
            ci_dir: ci_dir.clone(),
            check: false,
        }
        .run()
        .unwrap();
        fs::write(ci_dir.join("release.yml"), release).unwrap();
        (dir, toml, ci_dir)
    }

    #[test]
    fn update_check_fails_on_a_stale_orb_argument_in_release_yml() {
        let (_dir, toml, ci_dir) = synced_repo_with_release(&stale_release());
        let err = Update {
            config: toml,
            ci_dir: ci_dir.clone(),
            check: true,
        }
        .run()
        .unwrap_err()
        .to_string();
        assert!(err.contains("release.yml"), "must name the file: {err}");
        assert!(err.contains("rust_image"), "must name the argument: {err}");
        assert!(
            err.contains("Run `gen-circleci-orb update`"),
            "must say how to fix it: {err}"
        );
        assert!(
            err.contains("Summary of the change"),
            "must show what update would do: {err}"
        );
        assert!(
            err.contains("-           rust_image: old-image"),
            "must show the line update would remove: {err}"
        );
        assert_eq!(
            fs::read_to_string(ci_dir.join("release.yml")).unwrap(),
            stale_release(),
            "--check must not write"
        );
    }

    #[test]
    fn update_strips_a_stale_orb_argument_from_release_yml() {
        let (_dir, toml, ci_dir) = synced_repo_with_release(&stale_release());
        let cmd = |check| Update {
            config: toml.clone(),
            ci_dir: ci_dir.clone(),
            check,
        };
        cmd(false).run().unwrap();
        assert_eq!(
            fs::read_to_string(ci_dir.join("release.yml")).unwrap(),
            stale_release().replace("          rust_image: old-image\n", ""),
        );
        cmd(true).run().unwrap();
    }

    #[test]
    fn pin_warning_names_the_binary_version_and_the_file_pin() {
        let w = pin_warning(
            std::path::Path::new(".circleci/release.yml"),
            Some("0.0.1"),
            false,
            false,
        )
        .unwrap();
        assert!(w.contains(".circleci/release.yml"), "{w}");
        assert!(w.contains("pins gen-circleci-orb@0.0.1"), "{w}");
        assert!(
            w.contains(&format!("binary is {}", env!("CARGO_PKG_VERSION"))),
            "{w}"
        );
    }

    #[test]
    fn pin_warning_is_absent_when_the_pin_matches_or_the_orb_is_not_imported() {
        let p = std::path::Path::new("f.yml");
        assert_eq!(
            pin_warning(p, Some(env!("CARGO_PKG_VERSION")), false, false),
            None
        );
        assert_eq!(pin_warning(p, None, false, false), None);
    }

    #[test]
    fn styled_file_wraps_the_path_in_cyan_when_color_is_on() {
        assert_eq!(styled_file("config.yml", true), "\x1b[36mconfig.yml\x1b[0m");
    }

    #[test]
    fn styled_file_is_plain_text_when_color_is_off() {
        assert_eq!(styled_file("config.yml", false), "config.yml");
    }

    #[test]
    fn drift_message_names_the_file() {
        let m = drift_message("0.1.22", "config.yml", "a\n", "b\n", false);
        assert!(m.starts_with("config.yml: CI wiring is out of date"), "{m}");
    }

    #[test]
    fn pin_warning_says_update_resets_the_pin_of_a_managed_file() {
        let w = pin_warning(
            std::path::Path::new("config.yml"),
            Some("0.0.1"),
            true,
            false,
        )
        .unwrap();
        assert!(
            w.contains(&format!(
                "`update` will set this pin to {}",
                env!("CARGO_PKG_VERSION")
            )),
            "{w}"
        );
    }

    #[test]
    fn pin_warning_says_update_leaves_the_pin_of_an_unmanaged_file() {
        let w = pin_warning(
            std::path::Path::new("release.yml"),
            Some("0.0.1"),
            false,
            false,
        )
        .unwrap();
        assert!(w.contains("`update` does not change this pin"), "{w}");
        assert!(w.contains("not rewritten"), "{w}");
    }

    #[test]
    fn update_check_reports_drift_and_bad_arguments_together_and_writes_nothing() {
        let dir = TempDir::new().unwrap();
        let (toml, ci_dir) = write_repo(&dir, TOML, OLD_CONFIG);
        fs::write(ci_dir.join("release.yml"), stale_release()).unwrap();
        let config_before = fs::read_to_string(ci_dir.join("config.yml")).unwrap();
        let err = Update {
            config: toml,
            ci_dir: ci_dir.clone(),
            check: true,
        }
        .run()
        .unwrap_err()
        .to_string();
        assert!(err.contains("out of date"), "drift missing: {err}");
        assert!(err.contains("config.yml"), "drifted file not named: {err}");
        assert!(
            err.contains("rust_image"),
            "argument problem missing: {err}"
        );
        assert_eq!(
            fs::read_to_string(ci_dir.join("config.yml")).unwrap(),
            config_before
        );
        assert_eq!(
            fs::read_to_string(ci_dir.join("release.yml")).unwrap(),
            stale_release()
        );
    }

    #[test]
    fn update_fixes_drift_and_stale_arguments_in_one_run() {
        let dir = TempDir::new().unwrap();
        let (toml, ci_dir) = write_repo(&dir, TOML, OLD_CONFIG);
        fs::write(ci_dir.join("release.yml"), stale_release()).unwrap();
        let cmd = |check| Update {
            config: toml.clone(),
            ci_dir: ci_dir.clone(),
            check,
        };
        cmd(false).run().unwrap();
        cmd(true).run().unwrap();
    }

    #[test]
    fn check_error_tells_the_user_to_run_update_for_a_stale_argument() {
        let (_dir, toml, ci_dir) = synced_repo_with_release(&stale_release());
        let err = Update {
            config: toml,
            ci_dir,
            check: true,
        }
        .run()
        .unwrap_err()
        .to_string();
        assert!(err.contains("Run `gen-circleci-orb update`"), "{err}");
    }

    #[test]
    fn check_error_says_a_missing_argument_needs_a_hand_edit() {
        let release = stale_release().replace("          package: mytool\n", "");
        let (_dir, toml, ci_dir) = synced_repo_with_release(&release);
        let err = Update {
            config: toml,
            ci_dir,
            check: true,
        }
        .run()
        .unwrap_err()
        .to_string();
        assert!(err.contains("package"), "{err}");
        assert!(err.contains("Edit the file by hand"), "{err}");
    }

    #[test]
    fn check_error_for_a_differently_pinned_file_points_at_the_pin() {
        let release = stale_release_pinned("0.0.1");
        let (_dir, toml, ci_dir) = synced_repo_with_release(&release);
        let err = Update {
            config: toml,
            ci_dir,
            check: true,
        }
        .run()
        .unwrap_err()
        .to_string();
        assert!(err.contains("pins gen-circleci-orb@0.0.1"), "{err}");
    }

    #[test]
    fn update_does_not_strip_arguments_when_the_pin_differs_from_the_binary() {
        let release = stale_release_pinned("0.0.1");
        let (_dir, toml, ci_dir) = synced_repo_with_release(&release);
        let err = Update {
            config: toml,
            ci_dir: ci_dir.clone(),
            check: false,
        }
        .run()
        .unwrap_err()
        .to_string();
        assert!(err.contains("rust_image"), "{err}");
        assert_eq!(
            fs::read_to_string(ci_dir.join("release.yml")).unwrap(),
            release,
            "a file pinned to another orb version must not be rewritten"
        );
    }

    #[test]
    fn update_fails_when_a_required_orb_argument_is_missing() {
        let release = stale_release().replace("          package: mytool\n", "");
        let (_dir, toml, ci_dir) = synced_repo_with_release(&release);
        let err = Update {
            config: toml,
            ci_dir,
            check: false,
        }
        .run()
        .unwrap_err()
        .to_string();
        assert!(err.contains("package"), "{err}");
    }

    #[test]
    fn update_check_passes_a_valid_release_yml() {
        let release = stale_release().replace("          rust_image: old-image\n", "");
        let (_dir, toml, ci_dir) = synced_repo_with_release(&release);
        Update {
            config: toml,
            ci_dir,
            check: true,
        }
        .run()
        .unwrap();
    }

    #[test]
    fn update_resyncs_an_old_config_in_place() {
        let dir = TempDir::new().unwrap();
        let (toml, ci_dir) = write_repo(&dir, TOML, OLD_CONFIG);
        let cmd = Update {
            config: toml,
            ci_dir: ci_dir.clone(),
            check: false,
        };
        cmd.run().unwrap();
        let after = fs::read_to_string(ci_dir.join("config.yml")).unwrap();
        // new flow + markers, old push-orb gone, consumer job preserved.
        assert!(after.contains(ci_patcher::MANAGED_BEGIN));
        assert!(
            !after.contains("name: push-orb"),
            "old push-orb removed:\n{after}"
        );
        assert!(
            after.contains("- toolkit/common_tests"),
            "consumer job kept:\n{after}"
        );
        assert!(
            after.contains("name: orb-release-container") && !after.contains("name: verify-orb"),
            "orb-release regenerated without the removed verify-orb job (#201):\n{after}"
        );
        // re-running update is now a no-op (the wiring is current).
        let cmd2 = Update {
            config: dir.path().join("gen-circleci-orb.toml"),
            ci_dir,
            check: true,
        };
        cmd2.run().unwrap();
    }

    const TOML_WITH_POST_MERGE_REGEN: &str = "\
[orb]
binary = \"mytool\"
namespaces = [\"my-org\"]
orb_dir = \"orb\"

[ci]
build_workflow = \"validation\"
requires_job = \"toolkit/common_tests\"
crate_tag_prefix = \"mytool-v\"
docker_namespace = \"my-docker-org\"

[record]
enabled = true
contexts = [\"release\"]

[post_merge_regen]
branch_patterns = [\"renovate/*\"]
workflow = \"update_prlog\"
file = \"update_prlog.yml\"
";

    const UPDATE_PRLOG_CONFIG: &str = "\
version: 2.1

orbs:
  toolkit: jerus-org/circleci-toolkit@7.4.0

workflows:
  update_prlog:
    jobs:
      - toolkit/update_prlog:
          context: [pcu-app]
";

    #[test]
    fn update_resyncs_the_post_merge_regen_file_too() {
        // gen-circleci-orb#328: when [post_merge_regen] names a dedicated
        // file, `update` must resync it alongside config.yml, not just
        // config.yml.
        let dir = TempDir::new().unwrap();
        let (toml, ci_dir) = write_repo(&dir, TOML_WITH_POST_MERGE_REGEN, OLD_CONFIG);
        fs::write(ci_dir.join("update_prlog.yml"), UPDATE_PRLOG_CONFIG).unwrap();
        let cmd = Update {
            config: toml,
            ci_dir: ci_dir.clone(),
            check: false,
        };
        cmd.run().unwrap();
        let after = fs::read_to_string(ci_dir.join("update_prlog.yml")).unwrap();
        assert!(
            after.contains("name: post-merge-build-binary"),
            "update_prlog.yml must gain the relocated chain:\n{after}"
        );
        assert!(
            after.contains("toolkit/update_prlog:"),
            "the consumer's pre-existing job must survive:\n{after}"
        );

        // re-running update is now a no-op (both files current).
        let cmd2 = Update {
            config: dir.path().join("gen-circleci-orb.toml"),
            ci_dir,
            check: true,
        };
        cmd2.run().unwrap();
    }

    #[test]
    fn update_creates_a_missing_post_merge_regen_file() {
        // gen-circleci-orb#328 review finding: most consumers won't already
        // have a dedicated post-merge file — `update` must create it, not
        // error out reading a file that was never there.
        let dir = TempDir::new().unwrap();
        let (toml, ci_dir) = write_repo(&dir, TOML_WITH_POST_MERGE_REGEN, OLD_CONFIG);
        // Deliberately no update_prlog.yml written.
        let cmd = Update {
            config: toml,
            ci_dir: ci_dir.clone(),
            check: false,
        };
        cmd.run().unwrap();
        let created = fs::read_to_string(ci_dir.join("update_prlog.yml")).unwrap();
        assert!(created.starts_with("version: 2.1"));
        assert!(created.contains("name: post-merge-build-binary"));
    }

    // ── line_diff (gen-circleci-orb#409): must show REAL drift, not just ──
    // ── lines whose CONTENT is unique to one side ──

    #[test]
    fn line_diff_shows_a_pure_reordering_of_identical_lines() {
        // The exact failure mode hit live on the renovate/gen-circleci-orb-0.x
        // PR (#405 follow-up, gen-circleci-orb#409): `resynced != current` was
        // true (out_of_date correctly detected), but line_diff's old
        // HashSet-based implementation only reports lines whose CONTENT is
        // unique to one side — a pure position swap of byte-identical lines
        // produces an empty diff, even though `git diff` after `update`
        // would show real churn. A reader following the printed "run
        // `gen-circleci-orb update` then `git diff`" instruction sees no
        // preview of what that diff will be.
        let current = "a\nb\nc\n";
        let reordered = "a\nc\nb\n";
        let diff = line_diff(current, reordered);
        assert!(
            !diff.trim().is_empty(),
            "a pure line reordering must not produce an empty diff"
        );
    }

    #[test]
    fn line_diff_counts_duplicate_lines_by_occurrence() {
        // A second failure mode of the same HashSet bug: dropping one of two
        // duplicate occurrences of a line is invisible to set membership
        // (the line is still "in the set" once), but is real drift.
        let current = "x\nx\ny\n";
        let dropped_one = "x\ny\n";
        let diff = line_diff(current, dropped_one);
        assert!(
            !diff.trim().is_empty(),
            "dropping one of two duplicate lines must not produce an empty diff"
        );
    }

    #[test]
    fn line_diff_reports_true_content_changes() {
        // Regression guard: the common case (an actual content change, e.g.
        // the orb pin version bump) must still show up, as it always has.
        let current = "  gen-circleci-orb: jerus-org/gen-circleci-orb@0.1.19\n";
        let bumped = "  gen-circleci-orb: jerus-org/gen-circleci-orb@0.1.20\n";
        let diff = line_diff(current, bumped);
        assert!(
            diff.contains("0.1.19"),
            "must show the removed line:\n{diff}"
        );
        assert!(diff.contains("0.1.20"), "must show the added line:\n{diff}");
    }
}
