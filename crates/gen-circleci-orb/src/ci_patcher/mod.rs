use anyhow::Result;

pub struct PatchOpts {
    pub binary: String,
    /// One or more CircleCI namespaces to publish the orb under.
    /// Each namespace gets its own `gen-circleci-orb/ensure_orb_registered` workflow step and
    /// `orb-tools/publish: name: publish-orb-<ns>` workflow step.
    pub namespaces: Vec<String>,
    pub docker_namespace: String,
    pub orb_dir: String,
    pub build_workflow: String,
    pub release_workflow: String,
    pub requires_job: Option<String>,
    /// The tag prefix used by `toolkit/release_crate` (e.g. `"gen-orb-mcp-v"`).
    /// Used to filter the `orb-release:` workflow trigger and to strip the prefix
    /// when normalising `CIRCLE_TAG` for `orb-tools/publish`.
    pub crate_tag_prefix: String,
    pub release_after_job: String,
    pub orb_tools_version: String,
    pub docker_orb_version: String,
    pub docker_context: String,
    pub orb_context: String,
    /// Namespaces that should be registered as private orbs.
    /// A namespace listed here gets `private: true` in its `ensure_orb_registered` step.
    /// Visibility is set at orb creation time and cannot be changed afterwards.
    pub private_namespaces: Vec<String>,
    /// Version of jerus-org/gen-circleci-orb orb to pin in the orbs section.
    pub gen_circleci_orb_version: String,
    pub mcp: bool,
    /// Earliest orb version to include when priming prior-version snapshots.
    /// Passed to `gen-circleci-orb/build_mcp_server` as `earliest_version`.
    /// Only used when `mcp` is true.
    pub mcp_earliest_version: String,
    /// CircleCI context providing push authority for MCP server build + publish + save steps.
    /// Only used when `mcp` is true.
    pub mcp_context: Vec<String>,
    /// CircleCI context(s) the regenerate-orb job attaches when auto-record is
    /// enabled, supplying the signing material. Empty when auto-record is
    /// disabled (no context attached, and no end push job wired).
    pub record_contexts: Vec<String>,
    /// SSH key fingerprint (a public-key hash, not a secret) for the end-of-workflow
    /// push job. When non-empty, the push job loads this write key (and drops the
    /// read-only checkout key). Empty falls back to the ambient environment
    /// credentials (the push then fails on a read-only key, with guidance).
    pub record_push_ssh_fingerprint: String,
}

pub struct PatchReport {
    pub insertions: Vec<String>,
    pub skipped: Vec<String>,
}

/// Patch a build/validation CircleCI config string.
/// Returns the modified content and a report of what was changed or skipped.
pub fn patch_build(content: &str, opts: &PatchOpts) -> (String, PatchReport) {
    let mut report = PatchReport {
        insertions: vec![],
        skipped: vec![],
    };
    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();

    patch_step0_gen_circleci_orb_orb(content, &mut lines, opts, &mut report);
    patch_step1_orb_tools(content, &mut lines, opts, &mut report);
    patch_step2_build_regen_jobs(content, &mut lines, opts, &mut report);
    patch_step3_pack_validate(content, &mut lines, opts, &mut report);
    patch_step5_orb_release_workflow(content, &mut lines, opts, &mut report);

    let mut output = lines.join("\n");
    if content.ends_with('\n') {
        output.push('\n');
    }
    (output, report)
}

fn insert_block_at(lines: &mut Vec<String>, pos: usize, block: &[String]) {
    for (i, l) in block.iter().enumerate() {
        lines.insert(pos + i, l.clone());
    }
}

fn patch_step0_gen_circleci_orb_orb(
    content: &str,
    lines: &mut Vec<String>,
    opts: &PatchOpts,
    report: &mut PatchReport,
) {
    let version = &opts.gen_circleci_orb_version;
    let orb_entry = format!("  gen-circleci-orb: jerus-org/gen-circleci-orb@{version}");
    if content.contains("gen-circleci-orb:") {
        report.skipped.push("gen-circleci-orb orb".to_string());
    } else if let Some(pos) = find_section_end(lines, "orbs:") {
        lines.insert(pos, orb_entry);
        report.insertions.push("gen-circleci-orb orb".to_string());
    }
}

fn patch_step1_orb_tools(
    content: &str,
    lines: &mut Vec<String>,
    opts: &PatchOpts,
    report: &mut PatchReport,
) {
    let orb_entry = format!("  orb-tools: circleci/orb-tools@{}", opts.orb_tools_version);
    if content.contains("orb-tools:") {
        report.skipped.push("orb-tools orb".to_string());
    } else if let Some(pos) = find_section_end(lines, "orbs:") {
        lines.insert(pos, orb_entry);
        report.insertions.push("orb-tools orb".to_string());
    }
}

fn patch_step2_build_regen_jobs(
    content: &str,
    _lines: &mut Vec<String>,
    _opts: &PatchOpts,
    report: &mut PatchReport,
) {
    // Detect either the old inline-job approach or the orb-reference approach.
    // Both are considered "already present" for idempotency — no inline job defs are
    // added any more; the workflow steps reference gen-circleci-orb orb jobs directly.
    let has_build = content.contains("build-binary:")
        || content.contains("gen-circleci-orb/build_rust_binary:");
    let has_regen =
        content.contains("regenerate-orb:") || content.contains("gen-circleci-orb/generate:");
    if has_build && has_regen {
        report
            .skipped
            .push("build-binary and regenerate-orb jobs".to_string());
    } else {
        // Nothing to insert — the workflow steps added by patch_step3 now reference
        // gen-circleci-orb orb jobs directly so no inline job definitions are needed.
        report
            .insertions
            .push("build-binary and regenerate-orb jobs".to_string());
    }
}

fn patch_step3_pack_validate(
    content: &str,
    lines: &mut Vec<String>,
    opts: &PatchOpts,
    report: &mut PatchReport,
) {
    if content.contains("orb-tools/pack:") {
        report.skipped.push("workflow steps".to_string());
        return;
    }
    let step_block = pack_validate_steps(opts);
    if let Some(pos) = find_workflow_jobs_end(lines, &opts.build_workflow) {
        insert_block_at(lines, pos, &step_block);
    }
    report.insertions.push("workflow steps".to_string());
}

fn patch_step5_orb_release_workflow(
    content: &str,
    lines: &mut Vec<String>,
    opts: &PatchOpts,
    report: &mut PatchReport,
) {
    if content.contains("  orb-release:") {
        report.skipped.push("orb-release workflow".to_string());
        return;
    }
    let wf_block = orb_release_workflow_section(opts);
    if let Some(pos) = find_section_end(lines, "workflows:") {
        insert_block_at(lines, pos, &wf_block);
    }
    report.insertions.push("orb-release workflow".to_string());
}

/// Patch a release CircleCI config string.
///
/// The orb release pipeline (Docker build, orb pack, orb publish) is now wired into
/// `config.yml` as a tag-triggered `orb-release:` workflow by `patch_build`. Nothing
/// needs to be added to `release.yml`, so this function is a no-op.
pub fn patch_release(content: &str, _opts: &PatchOpts) -> (String, PatchReport) {
    let report = PatchReport {
        insertions: vec![],
        skipped: vec![],
    };
    (content.to_string(), report)
}

/// Apply patches to CI config files on disk (or dry-run).
pub fn apply_patches(
    ci_dir: &std::path::Path,
    opts: &PatchOpts,
    dry_run: bool,
) -> Result<Vec<String>> {
    let mut summary = vec![];

    for (filename, patch_fn) in &[
        (
            "config.yml",
            patch_build as fn(&str, &PatchOpts) -> (String, PatchReport),
        ),
        (
            "release.yml",
            patch_release as fn(&str, &PatchOpts) -> (String, PatchReport),
        ),
    ] {
        let path = ci_dir.join(filename);
        if !path.exists() {
            summary.push(format!("{filename}: not found, skipped"));
            continue;
        }
        let content = std::fs::read_to_string(&path)?;
        let (patched, report) = patch_fn(&content, opts);

        for ins in &report.insertions {
            summary.push(format!("{filename}: inserted {ins}"));
        }
        for sk in &report.skipped {
            summary.push(format!("{filename}: skipped {sk} (already present)"));
        }

        if !dry_run && patched != content {
            std::fs::write(&path, &patched)?;
        }
    }

    Ok(summary)
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn find_top_level(lines: &[String], header: &str) -> Option<usize> {
    lines.iter().position(|l| l.trim_end() == header)
}

/// Find the line index where new entries should be inserted inside a top-level section.
/// Returns the index of the first top-level line *after* the section header.
fn find_section_end(lines: &[String], header: &str) -> Option<usize> {
    let start = find_top_level(lines, header)?;
    for (i, l) in lines.iter().enumerate().skip(start + 1) {
        if !l.is_empty() && !l.starts_with(' ') && !l.starts_with('\t') && !l.starts_with('#') {
            return Some(i);
        }
    }
    Some(lines.len())
}

/// Find the insertion point at the end of a named workflow's `jobs:` list.
fn find_workflow_jobs_end(lines: &[String], workflow: &str) -> Option<usize> {
    let wf_line = format!("  {workflow}:");
    let wf_idx = lines
        .iter()
        .position(|l| l.trim_end() == wf_line.trim_end())?;

    // Find `    jobs:` within this workflow
    let mut jobs_idx = None;
    for (i, line) in lines.iter().enumerate().skip(wf_idx + 1) {
        let l = line.trim_end();
        if !line.starts_with("  ") {
            break;
        }
        if l == "    jobs:" || l == "  jobs:" {
            jobs_idx = Some(i);
            break;
        }
    }
    let jobs_start = jobs_idx?;

    // Scan forward to find where the jobs list ends
    for (i, l) in lines.iter().enumerate().skip(jobs_start + 1) {
        if l.trim_end().is_empty() {
            continue;
        }
        // Jobs entries are indented 6+ spaces; anything less ends the block
        let indent = l.len() - l.trim_start().len();
        if indent <= 2 {
            return Some(i);
        }
    }
    Some(lines.len())
}

fn pack_validate_steps(opts: &PatchOpts) -> Vec<String> {
    let orb_dir = &opts.orb_dir;
    let binary = &opts.binary;
    let mut steps = vec![];

    // gen-circleci-orb/build_rust_binary — compiles the binary and persists to workspace
    steps.push("      - gen-circleci-orb/build_rust_binary:".to_string());
    steps.push("          name: build-binary".to_string());
    steps.push(format!("          package: {binary}"));
    if let Some(req) = &opts.requires_job {
        steps.push(format!("          requires: [{req}]"));
    }

    // gen-circleci-orb/generate — attaches workspace, runs gen-circleci-orb generate
    steps.push("      - gen-circleci-orb/generate:".to_string());
    steps.push("          name: regenerate-orb".to_string());
    steps.push(format!("          binary: {binary}"));
    for ns in &opts.namespaces {
        steps.push(format!("          orb_namespace: {ns}"));
    }
    steps.push(format!("          orb_dir: {orb_dir}"));
    steps.push("          attach_workspace: true".to_string());
    // Model B: the binary does NOT push here. It regenerates the orb
    // (no_record) and persists it to the workspace, so pack/review validate the
    // *regenerated* orb and the end-of-workflow push job commits + pushes it
    // once, after validation. Without no_record the binary would attempt an
    // ambient push that the read-only checkout key rejects (breaking CI).
    steps.push("          no_record: true".to_string());
    steps.push("          persist_orb_workspace: true".to_string());
    // Record context(s) remain attached for now; relocating signing to the
    // end-of-workflow push job is handled separately. Harmless under no_record.
    if !opts.record_contexts.is_empty() {
        steps.push(format!(
            "          context: [{}]",
            opts.record_contexts.join(", ")
        ));
    }
    steps.push("          requires: [build-binary]".to_string());
    // The orb regen/validate/push chain is pointless on `main` (can't push, and
    // the orb-release verify gate covers publish-time drift). Run on PR branches
    // only; the regen still validates on forked PRs (no push there).
    push_branch_ignore(&mut steps, &["main"]);

    // orb-tools/pack — checkout:false + attach the regenerated orb from the
    // workspace (persisted by regenerate-orb), so the packed/validated orb is
    // exactly what was just generated, not the (possibly stale) committed copy.
    steps.push("      - orb-tools/pack:".to_string());
    steps.push("          name: pack-orb".to_string());
    steps.push("          checkout: false".to_string());
    steps.push(format!("          source_dir: {orb_dir}/src"));
    steps.push("          pre-steps:".to_string());
    steps.push("            - attach_workspace:".to_string());
    steps.push("                at: .".to_string());
    steps.push("          requires: [regenerate-orb]".to_string());
    push_branch_ignore(&mut steps, &["main"]);

    // orb-tools/review (best-practice review of the regenerated, packed orb)
    steps.push("      - orb-tools/review:".to_string());
    steps.push("          name: review-orb".to_string());
    steps.push("          checkout: false".to_string());
    steps.push(format!("          source_dir: {orb_dir}/src"));
    steps.push("          pre-steps:".to_string());
    steps.push("            - attach_workspace:".to_string());
    steps.push("                at: .".to_string());
    steps.push("          requires: [pack-orb]".to_string());
    push_branch_ignore(&mut steps, &["main"]);

    // End-of-workflow push job (only when auto-record is enabled). Runs generate
    // WITH record (regenerate + signed commit + push) gated on the orb validations
    // (pack/review) → the PR branch only receives a validated orb. When a write-key
    // fingerprint is configured it is loaded (and the read-only checkout key dropped)
    // so the push has write authority; otherwise the push uses the ambient
    // credentials and fails with guidance (a read-only checkout key can't push).
    if !opts.record_contexts.is_empty() {
        steps.push("      - gen-circleci-orb/generate:".to_string());
        steps.push("          name: push-orb".to_string());
        steps.push(format!("          binary: {binary}"));
        for ns in &opts.namespaces {
            steps.push(format!("          orb_namespace: {ns}"));
        }
        steps.push(format!("          orb_dir: {orb_dir}"));
        steps.push("          attach_workspace: true".to_string());
        if !opts.record_push_ssh_fingerprint.is_empty() {
            steps.push(format!(
                "          ssh_fingerprint: \"{}\"",
                opts.record_push_ssh_fingerprint
            ));
        }
        steps.push(format!(
            "          context: [{}]",
            opts.record_contexts.join(", ")
        ));
        steps.push("          requires: [pack-orb, review-orb]".to_string());
        // push-orb additionally skips forked PRs (no write access to the fork);
        // it commits the regen to the same-repo PR branch, which reaches main via
        // the merge. The pcu App authorizes the push, so it must NOT run on main.
        push_branch_ignore(&mut steps, &["main", "/pull\\/[0-9]+/"]);
    }

    steps
}

// ── orb-release helpers (tag-triggered, lives in config.yml) ─────────────────

fn push_tag_filters(lines: &mut Vec<String>, only_tag: &str, ignore_branches: &str) {
    lines.push("          filters:".to_string());
    lines.push("            tags:".to_string());
    lines.push(only_tag.to_string());
    lines.push("            branches:".to_string());
    lines.push(ignore_branches.to_string());
}

/// Append a `filters: branches: ignore:` block (block sequence — never an inline
/// `[a, b]` flow sequence, whose `\` in `/pull\/...` is invalid YAML).
fn push_branch_ignore(steps: &mut Vec<String>, branches: &[&str]) {
    steps.push("          filters:".to_string());
    steps.push("            branches:".to_string());
    steps.push("              ignore:".to_string());
    for b in branches {
        steps.push(format!("                - {b}"));
    }
}

/// Generate the complete `orb-release:` workflow section for config.yml.
/// Uses orb job references (gen-circleci-orb/build_rust_binary, build_container,
/// ensure_orb_registered) instead of inline job definitions.
fn orb_release_workflow_section(opts: &PatchOpts) -> Vec<String> {
    let prefix = &opts.crate_tag_prefix;
    let orb_dir = &opts.orb_dir;
    let docker_ns = &opts.docker_namespace;
    let docker_ctx = &opts.docker_context;
    let orb_ctx = &opts.orb_context;
    let binary = &opts.binary;

    let only_tag = format!("              only: /^{prefix}.*/");
    let ignore_branches = "              ignore: /.*/".to_string();

    let mut lines = vec![
        String::new(),
        "  orb-release:".to_string(),
        "    jobs:".to_string(),
        "      - gen-circleci-orb/build_rust_binary:".to_string(),
        "          name: orb-release-binary".to_string(),
        format!("          package: {binary}"),
    ];
    push_tag_filters(&mut lines, &only_tag, &ignore_branches);
    lines.push(String::new());

    // Verify gate: regenerate the orb with the freshly-built binary and fail if
    // the committed orb is out of sync (generate --check). pack and container —
    // and therefore publish — require it, so a drifted or hand-edited orb is
    // never packed or published.
    lines.push("      - gen-circleci-orb/generate:".to_string());
    lines.push("          name: verify-orb".to_string());
    lines.push(format!("          binary: {binary}"));
    for ns in &opts.namespaces {
        lines.push(format!("          orb_namespace: {ns}"));
    }
    lines.push(format!("          orb_dir: {orb_dir}"));
    lines.push("          attach_workspace: true".to_string());
    lines.push("          check: true".to_string());
    lines.push("          requires: [orb-release-binary]".to_string());
    push_tag_filters(&mut lines, &only_tag, &ignore_branches);
    lines.push(String::new());

    // orb-tools/pack (checkout: false + pre-steps for version injection)
    lines.push("      - orb-tools/pack:".to_string());
    lines.push("          name: orb-release-pack".to_string());
    lines.push("          checkout: false".to_string());
    lines.push(format!("          source_dir: {orb_dir}/src"));
    lines.push("          requires: [verify-orb]".to_string());
    lines.push("          pre-steps:".to_string());
    lines.push("            - checkout".to_string());
    lines.push("            - run:".to_string());
    lines
        .push("                name: Inject release version into executor default tag".to_string());
    lines.push("                command: |".to_string());
    lines.push(format!(
        "                  VERSION=${{CIRCLE_TAG#{prefix}}}"
    ));
    lines.push("                  echo \"Injecting version: ${VERSION}\"".to_string());
    lines.push(format!("                  sed -i \"s/default: latest/default: ${{VERSION}}/\" {orb_dir}/src/executors/default.yml"));
    lines.push("                  echo \"Updated executor:\"".to_string());
    lines.push(format!(
        "                  cat {orb_dir}/src/executors/default.yml"
    ));
    push_tag_filters(&mut lines, &only_tag, &ignore_branches);
    lines.push(String::new());

    // gen-circleci-orb/build_container — Docker build + push via orb job
    lines.push("      - gen-circleci-orb/build_container:".to_string());
    lines.push("          name: orb-release-container".to_string());
    lines.push(format!("          binary: {binary}"));
    lines.push(format!("          docker_namespace: {docker_ns}"));
    lines.push(format!("          orb_dir: {orb_dir}"));
    lines.push(format!("          crate_tag_prefix: {prefix}"));
    lines.push("          requires: [verify-orb]".to_string());
    lines.push(format!("          context: [{docker_ctx}]"));
    push_tag_filters(&mut lines, &only_tag, &ignore_branches);
    lines.push(String::new());

    // Per-namespace: ensure_orb_registered + publish
    for ns in &opts.namespaces {
        let is_private = opts.private_namespaces.contains(ns);
        lines.push("      - gen-circleci-orb/ensure_orb_registered:".to_string());
        lines.push(format!(
            "          name: orb-release-ensure-registered-{ns}"
        ));
        lines.push(format!("          orb_name: {ns}/{binary}"));
        if is_private {
            lines.push("          private: true".to_string());
        }
        lines.push(format!("          context: [{orb_ctx}]"));
        push_tag_filters(&mut lines, &only_tag, &ignore_branches);
        lines.push(String::new());

        lines.push("      - orb-tools/publish:".to_string());
        lines.push(format!("          name: publish-orb-{ns}"));
        lines.push("          pre-steps:".to_string());
        lines.push("            - run:".to_string());
        lines.push("                name: Normalize CIRCLE_TAG for orb version".to_string());
        lines.push("                command: |".to_string());
        lines.push(format!(
            "                  VERSION=${{CIRCLE_TAG#{prefix}}}"
        ));
        lines.push(
            "                  echo \"export CIRCLE_TAG=v${VERSION}\" >> \"$BASH_ENV\"".to_string(),
        );
        lines.push(format!("          orb_name: {ns}/{binary}"));
        lines.push("          pub_type: production".to_string());
        lines.push("          vcs_type: github".to_string());
        lines.push("          enable_pr_comment: false".to_string());
        lines.push(format!("          requires: [orb-release-container, orb-release-pack, orb-release-ensure-registered-{ns}]"));
        lines.push(format!("          context: [{orb_ctx}]"));
        push_tag_filters(&mut lines, &only_tag, &ignore_branches);
        if ns != opts.namespaces.last().unwrap() {
            lines.push(String::new());
        }
    }

    if opts.mcp {
        lines.push(String::new());
        push_mcp_workflow_steps(&mut lines, opts, &only_tag, &ignore_branches);
    }

    lines
}

fn push_mcp_workflow_steps(
    lines: &mut Vec<String>,
    opts: &PatchOpts,
    only_tag: &str,
    ignore_branches: &str,
) {
    let binary = &opts.binary;
    let orb_dir = &opts.orb_dir;
    let prefix = &opts.crate_tag_prefix;
    let mcp_ctx_str = opts.mcp_context.join(", ");
    let earliest = &opts.mcp_earliest_version;

    // Build the requires list from all publish-orb-<ns> steps
    let requires: Vec<String> = opts
        .namespaces
        .iter()
        .map(|ns| format!("publish-orb-{ns}"))
        .collect();
    let requires_str = requires.join(", ");

    // gen-circleci-orb/build_mcp_server — primes, generates, compiles, publishes, saves
    lines.push("      - gen-circleci-orb/build_mcp_server:".to_string());
    lines.push("          name: build-mcp-server".to_string());
    lines.push(format!("          binary_name: {binary}"));
    lines.push(format!("          tag_prefix: {prefix}"));
    lines.push(format!("          orb_path: {orb_dir}/src/@orb.yml"));
    lines.push(format!("          earliest_version: \"{earliest}\""));
    lines.push(format!("          requires: [{requires_str}]"));
    lines.push(format!("          context: [{mcp_ctx_str}]"));
    push_tag_filters(lines, only_tag, ignore_branches);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_opts() -> PatchOpts {
        PatchOpts {
            binary: "mytool".to_string(),
            namespaces: vec!["my-org".to_string()],
            docker_namespace: "my-docker-org".to_string(),
            orb_dir: "orb".to_string(),
            build_workflow: "validation".to_string(),
            release_workflow: "release".to_string(),
            requires_job: Some("common-tests".to_string()),
            crate_tag_prefix: "mytool-v".to_string(),
            release_after_job: "approve-release".to_string(),
            orb_tools_version: "12.3.3".to_string(),
            docker_orb_version: "3.0.1".to_string(),
            docker_context: "docker".to_string(),
            orb_context: "orb-publishing".to_string(),
            private_namespaces: vec![],
            gen_circleci_orb_version: "0.0.1".to_string(),
            mcp: false,
            mcp_earliest_version: "1.0.0".to_string(),
            mcp_context: vec!["pcu-app".to_string()],
            record_contexts: vec![],
            record_push_ssh_fingerprint: String::new(),
        }
    }

    fn make_opts_mcp() -> PatchOpts {
        PatchOpts {
            mcp: true,
            ..make_opts()
        }
    }

    fn make_opts_two_ns() -> PatchOpts {
        PatchOpts {
            namespaces: vec!["my-org".to_string(), "other-org".to_string()],
            ..make_opts()
        }
    }

    // ── auto-record context wiring on regenerate-orb ──────────────────────────

    #[test]
    fn regenerate_orb_gets_record_context_when_enabled() {
        let opts = PatchOpts {
            record_contexts: vec!["release".to_string()],
            ..make_opts()
        };
        let steps = pack_validate_steps(&opts).join("\n");
        assert!(
            steps.contains("name: regenerate-orb"),
            "regenerate-orb job must be present"
        );
        assert!(
            steps.contains("context: [release]"),
            "regenerate-orb must attach the record context:\n{steps}"
        );
    }

    #[test]
    fn model_b_regenerate_persists_and_pack_review_attach() {
        // Model B: regenerate-orb must not push (no_record) but must persist the
        // regenerated orb to the workspace; pack/review must consume it from the
        // workspace (checkout:false + attach_workspace) so they validate the
        // regenerated orb rather than the committed (possibly stale) copy.
        let steps = pack_validate_steps(&make_opts()).join("\n");
        assert!(
            steps.contains("no_record: true"),
            "regenerate-orb must run with no_record (defer the push):\n{steps}"
        );
        assert!(
            steps.contains("persist_orb_workspace: true"),
            "regenerate-orb must persist the orb dir to the workspace:\n{steps}"
        );
        // Both pack and review must attach the workspace with checkout disabled.
        assert_eq!(
            steps.matches("checkout: false").count(),
            2,
            "pack-orb and review-orb must both set checkout: false:\n{steps}"
        );
        assert_eq!(
            steps.matches("- attach_workspace:").count(),
            2,
            "pack-orb and review-orb must both attach the workspace:\n{steps}"
        );
    }

    #[test]
    fn end_push_job_wired_after_validation_when_record_enabled() {
        let opts = PatchOpts {
            record_contexts: vec!["release".to_string()],
            record_push_ssh_fingerprint: "SHA256:test".to_string(),
            ..make_opts()
        };
        let steps = pack_validate_steps(&opts).join("\n");
        assert!(
            steps.contains("name: push-orb"),
            "end push job must be present when record is enabled:\n{steps}"
        );
        assert!(
            steps.contains("ssh_fingerprint: \"SHA256:test\""),
            "push job must pass the configured write-key fingerprint:\n{steps}"
        );
        assert!(
            steps.contains("requires: [pack-orb, review-orb]"),
            "the push must be gated on the orb validations (atomic):\n{steps}"
        );
        // push-orb must record (commit + push) — it must NOT carry no_record.
        let push = &steps[steps.find("name: push-orb").unwrap()..];
        assert!(
            !push.contains("no_record"),
            "push-orb must record (no no_record):\n{push}"
        );
    }

    #[test]
    fn end_push_job_omits_fingerprint_when_unset_and_absent_without_record() {
        // Fingerprint is optional: present push job, no ssh_fingerprint line.
        let with_record = PatchOpts {
            record_contexts: vec!["release".to_string()],
            ..make_opts()
        };
        let s1 = pack_validate_steps(&with_record).join("\n");
        assert!(s1.contains("name: push-orb"), "push job present:\n{s1}");
        assert!(
            !s1.contains("ssh_fingerprint:"),
            "no ssh_fingerprint line when unset (ambient fallback):\n{s1}"
        );
        // No record configured → no end push job at all.
        let s2 = pack_validate_steps(&make_opts()).join("\n");
        assert!(
            !s2.contains("name: push-orb"),
            "no end push job when record is disabled:\n{s2}"
        );
    }

    #[test]
    fn regenerate_orb_has_no_context_when_record_disabled() {
        let opts = make_opts(); // record_contexts empty
        let steps = pack_validate_steps(&opts).join("\n");
        // The validation workflow has no other context-bearing job, so any
        // `context:` line would be the record one leaking in.
        assert!(
            !steps.contains("context:"),
            "no context must be attached when auto-record is disabled:\n{steps}"
        );
    }

    // ── patch_release: now a no-op ────────────────────────────────────────────
    // All orb release wiring (jobs + workflow) moved to patch_build in config.yml.
    // The tag-triggered `orb-release:` workflow in config.yml replaces the
    // approval-triggered inline jobs that were previously added to release.yml.

    #[test]
    fn patch_release_is_noop() {
        let fixture = RELEASE_FIXTURE;
        let (output, report) = patch_release(fixture, &make_opts());
        assert_eq!(
            output, fixture,
            "patch_release must be a no-op: content must be unchanged"
        );
        assert!(
            report.insertions.is_empty(),
            "patch_release must report no insertions: {:?}",
            report.insertions
        );
        assert!(
            report.skipped.is_empty(),
            "patch_release must report nothing skipped: {:?}",
            report.skipped
        );
    }

    // ── patch_build: orb-release jobs in config.yml ───────────────────────────

    // ── gen-circleci-orb orb entry ────────────────────────────────────────────

    #[test]
    fn patch_build_adds_gen_circleci_orb_to_orbs_section() {
        let (output, report) = patch_build(BUILD_FIXTURE, &make_opts());
        assert!(
            output.contains("gen-circleci-orb: jerus-org/gen-circleci-orb@0.0.1"),
            "missing gen-circleci-orb orb entry:\n{output}"
        );
        let orbs_pos = output.find("orbs:").expect("no orbs: section");
        let jobs_pos = output.find("\njobs:").expect("no jobs: section");
        let pos = output
            .find("gen-circleci-orb: jerus-org")
            .expect("no gen-circleci-orb entry");
        assert!(
            pos > orbs_pos && pos < jobs_pos,
            "gen-circleci-orb must be inside orbs section:\n{output}"
        );
        assert!(
            report
                .insertions
                .iter()
                .any(|s| s.contains("gen-circleci-orb orb")),
            "report missing gen-circleci-orb insertion"
        );
    }

    #[test]
    fn gen_circleci_orb_version_uses_opts_value() {
        let opts = PatchOpts {
            gen_circleci_orb_version: "9.8.7".to_string(),
            ..make_opts()
        };
        let (output, _) = patch_build(BUILD_FIXTURE, &opts);
        assert!(
            output.contains("gen-circleci-orb: jerus-org/gen-circleci-orb@9.8.7"),
            "gen-circleci-orb orb must use version from opts:\n{output}"
        );
    }

    #[test]
    fn gen_circleci_orb_orb_entry_is_idempotent() {
        let (first, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let (second, second_report) = patch_build(&first, &make_opts());
        assert_eq!(first, second, "second patch changed content");
        assert!(
            second_report
                .skipped
                .iter()
                .any(|s| s.contains("gen-circleci-orb orb")),
            "expected gen-circleci-orb orb skipped on second run:\n{:?}",
            second_report.skipped
        );
    }

    // ── orb-release workflow: orb job references (no inline job defs) ─────────

    #[test]
    fn no_inline_orb_release_jobs_in_jobs_section() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let jobs_end = output.find("\nworkflows:").expect("no workflows: section");
        let jobs_section = &output[..jobs_end];
        assert!(
            !jobs_section.contains("  orb-release-binary:"),
            "orb-release-binary must NOT be an inline job definition:\n{jobs_section}"
        );
        assert!(
            !jobs_section.contains("  orb-release-container:"),
            "orb-release-container must NOT be an inline job definition:\n{jobs_section}"
        );
        assert!(
            !jobs_section.contains("  orb-release-ensure-registered-"),
            "ensure-registered must NOT be an inline job definition:\n{jobs_section}"
        );
    }

    #[test]
    fn orb_release_workflow_uses_build_rust_binary_orb_job() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let after_wf = output
            .split("  orb-release:")
            .nth(1)
            .expect("no orb-release workflow");
        assert!(
            after_wf.contains("gen-circleci-orb/build_rust_binary:"),
            "orb-release must use gen-circleci-orb/build_rust_binary orb job:\n{after_wf}"
        );
        assert!(
            after_wf.contains("name: orb-release-binary"),
            "build_rust_binary step must be named orb-release-binary:\n{after_wf}"
        );
        assert!(
            after_wf.contains("package: mytool"),
            "build_rust_binary step must set package: mytool:\n{after_wf}"
        );
    }

    #[test]
    fn orb_release_workflow_uses_build_container_orb_job() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let after_wf = output
            .split("  orb-release:")
            .nth(1)
            .expect("no orb-release workflow");
        assert!(
            after_wf.contains("gen-circleci-orb/build_container:"),
            "orb-release must use gen-circleci-orb/build_container orb job:\n{after_wf}"
        );
        assert!(
            after_wf.contains("name: orb-release-container"),
            "build_container step must be named orb-release-container:\n{after_wf}"
        );
    }

    #[test]
    fn build_container_step_has_binary_docker_and_prefix_params() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let after_container = output
            .split("name: orb-release-container")
            .nth(1)
            .expect("no orb-release-container step");
        let step = after_container
            .split("\n      - ")
            .next()
            .unwrap_or(after_container);
        assert!(
            step.contains("binary: mytool"),
            "build_container step must pass binary param:\n{step}"
        );
        assert!(
            step.contains("docker_namespace: my-docker-org"),
            "build_container step must pass docker_namespace param:\n{step}"
        );
        assert!(
            step.contains("crate_tag_prefix: mytool-v"),
            "build_container step must pass crate_tag_prefix param:\n{step}"
        );
        assert!(
            step.contains("requires: [verify-orb]"),
            "build_container step must require the verify-orb gate:\n{step}"
        );
        assert!(
            step.contains("context: [docker]"),
            "build_container step must set docker context:\n{step}"
        );
    }

    #[test]
    fn orb_release_workflow_uses_ensure_orb_registered_orb_job() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let after_wf = output
            .split("  orb-release:")
            .nth(1)
            .expect("no orb-release workflow");
        assert!(
            after_wf.contains("gen-circleci-orb/ensure_orb_registered:"),
            "orb-release must use gen-circleci-orb/ensure_orb_registered orb job:\n{after_wf}"
        );
        assert!(
            after_wf.contains("name: orb-release-ensure-registered-my-org"),
            "ensure_orb_registered step must be named correctly:\n{after_wf}"
        );
    }

    #[test]
    fn ensure_orb_registered_step_has_orb_name() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let after_ensure = output
            .split("name: orb-release-ensure-registered-my-org")
            .nth(1)
            .expect("no ensure-registered step");
        let step = after_ensure
            .split("\n      - ")
            .next()
            .unwrap_or(after_ensure);
        assert!(
            step.contains("orb_name: my-org/mytool"),
            "ensure_orb_registered step must set orb_name:\n{step}"
        );
        assert!(
            step.contains("context: [orb-publishing]"),
            "ensure_orb_registered step must set orb context:\n{step}"
        );
    }

    #[test]
    fn ensure_orb_registered_step_sets_private_for_private_ns() {
        let opts = PatchOpts {
            namespaces: vec!["private-ns".to_string()],
            private_namespaces: vec!["private-ns".to_string()],
            ..make_opts()
        };
        let (output, _) = patch_build(BUILD_FIXTURE, &opts);
        let after_ensure = output
            .split("name: orb-release-ensure-registered-private-ns")
            .nth(1)
            .expect("no ensure-registered step for private-ns");
        let step = after_ensure
            .split("\n      - ")
            .next()
            .unwrap_or(after_ensure);
        assert!(
            step.contains("private: true"),
            "ensure_orb_registered step must set private: true for private namespace:\n{step}"
        );
    }

    #[test]
    fn ensure_orb_registered_step_omits_private_for_public_ns() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let after_ensure = output
            .split("name: orb-release-ensure-registered-my-org")
            .nth(1)
            .expect("no ensure-registered step");
        let step = after_ensure
            .split("\n      - ")
            .next()
            .unwrap_or(after_ensure);
        assert!(
            !step.contains("private:"),
            "public namespace ensure step must NOT set private:\n{step}"
        );
    }

    // (kept for backwards compat shape — now tests orb job refs in workflow)
    #[test]
    fn orb_release_binary_job_uses_cargo_build_with_package_flag() {
        // With orb jobs, the build logic lives in the orb; the workflow step
        // just passes the package parameter to gen-circleci-orb/build_rust_binary.
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let after_wf = output
            .split("  orb-release:")
            .nth(1)
            .expect("no orb-release workflow");
        assert!(
            after_wf.contains("package: mytool"),
            "build_rust_binary step must pass package: mytool:\n{after_wf}"
        );
        assert!(
            !after_wf.contains("cargo build --release -p mytool"),
            "inline cargo command must not appear in workflow YAML:\n{after_wf}"
        );
    }

    // (orb_release_binary_job_persists_binary_to_workspace: now lives in the orb script)

    // ── patch_build: orb-release workflow section ─────────────────────────────

    #[test]
    fn patch_build_adds_orb_release_workflow_section() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        assert!(
            output.contains("  orb-release:"),
            "orb-release workflow section missing:\n{output}"
        );
        // Must appear in the workflows section (after `workflows:`)
        let wf_pos = output.find("\nworkflows:").expect("no workflows: section");
        let orb_release_pos = output
            .find("  orb-release:")
            .expect("no orb-release workflow");
        assert!(
            orb_release_pos > wf_pos,
            "orb-release: must be inside the workflows: section:\n{output}"
        );
    }

    #[test]
    fn orb_release_workflow_steps_have_tag_filters() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        // After the orb-release workflow section starts, every step must have tag filters
        let after_wf = output
            .split("  orb-release:")
            .nth(1)
            .expect("no orb-release workflow");
        assert!(
            after_wf.contains("tags:"),
            "orb-release workflow steps must have tags: filter:\n{after_wf}"
        );
        assert!(
            after_wf.contains("/^mytool-v.*/"),
            "orb-release workflow must filter on crate_tag_prefix pattern:\n{after_wf}"
        );
    }

    #[test]
    fn orb_release_workflow_steps_have_branches_ignore_all() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let after_wf = output
            .split("  orb-release:")
            .nth(1)
            .expect("no orb-release workflow");
        assert!(
            after_wf.contains("branches:"),
            "orb-release workflow steps must have branches: filter:\n{after_wf}"
        );
        assert!(
            after_wf.contains("ignore: /.*/"),
            "orb-release workflow must ignore all branches (tag-triggered only):\n{after_wf}"
        );
    }

    #[test]
    fn orb_release_pack_uses_checkout_false_with_presteps() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let after_wf = output
            .split("  orb-release:")
            .nth(1)
            .expect("no orb-release workflow");
        assert!(
            after_wf.contains("checkout: false"),
            "orb-release pack step must use checkout: false so orb-tools owns the workspace contract:\n{after_wf}"
        );
        assert!(
            after_wf.contains("pre-steps:"),
            "orb-release pack must have pre-steps for checkout + version injection:\n{after_wf}"
        );
        // checkout must be a pre-step
        let after_pack = after_wf
            .split("orb-release-pack")
            .nth(1)
            .expect("no orb-release-pack step");
        assert!(
            after_pack
                .split("orb-tools/publish:")
                .next()
                .unwrap_or("")
                .contains("- checkout"),
            "checkout must appear in orb-release-pack pre-steps:\n{after_pack}"
        );
    }

    #[test]
    fn orb_release_pack_pre_steps_inject_version_into_executor() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let after_pack = output
            .split("orb-release-pack")
            .nth(1)
            .expect("no orb-release-pack step");
        assert!(
            after_pack.contains("sed -i"),
            "orb-release-pack pre-steps must inject version via sed:\n{after_pack}"
        );
        assert!(
            after_pack.contains("s/default: latest/default:"),
            "sed must replace 'default: latest' with the release version:\n{after_pack}"
        );
        assert!(
            after_pack.contains("executors/default.yml"),
            "sed must target the executor's default.yml:\n{after_pack}"
        );
        // Version is derived from CIRCLE_TAG by stripping the crate tag prefix
        assert!(
            after_pack.contains("CIRCLE_TAG#mytool-v"),
            "pack pre-steps must strip crate_tag_prefix from CIRCLE_TAG to get version:\n{after_pack}"
        );
    }

    #[test]
    fn orb_release_publish_normalizes_circle_tag() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let after_publish = output
            .split("      - orb-tools/publish:")
            .nth(1)
            .expect("no orb-tools/publish step in orb-release workflow");
        assert!(
            after_publish.contains("Normalize CIRCLE_TAG"),
            "publish pre-step must normalize CIRCLE_TAG for orb-tools/publish:\n{after_publish}"
        );
        // Strip the crate tag prefix; add 'v' for orb-tools version format
        assert!(
            after_publish.contains("CIRCLE_TAG=v${"),
            "publish must set CIRCLE_TAG with v prefix (orb-tools requires v-prefixed semver):\n{after_publish}"
        );
        assert!(
            after_publish.contains("CIRCLE_TAG#mytool-v"),
            "publish must strip crate_tag_prefix when normalising CIRCLE_TAG:\n{after_publish}"
        );
        assert!(
            after_publish.contains("BASH_ENV"),
            "publish must export CIRCLE_TAG via BASH_ENV:\n{after_publish}"
        );
    }

    #[test]
    fn orb_release_publish_has_enable_pr_comment_false() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let after_publish = output
            .split("      - orb-tools/publish:")
            .nth(1)
            .expect("no orb-tools/publish step");
        let publish_block = after_publish
            .split("\n      - ")
            .next()
            .unwrap_or(after_publish);
        assert!(
            publish_block.contains("enable_pr_comment: false"),
            "publish must set enable_pr_comment: false (no PR to comment on in tag-triggered pipeline):\n{publish_block}"
        );
    }

    #[test]
    fn orb_release_publish_requires_container_pack_and_ensure() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let after_publish = output
            .split("      - orb-tools/publish:")
            .nth(1)
            .expect("no orb-tools/publish step");
        let publish_block = after_publish
            .split("\n      - ")
            .next()
            .unwrap_or(after_publish);
        assert!(
            publish_block.contains("orb-release-container"),
            "publish must require orb-release-container:\n{publish_block}"
        );
        assert!(
            publish_block.contains("orb-release-pack"),
            "publish must require orb-release-pack:\n{publish_block}"
        );
        assert!(
            publish_block.contains("orb-release-ensure-registered-my-org"),
            "publish must require orb-release-ensure-registered-my-org:\n{publish_block}"
        );
    }

    #[test]
    fn validation_orb_chain_is_filtered_off_main() {
        let opts = PatchOpts {
            record_contexts: vec!["release".to_string()],
            ..make_opts()
        };
        let steps = pack_validate_steps(&opts).join("\n");
        let block = |name: &str| {
            steps
                .split(&format!("name: {name}"))
                .nth(1)
                .unwrap_or_else(|| panic!("job {name} not found:\n{steps}"))
                .split("\n      - ")
                .next()
                .unwrap()
                .to_string()
        };
        for job in ["regenerate-orb", "pack-orb", "review-orb", "push-orb"] {
            let b = block(job);
            assert!(
                b.contains("ignore:") && b.contains("- main"),
                "{job} must be filtered off main:\n{b}"
            );
        }
        // push-orb (which writes) additionally skips forked PRs.
        let push = block("push-orb");
        assert!(
            push.contains(r"- /pull\/[0-9]+/"),
            "push-orb must also skip forked PRs:\n{push}"
        );
    }

    #[test]
    fn orb_release_has_verify_gate_before_pack_and_container() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        // A verify-orb job runs `generate --check` (check: true) against the
        // freshly-built binary and gates the release.
        assert!(
            output.contains("name: verify-orb"),
            "orb-release must emit a verify-orb job:\n{output}"
        );
        let verify_block = output
            .split("name: verify-orb")
            .nth(1)
            .expect("no verify-orb job")
            .split("\n      - ")
            .next()
            .unwrap()
            .to_string();
        assert!(
            verify_block.contains("check: true"),
            "verify-orb must invoke generate with check: true:\n{verify_block}"
        );
        assert!(
            verify_block.contains("requires: [orb-release-binary]"),
            "verify-orb must require the freshly-built binary:\n{verify_block}"
        );
        // pack and container must be gated on the verify, so a drifted orb is
        // never packed/published.
        let pack_block = output
            .split("name: orb-release-pack")
            .nth(1)
            .unwrap()
            .split("\n      - ")
            .next()
            .unwrap()
            .to_string();
        assert!(
            pack_block.contains("verify-orb"),
            "orb-release-pack must require verify-orb:\n{pack_block}"
        );
        let container_block = output
            .split("name: orb-release-container")
            .nth(1)
            .unwrap()
            .split("\n      - ")
            .next()
            .unwrap()
            .to_string();
        assert!(
            container_block.contains("verify-orb"),
            "orb-release-container must require verify-orb:\n{container_block}"
        );
    }

    #[test]
    fn orb_release_publish_has_orb_name_and_pub_type() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let after_publish = output
            .split("      - orb-tools/publish:")
            .nth(1)
            .expect("no orb-tools/publish step");
        let publish_block = after_publish
            .split("\n      - ")
            .next()
            .unwrap_or(after_publish);
        assert!(
            publish_block.contains("orb_name: my-org/mytool"),
            "publish must set orb_name:\n{publish_block}"
        );
        assert!(
            publish_block.contains("pub_type: production"),
            "publish must set pub_type: production:\n{publish_block}"
        );
        assert!(
            publish_block.contains("vcs_type: github"),
            "publish must set vcs_type: github:\n{publish_block}"
        );
    }

    #[test]
    fn patch_build_orb_release_is_idempotent() {
        let (first, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let (second, second_report) = patch_build(&first, &make_opts());
        assert_eq!(
            first, second,
            "second patch must not change content — not idempotent"
        );
        // The orb-release jobs and workflow should be reported as skipped
        assert!(
            second_report
                .skipped
                .iter()
                .any(|s| s.contains("orb-release")),
            "expected orb-release entries skipped on second run:\n{:?}",
            second_report.skipped
        );
    }

    #[test]
    fn patch_build_orb_release_works_when_no_jobs_section() {
        let (output, _) = patch_build(BUILD_FIXTURE_NO_JOBS, &make_opts());
        assert!(
            output.contains("gen-circleci-orb/build_rust_binary:"),
            "build_rust_binary orb job ref missing when no pre-existing jobs section:\n{output}"
        );
        assert!(
            output.contains("gen-circleci-orb/build_container:"),
            "build_container orb job ref missing when no pre-existing jobs section:\n{output}"
        );
        assert!(
            output.contains("  orb-release:"),
            "orb-release workflow section missing when no pre-existing jobs section:\n{output}"
        );
    }

    #[test]
    fn patch_build_per_namespace_orb_release_ensure_and_publish() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts_two_ns());
        // Both namespaces get their own ensure_orb_registered step and publish step
        assert!(
            output.contains("name: orb-release-ensure-registered-my-org"),
            "missing ensure step for my-org:\n{output}"
        );
        assert!(
            output.contains("name: orb-release-ensure-registered-other-org"),
            "missing ensure step for other-org:\n{output}"
        );
        assert!(
            output.contains("name: publish-orb-my-org"),
            "missing publish-orb-my-org step:\n{output}"
        );
        assert!(
            output.contains("name: publish-orb-other-org"),
            "missing publish-orb-other-org step:\n{output}"
        );
    }

    // ── validation workflow: orb job references (not inline job defs) ────────

    #[test]
    fn validation_workflow_uses_build_rust_binary_orb_job() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let wf_start = output
            .find("  validation:")
            .expect("no validation workflow");
        let wf_section = &output[wf_start..];
        assert!(
            wf_section.contains("gen-circleci-orb/build_rust_binary:"),
            "validation workflow must reference gen-circleci-orb/build_rust_binary orb job (not an inline job def):\n{wf_section}"
        );
        assert!(
            wf_section.contains("name: build-binary"),
            "build_rust_binary step must be named build-binary:\n{wf_section}"
        );
    }

    #[test]
    fn validation_workflow_uses_generate_orb_job() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let wf_start = output
            .find("  validation:")
            .expect("no validation workflow");
        let wf_section = &output[wf_start..];
        assert!(
            wf_section.contains("gen-circleci-orb/generate:"),
            "validation workflow must reference gen-circleci-orb/generate orb job (not an inline job def):\n{wf_section}"
        );
        assert!(
            wf_section.contains("name: regenerate-orb"),
            "generate step must be named regenerate-orb:\n{wf_section}"
        );
    }

    #[test]
    fn no_inline_build_regen_job_defs_in_jobs_section() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        // Only inspect content before `workflows:` to isolate the jobs section
        let jobs_section = output.split("\nworkflows:").next().unwrap_or(&output);
        assert!(
            !jobs_section.contains("  build-binary:"),
            "build-binary must NOT appear as an inline job definition:\n{jobs_section}"
        );
        assert!(
            !jobs_section.contains("  regenerate-orb:"),
            "regenerate-orb must NOT appear as an inline job definition:\n{jobs_section}"
        );
    }

    #[test]
    fn validation_build_step_requires_configured_job() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let after_bb = output
            .split("gen-circleci-orb/build_rust_binary:")
            .nth(1)
            .expect("no build_rust_binary step in validation");
        let step_block = after_bb.split("      - ").next().unwrap_or(after_bb);
        assert!(
            step_block.contains("requires: [common-tests]"),
            "build_rust_binary step must require the configured job:\n{step_block}"
        );
    }

    #[test]
    fn validation_generate_step_requires_build_binary() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let after_regen = output
            .split("gen-circleci-orb/generate:")
            .nth(1)
            .expect("no gen-circleci-orb/generate step");
        let step_block = after_regen.split("      - ").next().unwrap_or(after_regen);
        assert!(
            step_block.contains("requires: [build-binary]"),
            "generate step must require build-binary:\n{step_block}"
        );
    }

    const BUILD_FIXTURE: &str = "\
version: 2.1

orbs:
  toolkit: jerus-org/circleci-toolkit@6.0.0

jobs:
  some-job:
    docker:
      - image: cimg/base:stable
    steps:
      - run: echo hello

workflows:
  validation:
    jobs:
      - some-job
";

    // Typical toolkit 6.0 config: no top-level jobs section, only orbs + workflows.
    const BUILD_FIXTURE_NO_JOBS: &str = "\
version: 2.1

parameters:
  min_rust_version:
    type: string
    default: \"1.87\"

orbs:
  toolkit: jerus-org/circleci-toolkit@6.0.0

workflows:
  validation:
    jobs:
      - toolkit/required_builds:
          min_rust_version: << pipeline.parameters.min_rust_version >>

      - toolkit/common_tests:
          min_rust_version: << pipeline.parameters.min_rust_version >>

      - toolkit/idiomatic_rust:
          filters:
            branches:
              ignore: main
";

    const RELEASE_FIXTURE: &str = "\
version: 2.1

orbs:
  toolkit: jerus-org/circleci-toolkit@6.0.0

jobs:
  release-mytool:
    docker:
      - image: cimg/rust:stable
    steps:
      - checkout

workflows:
  release:
    jobs:
      - release-mytool
";

    // ── patch_build (no pre-existing jobs section) ───────────────────────────

    #[test]
    fn patch_build_wires_workflow_steps_when_no_jobs_section() {
        let (output, report) = patch_build(BUILD_FIXTURE_NO_JOBS, &make_opts());
        assert!(
            output.contains("gen-circleci-orb/build_rust_binary:"),
            "missing build_rust_binary orb step when no pre-existing jobs section:\n{output}"
        );
        assert!(
            output.contains("gen-circleci-orb/generate:"),
            "missing generate orb step when no pre-existing jobs section:\n{output}"
        );
        assert!(
            output.contains("orb-tools/pack:"),
            "pack step not wired into workflow:\n{output}"
        );
        assert!(
            output.contains("orb-tools/review:"),
            "review step not wired into workflow:\n{output}"
        );
        assert!(
            report.insertions.iter().any(|s| s.contains("workflow")),
            "report missing workflow steps"
        );
    }

    // ── patch_build ───────────────────────────────────────────────────────────

    #[test]
    fn patch_build_adds_orb_tools_to_orbs_section() {
        let (output, report) = patch_build(BUILD_FIXTURE, &make_opts());
        assert!(
            output.contains("orb-tools: circleci/orb-tools@12.3.3"),
            "missing orb-tools entry:\n{output}"
        );
        assert!(
            report.insertions.iter().any(|s| s.contains("orb-tools")),
            "report missing orb-tools insertion"
        );
    }

    #[test]
    fn patch_build_adds_build_rust_binary_orb_step() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        assert!(
            output.contains("gen-circleci-orb/build_rust_binary:"),
            "missing build_rust_binary orb step:\n{output}"
        );
        assert!(
            output.contains("name: build-binary"),
            "build_rust_binary step must be named build-binary:\n{output}"
        );
        assert!(
            output.contains("package: mytool"),
            "build_rust_binary step must set package param:\n{output}"
        );
    }

    #[test]
    fn patch_build_adds_generate_orb_step() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        assert!(
            output.contains("gen-circleci-orb/generate:"),
            "missing gen-circleci-orb/generate orb step:\n{output}"
        );
        assert!(
            output.contains("name: regenerate-orb"),
            "generate step must be named regenerate-orb:\n{output}"
        );
        assert!(
            output.contains("attach_workspace: true"),
            "generate step must set attach_workspace: true:\n{output}"
        );
    }

    #[test]
    fn workflow_build_binary_precedes_regenerate_orb() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let bb_pos = output
            .find("name: build-binary")
            .expect("no build-binary step");
        let regen_pos = output
            .find("name: regenerate-orb")
            .expect("no regenerate-orb step");
        assert!(
            bb_pos < regen_pos,
            "build-binary must appear before regenerate-orb in the workflow"
        );
    }

    #[test]
    fn patch_build_adds_workflow_steps() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        assert!(
            output.contains("orb-tools/pack:"),
            "missing pack step:\n{output}"
        );
        assert!(
            output.contains("orb-tools/review:"),
            "missing review step:\n{output}"
        );
        // Parameters must use snake_case (orb-tools@12 API)
        assert!(
            output.contains("source_dir: orb/src"),
            "pack/review must use source_dir (not source-dir):\n{output}"
        );
        assert!(
            !output.contains("destination-orb-path"),
            "must not use deprecated destination-orb-path:\n{output}"
        );
    }

    #[test]
    fn patch_build_is_idempotent() {
        let (first, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let (second, second_report) = patch_build(&first, &make_opts());
        assert_eq!(
            first, second,
            "second patch changed content — not idempotent"
        );
        assert!(
            second_report
                .skipped
                .iter()
                .any(|s| s.contains("orb-tools")),
            "expected orb-tools skipped on second run"
        );
        assert!(
            second_report
                .skipped
                .iter()
                .any(|s| s.contains("build-binary")),
            "expected build-binary and regenerate-orb skipped on second run"
        );
    }

    #[test]
    fn patch_build_orb_steps_are_in_workflow_not_jobs_section() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let workflows_pos = output.find("\nworkflows:").expect("no workflows: section");
        // orb job references live in the workflow, not as inline job defs
        let build_ref_pos = output
            .find("gen-circleci-orb/build_rust_binary:")
            .expect("no build_rust_binary ref");
        let regen_ref_pos = output
            .find("gen-circleci-orb/generate:")
            .expect("no generate ref");
        assert!(
            build_ref_pos > workflows_pos,
            "build_rust_binary ref must be inside the workflows section"
        );
        assert!(
            regen_ref_pos > workflows_pos,
            "generate ref must be inside the workflows section"
        );
    }

    // ── --mcp: gen-circleci-orb/build_mcp_server integration ─────────────────

    #[test]
    fn patch_build_mcp_disabled_does_not_add_build_mcp_server() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let after_wf = output.split("  orb-release:").nth(1).unwrap_or("");
        assert!(
            !after_wf.contains("build_mcp_server:"),
            "build_mcp_server must not appear when --mcp is false:\n{output}"
        );
    }

    #[test]
    fn patch_build_mcp_uses_build_mcp_server_orb_job() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts_mcp());
        let after_wf = output
            .split("  orb-release:")
            .nth(1)
            .expect("no orb-release workflow");
        assert!(
            after_wf.contains("gen-circleci-orb/build_mcp_server:"),
            "mcp must use gen-circleci-orb/build_mcp_server orb job:\n{after_wf}"
        );
        assert!(
            after_wf.contains("name: build-mcp-server"),
            "build_mcp_server step must be named build-mcp-server:\n{after_wf}"
        );
    }

    #[test]
    fn patch_build_mcp_server_has_binary_name_and_tag_prefix() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts_mcp());
        let after_step = output
            .split("name: build-mcp-server")
            .nth(1)
            .expect("no build-mcp-server step");
        let step = after_step.split("\n      - ").next().unwrap_or(after_step);
        assert!(
            step.contains("binary_name: mytool"),
            "build_mcp_server must pass binary_name:\n{step}"
        );
        assert!(
            step.contains("tag_prefix: mytool-v"),
            "build_mcp_server must pass tag_prefix:\n{step}"
        );
    }

    #[test]
    fn patch_build_mcp_server_has_earliest_version() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts_mcp());
        let after_step = output
            .split("name: build-mcp-server")
            .nth(1)
            .expect("no build-mcp-server step");
        let step = after_step.split("\n      - ").next().unwrap_or(after_step);
        assert!(
            step.contains("earliest_version: \"1.0.0\""),
            "build_mcp_server must pass earliest_version from opts:\n{step}"
        );
    }

    #[test]
    fn patch_build_mcp_server_requires_publish_orb_steps() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts_mcp());
        let after_step = output
            .split("name: build-mcp-server")
            .nth(1)
            .expect("no build-mcp-server step");
        let step = after_step.split("\n      - ").next().unwrap_or(after_step);
        assert!(
            step.contains("requires: [publish-orb-my-org]"),
            "build_mcp_server must require publish-orb steps:\n{step}"
        );
    }

    #[test]
    fn patch_build_mcp_server_requires_all_namespaces() {
        let opts = PatchOpts {
            mcp: true,
            namespaces: vec!["ns-a".to_string(), "ns-b".to_string()],
            ..make_opts()
        };
        let (output, _) = patch_build(BUILD_FIXTURE, &opts);
        let after_step = output
            .split("name: build-mcp-server")
            .nth(1)
            .expect("no build-mcp-server step");
        let step = after_step.split("\n      - ").next().unwrap_or(after_step);
        assert!(
            step.contains("publish-orb-ns-a") && step.contains("publish-orb-ns-b"),
            "build_mcp_server must require all publish-orb steps:\n{step}"
        );
    }

    #[test]
    fn patch_build_mcp_server_uses_mcp_context() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts_mcp());
        let after_step = output
            .split("name: build-mcp-server")
            .nth(1)
            .expect("no build-mcp-server step");
        let step = after_step.split("\n      - ").next().unwrap_or(after_step);
        assert!(
            step.contains("context: [pcu-app]"),
            "build_mcp_server must use mcp_context:\n{step}"
        );
    }

    #[test]
    fn patch_build_mcp_server_supports_multiple_contexts() {
        let opts = PatchOpts {
            mcp: true,
            mcp_context: vec![
                "release".to_string(),
                "bot-check".to_string(),
                "pcu-app".to_string(),
            ],
            ..make_opts()
        };
        let (output, _) = patch_build(BUILD_FIXTURE, &opts);
        let after_step = output
            .split("name: build-mcp-server")
            .nth(1)
            .expect("no build-mcp-server step");
        let step = after_step.split("\n      - ").next().unwrap_or(after_step);
        assert!(
            step.contains("context: [release, bot-check, pcu-app]"),
            "build_mcp_server must emit all contexts:\n{step}"
        );
    }

    #[test]
    fn patch_build_mcp_server_has_tag_filter() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts_mcp());
        let after_step = output
            .split("name: build-mcp-server")
            .nth(1)
            .expect("no build-mcp-server step");
        let step = after_step.split("\n      - ").next().unwrap_or(after_step);
        assert!(
            step.contains("tags:") && step.contains("/^mytool-v.*/"),
            "build_mcp_server step must have tag filter:\n{step}"
        );
    }

    #[test]
    fn patch_build_mcp_is_idempotent() {
        let (first, _) = patch_build(BUILD_FIXTURE, &make_opts_mcp());
        let (second, second_report) = patch_build(&first, &make_opts_mcp());
        assert_eq!(first, second, "second mcp patch changed content");
        assert!(
            second_report
                .skipped
                .iter()
                .any(|s| s.contains("orb-release")),
            "expected orb-release skipped on second run:\n{:?}",
            second_report.skipped
        );
    }

    #[test]
    fn patch_build_mcp_does_not_add_gen_orb_mcp_orb_entry() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts_mcp());
        assert!(
            !output.contains("gen-orb-mcp: jerus-org"),
            "gen-orb-mcp orb must not appear — build_mcp_server is part of gen-circleci-orb:\n{output}"
        );
    }

    #[test]
    fn patch_build_step2_skips_when_orb_references_present() {
        // A config that already uses gen-circleci-orb orb references (not inline jobs)
        // must not get duplicate inline job definitions added by step2.
        let (first, _) = patch_build(BUILD_FIXTURE_ORB_REFS, &make_opts());
        let (second, second_report) = patch_build(&first, &make_opts());
        assert_eq!(
            first, second,
            "second patch changed content on orb-ref config"
        );
        assert!(
            second_report
                .skipped
                .iter()
                .any(|s| s.contains("build-binary")),
            "step2 must be skipped when orb references are already present:\n{:?}",
            second_report.skipped
        );
        // Must not contain two build-binary job definitions
        let count = first.matches("build-binary:").count();
        assert!(
            count <= 2,
            "must not have duplicate build-binary entries (found {count}):\n{first}"
        );
    }

    const BUILD_FIXTURE_ORB_REFS: &str = "\
version: 2.1

orbs:
  toolkit: jerus-org/circleci-toolkit@6.0.0
  orb-tools: circleci/orb-tools@12.4.0
  gen-circleci-orb: jerus-org/gen-circleci-orb@0.0.1

workflows:
  validation:
    jobs:
      - toolkit/common_tests

      - gen-circleci-orb/build_rust_binary:
          name: build-binary
          package: mytool
          requires:
            - toolkit/common_tests
      - gen-circleci-orb/generate:
          name: regenerate-orb
          binary: mytool
          orb_namespace: my-org
          requires:
            - build-binary
      - orb-tools/pack:
          name: pack-orb
          source_dir: orb/src
          requires: [regenerate-orb]
      - orb-tools/review:
          name: review-orb
          source_dir: orb/src
          requires: [pack-orb]
";
}
