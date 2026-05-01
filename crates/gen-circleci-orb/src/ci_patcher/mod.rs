use anyhow::Result;

pub struct PatchOpts {
    pub binary: String,
    pub namespace: String,
    pub docker_namespace: String,
    pub orb_dir: String,
    pub build_workflow: String,
    pub release_workflow: String,
    pub requires_job: Option<String>,
    pub release_after_job: Option<String>,
    pub orb_tools_version: String,
    pub docker_orb_version: String,
    pub docker_context: String,
    pub orb_context: String,
    pub mcp: bool,
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

    // 1. Add orb-tools to orbs section
    let orb_entry = format!("  orb-tools: circleci/orb-tools@{}", opts.orb_tools_version);
    if content.contains("orb-tools:") {
        report.skipped.push("orb-tools orb".to_string());
    } else if let Some(pos) = find_section_end(&lines, "orbs:") {
        lines.insert(pos, orb_entry);
        report.insertions.push("orb-tools orb".to_string());
    }

    // 2. Add jobs section if missing, then add build-binary + regenerate-orb jobs
    let jobs_present = content.contains("build-binary:") && content.contains("regenerate-orb:");
    if jobs_present {
        report
            .skipped
            .push("build-binary and regenerate-orb jobs".to_string());
    } else {
        let build_block = build_binary_job(opts);
        let regen_block = regenerate_orb_job(opts);
        if let Some(pos) = find_section_end(&lines, "jobs:") {
            for (i, l) in build_block.iter().enumerate() {
                lines.insert(pos + i, l.clone());
            }
            let after_build = pos + build_block.len();
            for (i, l) in regen_block.iter().enumerate() {
                lines.insert(after_build + i, l.clone());
            }
        } else {
            // No jobs section — insert before workflows:
            if let Some(wf_pos) = find_top_level(&lines, "workflows:") {
                lines.insert(wf_pos, String::new());
                lines.insert(wf_pos, String::new());
                // Insert regen block first (it goes last in jobs), then build block before it
                let regen_len = regen_block.len();
                for (i, _) in regen_block.iter().rev().enumerate() {
                    lines.insert(wf_pos, regen_block[regen_len - 1 - i].clone());
                }
                let build_len = build_block.len();
                for (i, _) in build_block.iter().rev().enumerate() {
                    lines.insert(wf_pos, build_block[build_len - 1 - i].clone());
                }
                lines.insert(wf_pos, "jobs:".to_string());
            }
        }
        report
            .insertions
            .push("build-binary and regenerate-orb jobs".to_string());
    }

    // 3. Add workflow steps (regenerate-orb, orb-tools/pack, orb-tools/validate)
    if content.contains("orb-tools/pack:") {
        report.skipped.push("workflow steps".to_string());
    } else {
        let step_block = pack_validate_steps(opts);
        if let Some(pos) = find_workflow_jobs_end(&lines, &opts.build_workflow) {
            for (i, l) in step_block.iter().enumerate() {
                lines.insert(pos + i, l.clone());
            }
        }
        report.insertions.push("workflow steps".to_string());
    }

    let mut output = lines.join("\n");
    if content.ends_with('\n') {
        output.push('\n');
    }
    (output, report)
}

/// Patch a release CircleCI config string.
/// Returns the modified content and a report of what was changed or skipped.
pub fn patch_release(content: &str, opts: &PatchOpts) -> (String, PatchReport) {
    let mut report = PatchReport {
        insertions: vec![],
        skipped: vec![],
    };
    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();

    // 1. Add docker + orb-tools orbs
    // Check specifically for circleci orb entries, not random "docker:" keys in job defs
    let docker_entry = format!("  docker: circleci/docker@{}", opts.docker_orb_version);
    let orb_entry = format!("  orb-tools: circleci/orb-tools@{}", opts.orb_tools_version);
    let has_docker_orb = content.contains("  docker: circleci/");
    let has_orb_tools = content.contains("  orb-tools: circleci/");

    if has_docker_orb && has_orb_tools {
        report.skipped.push("docker and orb-tools orbs".to_string());
    } else {
        if let Some(pos) = find_section_end(&lines, "orbs:") {
            let mut insert_pos = pos;
            if !has_orb_tools {
                lines.insert(insert_pos, orb_entry);
                insert_pos += 1;
            }
            if !has_docker_orb {
                lines.insert(insert_pos, docker_entry);
            }
            report
                .insertions
                .push("docker and orb-tools orbs".to_string());
        }
    }

    // 2. Add build-container job
    if content.contains("build-container:") {
        report.skipped.push("build-container job".to_string());
    } else {
        let job_block = build_container_job(opts);
        if let Some(pos) = find_section_end(&lines, "jobs:") {
            for (i, l) in job_block.iter().enumerate() {
                lines.insert(pos + i, l.clone());
            }
        } else if let Some(wf_pos) = find_top_level(&lines, "workflows:") {
            // No top-level jobs section — create one before workflows:
            lines.insert(wf_pos, String::new());
            lines.insert(wf_pos, String::new());
            let job_len = job_block.len();
            for (i, _) in job_block.iter().rev().enumerate() {
                lines.insert(wf_pos, job_block[job_len - 1 - i].clone());
            }
            lines.insert(wf_pos, "jobs:".to_string());
        }
        report.insertions.push("build-container job".to_string());
    }

    // 3. Add release workflow steps
    if content.contains("pack-orb-release")
        && content.contains("      - build-container:")
        && content.contains("orb-tools/publish:")
    {
        report.skipped.push("release workflow steps".to_string());
    } else {
        let step_block = release_workflow_steps(opts);
        if let Some(pos) = find_workflow_jobs_end(&lines, &opts.release_workflow) {
            for (i, l) in step_block.iter().enumerate() {
                lines.insert(pos + i, l.clone());
            }
        }
        report.insertions.push("release workflow steps".to_string());
    }

    let mut output = lines.join("\n");
    if content.ends_with('\n') {
        output.push('\n');
    }
    (output, report)
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

fn build_binary_job(opts: &PatchOpts) -> Vec<String> {
    let binary = &opts.binary;
    vec![
        "  build-binary:".to_string(),
        "    docker:".to_string(),
        "      - image: rust:latest".to_string(),
        "    steps:".to_string(),
        "      - checkout".to_string(),
        "      - run:".to_string(),
        "          name: Build binary".to_string(),
        "          command: cargo build --release".to_string(),
        "      - persist_to_workspace:".to_string(),
        "          root: target/release".to_string(),
        format!("          paths: [{binary}]"),
    ]
}

fn regenerate_orb_job(opts: &PatchOpts) -> Vec<String> {
    let binary = &opts.binary;
    let namespace = &opts.namespace;
    let orb_dir = &opts.orb_dir;
    // gen-circleci-orb is pre-installed in its own Docker image (jerusdp/gen-circleci-orb).
    // The target binary is attached from the build-binary workspace — no runtime installs needed.
    vec![
        "  regenerate-orb:".to_string(),
        "    docker:".to_string(),
        "      - image: jerusdp/gen-circleci-orb:latest".to_string(),
        "    steps:".to_string(),
        "      - checkout".to_string(),
        "      - attach_workspace:".to_string(),
        "          at: /tmp/bin".to_string(),
        "      - run:".to_string(),
        "          name: Regenerate orb source".to_string(),
        "          command: |".to_string(),
        "            export PATH=\"/tmp/bin:$PATH\"".to_string(),
        "            gen-circleci-orb generate \\".to_string(),
        format!("              --binary {binary} \\"),
        format!("              --namespace {namespace} \\"),
        format!("              --orb-dir {orb_dir}"),
    ]
}

fn pack_validate_steps(opts: &PatchOpts) -> Vec<String> {
    let orb_dir = &opts.orb_dir;
    let mut steps = vec![];

    // build-binary workflow step — compiles the binary and persists to workspace
    steps.push("      - build-binary:".to_string());
    if let Some(req) = &opts.requires_job {
        steps.push(format!("          requires: [{req}]"));
    }

    // regenerate-orb workflow step — attaches workspace, installs gen-circleci-orb, runs generate
    steps.push("      - regenerate-orb:".to_string());
    steps.push("          requires: [build-binary]".to_string());

    // orb-tools/pack (source_dir + workspace persistence; validates on pack)
    steps.push("      - orb-tools/pack:".to_string());
    steps.push("          name: pack-orb".to_string());
    steps.push(format!("          source_dir: {orb_dir}/src"));
    steps.push("          requires: [regenerate-orb]".to_string());

    // orb-tools/review (best-practice review of packed orb)
    steps.push("      - orb-tools/review:".to_string());
    steps.push("          name: review-orb".to_string());
    steps.push(format!("          source_dir: {orb_dir}/src"));
    steps.push("          requires: [pack-orb]".to_string());

    steps
}

fn build_container_job(opts: &PatchOpts) -> Vec<String> {
    let binary = &opts.binary;
    let docker_ns = &opts.docker_namespace;
    let orb_dir = &opts.orb_dir;
    // CIRCLE_TAG is only set when a pipeline is triggered by a tag push.
    // The release pipeline is triggered by a merge commit, so CIRCLE_TAG is empty.
    // Instead, fetch tags and derive the version from the most recent matching tag.
    vec![
        "  build-container:".to_string(),
        "    docker:".to_string(),
        "      - image: cimg/base:stable".to_string(),
        "    steps:".to_string(),
        "      - checkout".to_string(),
        "      - setup_remote_docker".to_string(),
        "      - run:".to_string(),
        "          name: Build Docker image".to_string(),
        "          command: |".to_string(),
        "            git fetch --tags".to_string(),
        format!("            VERSION=$(git tag --list \"{binary}-v*\" --sort=-version:refname | head -1 | sed 's/{binary}-v//')"),
        format!("            docker build -t {docker_ns}/{binary}:${{VERSION}} -t {docker_ns}/{binary}:latest {orb_dir}"),
        "      - run:".to_string(),
        "          name: Push Docker image".to_string(),
        "          command: |".to_string(),
        "            docker login -u \"${DOCKER_LOGIN}\" -p \"${DOCKER_PASSWORD}\"".to_string(),
        "            git fetch --tags".to_string(),
        format!("            VERSION=$(git tag --list \"{binary}-v*\" --sort=-version:refname | head -1 | sed 's/{binary}-v//')"),
        format!("            docker push {docker_ns}/{binary}:${{VERSION}}"),
        format!("            docker push {docker_ns}/{binary}:latest"),
    ]
}

fn release_workflow_steps(opts: &PatchOpts) -> Vec<String> {
    let binary = &opts.binary;
    let namespace = &opts.namespace;
    let docker_ctx = &opts.docker_context;
    let orb_ctx = &opts.orb_context;
    let orb_dir = &opts.orb_dir;
    let requires = opts
        .release_after_job
        .as_deref()
        .map(|r| format!("[{r}]"))
        .unwrap_or_default();
    let mut steps = vec![];

    // Pack orb from committed source (runs in parallel with build-container)
    steps.push("      - orb-tools/pack:".to_string());
    steps.push("          name: pack-orb-release".to_string());
    steps.push(format!("          source_dir: {orb_dir}/src"));
    if !requires.is_empty() {
        steps.push(format!("          requires: {requires}"));
    }

    steps.push("      - build-container:".to_string());
    if !requires.is_empty() {
        steps.push(format!("          requires: {requires}"));
    }
    steps.push(format!("          context: [{docker_ctx}]"));

    steps.push("      - orb-tools/publish:".to_string());
    steps.push(format!("          name: publish-orb-{namespace}"));
    steps.push(format!("          orb_name: {namespace}/{binary}"));
    steps.push("          pub_type: production".to_string());
    steps.push("          vcs_type: github".to_string());
    steps.push("          requires: [build-container, pack-orb-release]".to_string());
    steps.push(format!("          context: [{orb_ctx}]"));

    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_opts() -> PatchOpts {
        PatchOpts {
            binary: "mytool".to_string(),
            namespace: "my-org".to_string(),
            docker_namespace: "my-docker-org".to_string(),
            orb_dir: "orb".to_string(),
            build_workflow: "validation".to_string(),
            release_workflow: "release".to_string(),
            requires_job: Some("common-tests".to_string()),
            release_after_job: Some("release-mytool".to_string()),
            orb_tools_version: "12.3.3".to_string(),
            docker_orb_version: "3.0.1".to_string(),
            docker_context: "docker-credentials".to_string(),
            orb_context: "orb-publishing".to_string(),
            mcp: false,
        }
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

    // Toolkit-style release.yml: no top-level jobs section, only orbs + workflows.
    // This is the common case for projects using only toolkit jobs.
    const RELEASE_FIXTURE_NO_JOBS: &str = "\
version: 2.1

orbs:
  toolkit: jerus-org/circleci-toolkit@6.0.0

workflows:
  release:
    jobs:
      - toolkit/release_crate:
          name: release-mytool
          context: [release]
      - toolkit/release_prlog:
          requires: [release-mytool]
          context: [release]
";

    // ── patch_build (no pre-existing jobs section) ───────────────────────────

    #[test]
    fn patch_build_wires_workflow_steps_when_no_jobs_section() {
        let (output, report) = patch_build(BUILD_FIXTURE_NO_JOBS, &make_opts());
        assert!(
            output.contains("regenerate-orb:"),
            "missing job def:\n{output}"
        );
        assert!(
            output.contains("orb-tools/pack:"),
            "pack step not wired into workflow:\n{output}"
        );
        assert!(
            output.contains("orb-tools/review:"),
            "review step not wired into workflow:\n{output}"
        );
        // Both the job and the workflow steps should be in the report
        assert!(
            report
                .insertions
                .iter()
                .any(|s| s.contains("regenerate-orb")),
            "report missing regenerate-orb"
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
    fn patch_build_adds_build_binary_job() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        assert!(
            output.contains("  build-binary:"),
            "missing build-binary job:\n{output}"
        );
    }

    /// Collect lines belonging to a top-level job block (2-space indented header).
    /// Returns the content lines (not the header itself) as a single string.
    fn job_block(output: &str, job_name: &str) -> String {
        let header = format!("  {job_name}:");
        let mut in_block = false;
        let mut result = String::new();
        for line in output.lines() {
            if line.trim_end() == header.trim_end() {
                in_block = true;
                continue;
            }
            if in_block {
                // A non-empty line starting with ≤2 spaces that isn't the header means a new
                // top-level section or job; the block is done.
                if !line.is_empty() && !line.starts_with("   ") && !line.starts_with('\t') {
                    break;
                }
                result.push_str(line);
                result.push('\n');
            }
        }
        result
    }

    #[test]
    fn build_binary_uses_public_rust_image() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let block = job_block(&output, "build-binary");
        assert!(
            block.contains("rust:latest"),
            "build-binary must use the public rust:latest image, not a private CI image:\n{block}"
        );
    }

    #[test]
    fn build_binary_runs_cargo_build_release() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let block = job_block(&output, "build-binary");
        assert!(
            block.contains("cargo build --release"),
            "build-binary must run cargo build --release:\n{block}"
        );
    }

    #[test]
    fn build_binary_persists_to_workspace() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let block = job_block(&output, "build-binary");
        assert!(
            block.contains("persist_to_workspace"),
            "build-binary must persist binary to workspace:\n{block}"
        );
    }

    #[test]
    fn patch_build_adds_regenerate_orb_job() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        assert!(output.contains("regenerate-orb:"), "missing job:\n{output}");
        // gen-circleci-orb is pre-installed in its own image; no install step needed
        assert!(
            !output.contains("install-from-binstall-release.sh"),
            "regenerate-orb must not bootstrap cargo-binstall — use the gen-circleci-orb image:\n{output}"
        );
        assert!(
            !output.contains("cargo-binstall --no-confirm gen-circleci-orb"),
            "regenerate-orb must not install gen-circleci-orb at runtime:\n{output}"
        );
        assert!(
            output.contains("gen-circleci-orb generate"),
            "missing generate step:\n{output}"
        );
    }

    #[test]
    fn regenerate_orb_uses_gen_circleci_orb_image_and_attaches_workspace() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let block = job_block(&output, "regenerate-orb");
        assert!(
            block.contains("jerusdp/gen-circleci-orb:latest"),
            "regenerate-orb must use the gen-circleci-orb Docker image (gen-circleci-orb is pre-installed):\n{block}"
        );
        assert!(
            block.contains("attach_workspace"),
            "regenerate-orb must attach workspace to get the target binary:\n{block}"
        );
        assert!(
            !block.contains("cargo-binstall"),
            "regenerate-orb must not install anything — gen-circleci-orb is in the image:\n{block}"
        );
    }

    #[test]
    fn workflow_build_binary_precedes_regenerate_orb() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let bb_pos = output
            .find("      - build-binary:")
            .expect("no build-binary workflow step");
        let regen_pos = output
            .find("      - regenerate-orb:")
            .expect("no regenerate-orb workflow step");
        assert!(
            bb_pos < regen_pos,
            "build-binary must appear before regenerate-orb in the workflow"
        );
    }

    #[test]
    fn build_binary_workflow_step_requires_configured_job() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        // The build-binary workflow step must require the user-configured prerequisite
        let after_bb = output
            .split("      - build-binary:")
            .nth(1)
            .expect("no build-binary workflow step");
        let step_block = after_bb.split("      - ").next().unwrap_or(after_bb);
        assert!(
            step_block.contains("requires: [common-tests]"),
            "build-binary workflow step must require the configured job:\n{step_block}"
        );
    }

    #[test]
    fn regenerate_orb_workflow_step_requires_build_binary() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let after_regen = output
            .split("      - regenerate-orb:")
            .nth(1)
            .expect("no regenerate-orb workflow step");
        let step_block = after_regen.split("      - ").next().unwrap_or(after_regen);
        assert!(
            step_block.contains("requires: [build-binary]"),
            "regenerate-orb workflow step must require build-binary:\n{step_block}"
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
    fn patch_build_job_in_jobs_section_not_workflows() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        // The regenerate-orb: job definition must appear in the jobs section
        let jobs_pos = output.find("\njobs:").expect("no jobs: section");
        let workflows_pos = output.find("\nworkflows:").expect("no workflows: section");
        let job_def_pos = output.find("  regenerate-orb:").expect("no job def");
        assert!(
            job_def_pos > jobs_pos && job_def_pos < workflows_pos,
            "job definition not in jobs section"
        );
    }

    // ── patch_release ─────────────────────────────────────────────────────────

    #[test]
    fn patch_release_adds_docker_and_orb_tools_orbs() {
        let (output, _) = patch_release(RELEASE_FIXTURE, &make_opts());
        assert!(
            output.contains("docker: circleci/docker@3.0.1"),
            "missing docker orb:\n{output}"
        );
        assert!(
            output.contains("orb-tools: circleci/orb-tools@12.3.3"),
            "missing orb-tools orb:\n{output}"
        );
    }

    #[test]
    fn patch_release_adds_build_container_job() {
        let (output, _) = patch_release(RELEASE_FIXTURE, &make_opts());
        assert!(
            output.contains("build-container:"),
            "missing job:\n{output}"
        );
        assert!(
            output.contains("docker build"),
            "missing docker build step:\n{output}"
        );
        assert!(
            output.contains("docker push"),
            "missing docker push step:\n{output}"
        );
        // CIRCLE_TAG is empty in merge-triggered pipelines; version must come from git tags
        assert!(
            !output.contains("${CIRCLE_TAG}"),
            "must not use CIRCLE_TAG (empty in merge pipelines):\n{output}"
        );
        assert!(
            output.contains("git fetch --tags"),
            "must fetch tags to get the just-released version:\n{output}"
        );
        assert!(
            output.contains("docker login"),
            "must log in to Docker Hub before pushing:\n{output}"
        );
        assert!(
            output.contains(":latest"),
            "must also push a :latest tag:\n{output}"
        );
    }

    #[test]
    fn patch_release_uses_docker_namespace_not_orb_namespace() {
        let (output, _) = patch_release(RELEASE_FIXTURE, &make_opts());
        // Docker build/push must use docker_namespace, not the orb namespace
        assert!(
            output.contains("docker build -t my-docker-org/mytool"),
            "docker build must use docker_namespace:\n{output}"
        );
        assert!(
            output.contains("docker push my-docker-org/mytool"),
            "docker push must use docker_namespace:\n{output}"
        );
        assert!(
            !output.contains("docker build -t my-org/"),
            "docker build must NOT use orb namespace:\n{output}"
        );
    }

    #[test]
    fn patch_release_adds_workflow_steps() {
        let (output, _) = patch_release(RELEASE_FIXTURE, &make_opts());
        // Pack step must appear in release workflow to provide workspace for publish
        assert!(
            output.contains("orb-tools/pack:"),
            "missing pack step in release workflow:\n{output}"
        );
        assert!(
            output.contains("orb-tools/publish:"),
            "missing publish step:\n{output}"
        );
        assert!(
            output.contains("publish-orb-my-org"),
            "missing publish job name:\n{output}"
        );
        assert!(
            output.contains("pub_type: production"),
            "publish must set pub_type production:\n{output}"
        );
        assert!(
            output.contains("vcs_type: github"),
            "publish must set vcs_type (required by orb-tools/publish):\n{output}"
        );
    }

    #[test]
    fn patch_release_is_idempotent() {
        let (first, _) = patch_release(RELEASE_FIXTURE, &make_opts());
        let (second, second_report) = patch_release(&first, &make_opts());
        assert_eq!(
            first, second,
            "second release patch changed content — not idempotent"
        );
        assert!(
            !second_report.skipped.is_empty(),
            "expected skipped entries on second run"
        );
    }

    #[test]
    fn patch_release_includes_namespace_and_binary_in_publish() {
        let (output, _) = patch_release(RELEASE_FIXTURE, &make_opts());
        // orb-tools@12 uses snake_case orb_name (not hyphenated orb-name)
        assert!(
            output.contains("orb_name: my-org/mytool"),
            "missing orb_name (underscore):\n{output}"
        );
        assert!(
            !output.contains("orb-path:"),
            "must not use deprecated orb-path:\n{output}"
        );
    }

    #[test]
    fn patch_release_adds_build_container_job_when_no_jobs_section() {
        // release.yml with only toolkit jobs (no top-level jobs: section) is the
        // common case. patch_release must create the jobs: section and insert
        // build-container, not silently skip it.
        let (output, _) = patch_release(RELEASE_FIXTURE_NO_JOBS, &make_opts());
        assert!(
            output.contains("build-container:"),
            "build-container job definition missing when no pre-existing jobs section:\n{output}"
        );
        let jobs_pos = output.find("\njobs:").expect("jobs: section not created");
        let workflows_pos = output.find("\nworkflows:").expect("no workflows: section");
        let job_def_pos = output
            .find("  build-container:")
            .expect("no build-container job definition");
        assert!(
            job_def_pos > jobs_pos && job_def_pos < workflows_pos,
            "build-container definition must be in jobs: section, not workflows:\n{output}"
        );
    }
}
