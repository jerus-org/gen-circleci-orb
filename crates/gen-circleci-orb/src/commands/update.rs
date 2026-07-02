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
/// when the wiring is out of date.
#[derive(Debug, clap::Args)]
pub struct Update {
    /// Path to gen-circleci-orb.toml.
    #[arg(long, default_value = "gen-circleci-orb.toml")]
    pub config: PathBuf,

    /// Path to the .circleci/ directory.
    #[arg(long, default_value = ".circleci")]
    pub ci_dir: PathBuf,

    /// Verify mode: write nothing and exit non-zero (with a diff and guidance)
    /// when the CI wiring is out of date. For use in CI.
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

        let config_path = self.ci_dir.join("config.yml");
        let current = std::fs::read_to_string(&config_path)
            .with_context(|| format!("reading {}", config_path.display()))?;
        let (resynced, report) = ci_patcher::resync_build(&current, &opts);

        // Content the strip kept because it was not recognised as ours, yet sat
        // inside a managed-marker region: preserved, but worth a human's eyes.
        if !report.warnings.is_empty() {
            eprintln!(
                "warning: {} item(s) inside a gen-circleci-orb managed region were not recognised \
                 and have been preserved — review them (a marker may be damaged, or custom content \
                 was added inside a managed block):",
                report.warnings.len()
            );
            for w in &report.warnings {
                eprintln!("  - {w}");
            }
        }

        if self.check {
            if resynced != current {
                eprintln!(
                    "{}",
                    drift_message(&opts.gen_circleci_orb_version, &current, &resynced)
                );
                anyhow::bail!("CI wiring is out of date — run `gen-circleci-orb update`");
            }
            println!("CI wiring is up to date.");
            return Ok(());
        }

        if resynced != current {
            std::fs::write(&config_path, &resynced)
                .with_context(|| format!("writing {}", config_path.display()))?;
            println!("Re-synced CI wiring in {}", config_path.display());
        } else {
            println!("{} CI wiring already up to date.", config_path.display());
        }
        Ok(())
    }
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
    ci_patcher::PatchOpts {
        binary: orb.and_then(|o| o.binary.clone()).unwrap_or_default(),
        namespaces: orb.and_then(|o| o.namespaces.clone()).unwrap_or_default(),
        orb_dir: orb
            .and_then(|o| o.orb_dir.clone())
            .unwrap_or_else(|| "orb".to_string()),
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
    }
}

/// Operator-facing message when `--check` finds the wiring out of date. The local
/// CLI must be upgraded to the pinned version FIRST, or `update` reproduces the
/// old wiring.
fn drift_message(version: &str, current: &str, would_be: &str) -> String {
    format!(
        "CI wiring is out of date for gen-circleci-orb@{version}.\n\
         \x20 1. Upgrade your local CLI to match:  cargo binstall gen-circleci-orb@{version}\n\
         \x20    (an older CLI would re-create the OLD wiring)\n\
         \x20 2. Re-sync the wiring:               gen-circleci-orb update\n\
         \x20 3. Commit + push the config change.\n\
         Summary of the change (run `gen-circleci-orb update` then `git diff` for the exact diff):\n{}",
        line_diff(current, would_be)
    )
}

/// Minimal line diff for the alert. Not a true unified diff (the authoritative
/// diff is `git diff` after running `update`); lines unique to the current config
/// are shown with `-`, lines unique to the re-synced config with `+`.
fn line_diff(current: &str, would_be: &str) -> String {
    use std::collections::HashSet;
    let cur: HashSet<&str> = current.lines().collect();
    let new: HashSet<&str> = would_be.lines().collect();
    let mut out = String::new();
    for l in current.lines() {
        if !new.contains(l) {
            out.push_str(&format!("- {l}\n"));
        }
    }
    for l in would_be.lines() {
        if !cur.contains(l) {
            out.push_str(&format!("+ {l}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orb_config::{CiSection, OrbConfig, OrbSection, RecordConfig};

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
            after.contains("name: verify-orb"),
            "verify gate added:\n{after}"
        );
        // re-running update is now a no-op (the wiring is current).
        let cmd2 = Update {
            config: dir.path().join("gen-circleci-orb.toml"),
            ci_dir,
            check: true,
        };
        cmd2.run().unwrap();
    }
}
