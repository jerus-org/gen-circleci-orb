//! Validation of `gen-circleci-orb/<job>` invocations in CircleCI config files.
//!
//! Managed blocks are regenerated from templates, so their arguments are valid
//! by construction. Invocations outside them (e.g. in `release.yml`) are
//! hand-authored; this module checks their arguments against the job
//! parameters declared by this binary's version of the orb.

use std::collections::BTreeMap;

use anyhow::{Context, Result};

/// CircleCI job-level keys that are not orb job parameters.
const RESERVED_KEYS: &[&str] = &[
    "name",
    "requires",
    "context",
    "filters",
    "type",
    "matrix",
    "pre-steps",
    "post-steps",
];

const ORB_SOURCE: &str = "jerus-org/gen-circleci-orb@";

const JOB_SOURCES: &[(&str, &str)] = &[
    (
        "build_container",
        include_str!("../orb-jobs/build_container.yml"),
    ),
    (
        "build_rust_binary",
        include_str!("../orb-jobs/build_rust_binary.yml"),
    ),
    (
        "ensure_orb_registered",
        include_str!("../orb-jobs/ensure_orb_registered.yml"),
    ),
    ("generate", include_str!("../orb-jobs/generate.yml")),
    ("update", include_str!("../orb-jobs/update.yml")),
];

/// Parameter names of one orb job, mapped to whether the caller must supply them.
pub type JobSchema = BTreeMap<String, bool>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindingKind {
    UnexpectedArg(String),
    MissingRequiredArg(String),
    UnknownJob,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub job: String,
    /// 1-based line of the job invocation.
    pub line: usize,
    /// 1-based line of the offending argument (`UnexpectedArg` only).
    pub arg_line: Option<usize>,
    pub kind: FindingKind,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            FindingKind::UnexpectedArg(a) => write!(
                f,
                "line {}: job '{}' has unexpected argument '{a}'",
                self.arg_line.unwrap_or(self.line),
                self.job
            ),
            FindingKind::MissingRequiredArg(a) => write!(
                f,
                "line {}: job '{}' is missing required argument '{a}'",
                self.line, self.job
            ),
            FindingKind::UnknownJob => {
                write!(f, "line {}: unknown orb job '{}'", self.line, self.job)
            }
        }
    }
}

/// Job schemas for this binary's version of the orb, keyed by job name.
pub fn schema() -> Result<BTreeMap<String, JobSchema>> {
    JOB_SOURCES
        .iter()
        .map(|(name, src)| Ok(((*name).to_string(), parse_job_schema(name, src)?)))
        .collect()
}

fn parse_job_schema(name: &str, src: &str) -> Result<JobSchema> {
    let doc: serde_yaml::Value =
        serde_yaml::from_str(src).with_context(|| format!("parsing orb job '{name}'"))?;
    let mut out = JobSchema::new();
    if let Some(params) = doc.get("parameters").and_then(|p| p.as_mapping()) {
        for (k, v) in params {
            if let Some(k) = k.as_str() {
                out.insert(k.to_string(), v.get("default").is_none());
            }
        }
    }
    Ok(out)
}

/// Check every orb job invocation in `content` against `schema`.
///
/// Returns no findings for a file that does not declare the orb.
pub fn validate(content: &str, schema: &BTreeMap<String, JobSchema>) -> Vec<Finding> {
    let lines: Vec<&str> = content.lines().collect();
    let Some(alias) = orb_alias(&lines) else {
        return Vec::new();
    };
    let prefix = format!("{alias}/");
    let mut findings = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some((job, flow)) = invocation_job(line, &prefix) else {
            continue;
        };
        let Some(params) = schema.get(job) else {
            findings.push(finding(job, i, None, FindingKind::UnknownJob));
            continue;
        };
        // Arguments given as a flow mapping, through `matrix:` or a `<<:` merge
        // are not visible as block keys, so completeness cannot be judged.
        let mut opaque = flow;
        let mut supplied = Vec::new();
        for (arg_i, key) in invocation_args(&lines, i) {
            if key == "matrix" || key == "<<" {
                opaque = true;
                continue;
            }
            if RESERVED_KEYS.contains(&key) {
                continue;
            }
            supplied.push(key);
            if !flow && !params.contains_key(key) {
                findings.push(finding(
                    job,
                    i,
                    Some(arg_i),
                    FindingKind::UnexpectedArg(key.to_string()),
                ));
            }
        }
        for (param, required) in params {
            if !opaque && *required && !supplied.contains(&param.as_str()) {
                findings.push(finding(
                    job,
                    i,
                    None,
                    FindingKind::MissingRequiredArg(param.clone()),
                ));
            }
        }
    }
    findings
}

fn finding(job: &str, line_idx: usize, arg_idx: Option<usize>, kind: FindingKind) -> Finding {
    Finding {
        job: job.to_string(),
        line: line_idx + 1,
        arg_line: arg_idx.map(|i| i + 1),
        kind,
    }
}

/// The version the file pins the orb at.
pub fn orb_pin(content: &str) -> Option<String> {
    content.lines().find_map(|l| {
        let (_, source) = l.trim().split_once(':')?;
        let source = source.trim().trim_matches(['"', '\'']);
        let version = source.strip_prefix(ORB_SOURCE)?;
        Some(version.split('#').next()?.trim().to_string())
    })
}

/// The alias under which the file's `orbs:` map imports this orb.
fn orb_alias<'a>(lines: &[&'a str]) -> Option<&'a str> {
    lines.iter().find_map(|l| {
        let (alias, source) = l.trim().split_once(':')?;
        let source = source.trim().trim_matches(['"', '\'']);
        (source.starts_with(ORB_SOURCE) && !alias.contains(char::is_whitespace)).then_some(alias)
    })
}

/// The job name if `line` is a workflow entry `- <alias>/<job>` (with or
/// without a trailing colon), and whether its arguments follow inline as a
/// flow mapping.
fn invocation_job<'a>(line: &'a str, prefix: &str) -> Option<(&'a str, bool)> {
    let rest = line.trim_start().strip_prefix("- ")?.trim_start();
    let rest = rest.strip_prefix(prefix)?.split('#').next()?.trim();
    let (job, inline) = rest.split_once(':').unwrap_or((rest, ""));
    let job = job.trim();
    (!job.is_empty() && !job.contains(char::is_whitespace))
        .then_some((job, !inline.trim().is_empty()))
}

/// `(line index, key)` for each argument of the invocation at `entry`: the
/// keys at the first nested indent, ignoring deeper (value) lines.
fn invocation_args<'a>(lines: &[&'a str], entry: usize) -> Vec<(usize, &'a str)> {
    let entry_indent = indent(lines[entry]);
    let mut arg_indent = None;
    let mut args = Vec::new();
    for (i, line) in lines.iter().enumerate().skip(entry + 1) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let ind = indent(line);
        if ind <= entry_indent {
            break;
        }
        let arg_indent = *arg_indent.get_or_insert(ind);
        if ind == arg_indent {
            if let Some((key, _)) = trimmed.split_once(':') {
                args.push((i, key.trim()));
            }
        }
    }
    args
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Remove the arguments reported as [`FindingKind::UnexpectedArg`], with any
/// continuation lines, leaving every other line untouched.
pub fn strip_unexpected(content: &str, findings: &[Finding]) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut drop = vec![false; lines.len()];
    for arg_i in findings
        .iter()
        .filter(|f| matches!(f.kind, FindingKind::UnexpectedArg(_)))
        .filter_map(|f| f.arg_line)
        .map(|n| n - 1)
    {
        drop[arg_i] = true;
        let arg_indent = indent(lines[arg_i]);
        // Blank and comment lines belong to the value only when more of the
        // value follows them (block scalars may contain both).
        let mut pending = Vec::new();
        for (j, line) in lines.iter().enumerate().skip(arg_i + 1) {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                pending.push(j);
                continue;
            }
            let ind = indent(line);
            let same_indent_item = ind == arg_indent && trimmed.starts_with("- ");
            if ind <= arg_indent && !same_indent_item {
                break;
            }
            for k in pending.drain(..) {
                drop[k] = true;
            }
            drop[j] = true;
        }
    }
    let mut out: String = lines
        .iter()
        .zip(&drop)
        .filter(|(_, d)| !**d)
        .map(|(l, _)| format!("{l}\n"))
        .collect();
    if !content.ends_with('\n') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn schemas() -> BTreeMap<String, JobSchema> {
        schema().unwrap()
    }

    fn file(jobs: &str) -> String {
        format!(
            "version: 2.1\norbs:\n  gen-circleci-orb: jerus-org/gen-circleci-orb@0.1.22\n  toolkit: jerus-org/circleci-toolkit@8.0.0\nworkflows:\n  release:\n    jobs:\n{jobs}"
        )
    }

    #[test]
    fn schema_marks_params_without_default_as_required() {
        let s = schemas();
        assert_eq!(s["build_rust_binary"]["package"], true);
        assert_eq!(s["build_rust_binary"]["cargo_args"], false);
    }

    #[test]
    fn schema_matches_the_orb_source_jobs() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../orb/src/jobs");
        if !dir.is_dir() {
            return; // packaged crate: no workspace orb to compare with
        }
        for (name, src) in JOB_SOURCES {
            let orb = std::fs::read_to_string(dir.join(format!("{name}.yml"))).unwrap();
            assert_eq!(
                &orb, src,
                "orb-jobs/{name}.yml is stale; re-copy from orb/src/jobs"
            );
        }
    }

    #[test]
    fn flags_an_argument_the_job_no_longer_declares() {
        let f = file(
            "      - gen-circleci-orb/build_rust_binary:\n          name: build-binary\n          package: jci-audit\n          rust_image: old\n          requires: [approve-release]\n",
        );
        let found = validate(&f, &schemas());
        assert_eq!(
            found,
            vec![Finding {
                job: "build_rust_binary".into(),
                line: 8,
                arg_line: Some(11),
                kind: FindingKind::UnexpectedArg("rust_image".into()),
            }]
        );
    }

    #[test]
    fn flags_a_missing_required_argument() {
        let f = file("      - gen-circleci-orb/build_rust_binary:\n          name: build-binary\n");
        let found = validate(&f, &schemas());
        assert_eq!(
            found.iter().map(|x| x.kind.clone()).collect::<Vec<_>>(),
            vec![FindingKind::MissingRequiredArg("package".into())]
        );
    }

    #[test]
    fn flags_a_bare_invocation_missing_a_required_argument() {
        let f = file("      - gen-circleci-orb/build_rust_binary\n");
        let found = validate(&f, &schemas());
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].kind,
            FindingKind::MissingRequiredArg("package".into())
        );
    }

    #[test]
    fn flags_an_unknown_job() {
        let f = file("      - gen-circleci-orb/no_such_job:\n          name: x\n");
        let found = validate(&f, &schemas());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, FindingKind::UnknownJob);
    }

    #[test]
    fn reserved_circleci_keys_are_not_arguments() {
        let f = file(
            "      - gen-circleci-orb/build_rust_binary:\n          name: b\n          package: p\n          requires: [a]\n          context:\n            - release\n          filters:\n            branches:\n              only: main\n          pre-steps:\n            - run: echo hi\n",
        );
        assert_eq!(validate(&f, &schemas()), Vec::new());
    }

    #[test]
    fn a_valid_invocation_passes() {
        let f = file("      - gen-circleci-orb/build_rust_binary:\n          package: p\n          cargo_args: --locked\n");
        assert_eq!(validate(&f, &schemas()), Vec::new());
    }

    #[test]
    fn other_orbs_jobs_are_ignored() {
        let f = file("      - toolkit/release_crate:\n          bogus: 1\n");
        assert_eq!(validate(&f, &schemas()), Vec::new());
    }

    #[test]
    fn a_file_without_the_orb_is_ignored() {
        let f = "version: 2.1\nworkflows:\n  w:\n    jobs:\n      - gen-circleci-orb/build_rust_binary:\n          bogus: 1\n";
        assert_eq!(validate(f, &schemas()), Vec::new());
    }

    #[test]
    fn the_orb_alias_is_read_from_the_file() {
        let f = "orbs:\n  gco: jerus-org/gen-circleci-orb@0.1.22\nworkflows:\n  w:\n    jobs:\n      - gco/build_rust_binary:\n          package: p\n          rust_image: x\n";
        let found = validate(f, &schemas());
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].kind,
            FindingKind::UnexpectedArg("rust_image".into())
        );
    }

    #[test]
    fn strip_removes_only_the_stale_argument_line() {
        let f = file(
            "      # keep me\n      - gen-circleci-orb/build_rust_binary:\n          name: build-binary\n          package: jci-audit\n          rust_image: old # trailing\n          requires: [approve-release]\n",
        );
        let out = strip_unexpected(&f, &validate(&f, &schemas()));
        assert_eq!(
            out,
            file("      # keep me\n      - gen-circleci-orb/build_rust_binary:\n          name: build-binary\n          package: jci-audit\n          requires: [approve-release]\n")
        );
    }

    #[test]
    fn strip_removes_continuation_lines_of_a_stale_argument() {
        let f = file(
            "      - gen-circleci-orb/build_rust_binary:\n          package: p\n          rust_image:\n            - a\n            - b\n          cargo_args: x\n",
        );
        let out = strip_unexpected(&f, &validate(&f, &schemas()));
        assert_eq!(
            out,
            file("      - gen-circleci-orb/build_rust_binary:\n          package: p\n          cargo_args: x\n")
        );
    }

    #[test]
    fn strip_is_idempotent() {
        let f = file("      - gen-circleci-orb/build_rust_binary:\n          package: p\n          rust_image: old\n");
        let once = strip_unexpected(&f, &validate(&f, &schemas()));
        assert_eq!(validate(&once, &schemas()), Vec::new());
        assert_eq!(strip_unexpected(&once, &validate(&once, &schemas())), once);
    }

    #[test]
    fn strip_removes_a_block_scalar_value_containing_blank_and_comment_lines() {
        let f = file(
            "      - gen-circleci-orb/build_rust_binary:\n          package: p\n          rust_image: |\n            line one\n\n            # not a yaml comment\n            line two\n          cargo_args: x\n",
        );
        let out = strip_unexpected(&f, &validate(&f, &schemas()));
        assert_eq!(
            out,
            file("      - gen-circleci-orb/build_rust_binary:\n          package: p\n          cargo_args: x\n")
        );
    }

    #[test]
    fn strip_removes_a_sequence_written_at_the_keys_own_indent() {
        let f = file(
            "      - gen-circleci-orb/build_rust_binary:\n          package: p\n          rust_image:\n          - a\n          - b\n          cargo_args: x\n",
        );
        let out = strip_unexpected(&f, &validate(&f, &schemas()));
        assert_eq!(
            out,
            file("      - gen-circleci-orb/build_rust_binary:\n          package: p\n          cargo_args: x\n")
        );
    }

    #[test]
    fn strip_keeps_a_comment_that_precedes_the_next_argument() {
        let f = file(
            "      - gen-circleci-orb/build_rust_binary:\n          package: p\n          rust_image: old\n          # about cargo_args\n          cargo_args: x\n",
        );
        let out = strip_unexpected(&f, &validate(&f, &schemas()));
        assert_eq!(
            out,
            file("      - gen-circleci-orb/build_rust_binary:\n          package: p\n          # about cargo_args\n          cargo_args: x\n")
        );
    }

    #[test]
    fn a_flow_mapping_invocation_is_not_reported_missing_arguments() {
        let f = file("      - gen-circleci-orb/build_rust_binary: {package: p}\n");
        assert_eq!(validate(&f, &schemas()), Vec::new());
    }

    #[test]
    fn matrix_and_merge_keys_suppress_the_missing_argument_check() {
        let matrix = file(
            "      - gen-circleci-orb/build_rust_binary:\n          matrix:\n            parameters:\n              package: [a, b]\n",
        );
        assert_eq!(validate(&matrix, &schemas()), Vec::new());
        let merge = file("      - gen-circleci-orb/build_rust_binary:\n          <<: *defaults\n");
        assert_eq!(validate(&merge, &schemas()), Vec::new());
    }

    #[test]
    fn the_orb_pin_is_read_from_the_import() {
        let f = file("      - gen-circleci-orb/build_rust_binary\n");
        assert_eq!(orb_pin(&f).as_deref(), Some("0.1.22"));
        assert_eq!(orb_pin("version: 2.1\n"), None);
    }

    #[test]
    fn every_orb_job_file_is_in_the_embedded_schema() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../orb/src/jobs");
        if !dir.is_dir() {
            return;
        }
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_stem().unwrap().to_str().unwrap().to_string();
            assert!(
                JOB_SOURCES.iter().any(|(n, _)| *n == name),
                "orb job '{name}' is missing from JOB_SOURCES (add it and copy to orb-jobs/)"
            );
        }
    }
}
