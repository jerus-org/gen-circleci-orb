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

    // 2. Add build-binary-release and build-container jobs
    let has_binary_release = content.contains("build-binary-release:");
    let has_container = content.contains("build-container:");
    if has_binary_release && has_container {
        report
            .skipped
            .push("build-binary-release and build-container jobs".to_string());
    } else {
        if let Some(pos) = find_section_end(&lines, "jobs:") {
            let mut insert_pos = pos;
            if !has_binary_release {
                let binary_block = build_binary_release_job(opts);
                let binary_len = binary_block.len();
                for (i, l) in binary_block.into_iter().enumerate() {
                    lines.insert(insert_pos + i, l);
                }
                insert_pos += binary_len;
            }
            if !has_container {
                let container_block = build_container_job(opts);
                for (i, l) in container_block.into_iter().enumerate() {
                    lines.insert(insert_pos + i, l);
                }
            }
        } else if let Some(wf_pos) = find_top_level(&lines, "workflows:") {
            // No top-level jobs section — create one before workflows:
            lines.insert(wf_pos, String::new());
            lines.insert(wf_pos, String::new());
            let container_block = build_container_job(opts);
            let container_len = container_block.len();
            for i in 0..container_len {
                lines.insert(wf_pos, container_block[container_len - 1 - i].clone());
            }
            let binary_block = build_binary_release_job(opts);
            let binary_len = binary_block.len();
            for i in 0..binary_len {
                lines.insert(wf_pos, binary_block[binary_len - 1 - i].clone());
            }
            lines.insert(wf_pos, "jobs:".to_string());
        }
        report
            .insertions
            .push("build-binary-release and build-container jobs".to_string());
    }

    // 3. Add release workflow steps
    if content.contains("pack-orb-release")
        && content.contains("      - build-binary-release:")
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

fn build_binary_release_job(opts: &PatchOpts) -> Vec<String> {
    let binary = &opts.binary;
    vec![
        "  build-binary-release:".to_string(),
        "    docker:".to_string(),
        "      - image: rust:latest".to_string(),
        "    steps:".to_string(),
        "      - checkout".to_string(),
        "      - run:".to_string(),
        "          name: Build release binary".to_string(),
        format!("          command: cargo build --release -p {binary}"),
        "      - persist_to_workspace:".to_string(),
        "          root: target/release".to_string(),
        format!("          paths: [{binary}]"),
    ]
}

fn build_container_job(opts: &PatchOpts) -> Vec<String> {
    let binary = &opts.binary;
    let docker_ns = &opts.docker_namespace;
    let orb_dir = &opts.orb_dir;
    // Version is read from versions.env written by toolkit/calculate_versions.
    // The release pipeline is approval-triggered (not tag-triggered), so CIRCLE_TAG is empty.
    // The binary is attached from the build-binary-release workspace and copied into the
    // Docker build context so the image contains the freshly-compiled binary.
    let version_var = format!("CRATE_VERSION_{}", binary.to_uppercase().replace('-', "_"));
    vec![
        "  build-container:".to_string(),
        "    docker:".to_string(),
        "      - image: cimg/base:stable".to_string(),
        "    steps:".to_string(),
        "      - checkout".to_string(),
        "      - setup_remote_docker".to_string(),
        "      - attach_workspace:".to_string(),
        "          at: /tmp/release-versions".to_string(),
        "      - attach_workspace:".to_string(),
        "          at: /tmp/bin".to_string(),
        "      - run:".to_string(),
        "          name: Build and push Docker image".to_string(),
        "          command: |".to_string(),
        "            source /tmp/release-versions/versions.env".to_string(),
        format!("            VERSION=${{{version_var}}}"),
        format!("            cp /tmp/bin/{binary} {orb_dir}/{binary}"),
        format!("            docker build -t {docker_ns}/{binary}:${{VERSION}} -t {docker_ns}/{binary}:latest {orb_dir}"),
        "            echo \"${DOCKERHUB_PASSWORD}\" | docker login -u \"${DOCKERHUB_USERNAME}\" --password-stdin".to_string(),
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
    // release_after_job is the approval gate; both build-binary-release and pack-orb-release
    // require it so they run in parallel immediately after the gate clears.
    let requires = opts
        .release_after_job
        .as_deref()
        .map(|r| format!("[{r}]"))
        .unwrap_or_default();
    // Variable name: CRATE_VERSION_ + binary uppercased with hyphens replaced by underscores.
    // This matches the format written by toolkit/calculate_versions into versions.env.
    let version_var = format!("CRATE_VERSION_{}", binary.to_uppercase().replace('-', "_"));
    let mut steps = vec![];

    // build-binary-release: compile the release binary, persist to workspace
    steps.push("      - build-binary-release:".to_string());
    if !requires.is_empty() {
        steps.push(format!("          requires: {requires}"));
    }

    // Pack orb source (parallel with build-binary-release; both require the approval gate)
    steps.push("      - orb-tools/pack:".to_string());
    steps.push("          name: pack-orb-release".to_string());
    steps.push(format!("          source_dir: {orb_dir}/src"));
    if !requires.is_empty() {
        steps.push(format!("          requires: {requires}"));
    }

    // build-container: requires binary from workspace → sequential after build-binary-release
    steps.push("      - build-container:".to_string());
    steps.push("          requires: [build-binary-release]".to_string());
    steps.push(format!("          context: [{docker_ctx}]"));

    // orb-tools/publish: inject CIRCLE_TAG via pre-steps (pipeline is approval-triggered,
    // not tag-triggered, so CIRCLE_TAG is empty; inject it from versions.env)
    steps.push("      - orb-tools/publish:".to_string());
    steps.push(format!("          name: publish-orb-{namespace}"));
    steps.push("          pre-steps:".to_string());
    // Ensure the orb is registered before publishing. First release fails with
    // "Cannot find orb" if the orb has never been created. The orb-tools executor
    // has the CircleCI CLI available. Pattern: check first, create only if missing.
    // Using `orb info || orb create` (not `orb create || true`) so wrong-namespace/
    // wrong-token failures still surface rather than being silently swallowed.
    steps.push("            - run:".to_string());
    steps.push("                name: Ensure orb is registered".to_string());
    steps.push("                command: |".to_string());
    steps.push(format!(
        "                  circleci orb info {namespace}/{binary} > /dev/null 2>&1 || \\"
    ));
    steps.push(format!(
        "                    circleci orb create {namespace}/{binary} --no-prompt"
    ));
    steps.push("            - attach_workspace:".to_string());
    steps.push("                at: /tmp/release-versions".to_string());
    steps.push("            - run:".to_string());
    steps.push("                name: Export orb version as CIRCLE_TAG".to_string());
    steps.push("                command: |".to_string());
    steps.push("                  source /tmp/release-versions/versions.env".to_string());
    // orb-tools/publish requires CIRCLE_TAG to match ^v[0-9]+\.[0-9]+\.[0-9]+$
    steps.push(format!(
        "                  echo \"export CIRCLE_TAG=v${{{version_var}}}\" >> \"$BASH_ENV\""
    ));
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
            // The approval gate job that binary build and orb pack run after.
            // Docker/orb publish run before crates.io to establish the correct release order.
            release_after_job: Some("approve-release".to_string()),
            orb_tools_version: "12.3.3".to_string(),
            docker_orb_version: "3.0.1".to_string(),
            docker_context: "docker".to_string(),
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
        // Version comes from versions.env written by calculate_versions, not from git tags.
        // The release pipeline is approval-triggered (not tag-triggered), so CIRCLE_TAG is empty.
        assert!(
            !output.contains("${CIRCLE_TAG}"),
            "must not use CIRCLE_TAG (empty in approval-triggered pipelines):\n{output}"
        );
        assert!(
            output.contains("versions.env"),
            "must source versions.env to get the release version:\n{output}"
        );
        assert!(
            output.contains("attach_workspace"),
            "must attach workspace to get versions.env and the binary:\n{output}"
        );
        assert!(
            output.contains("docker login -u \"${DOCKERHUB_USERNAME}\""),
            "must log in to Docker Hub with DOCKERHUB_USERNAME before pushing:\n{output}"
        );
        assert!(
            output.contains("--password-stdin"),
            "must use --password-stdin (not -p flag) for Docker login:\n{output}"
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
        // both build-binary-release and build-container, not silently skip them.
        let (output, _) = patch_release(RELEASE_FIXTURE_NO_JOBS, &make_opts());
        assert!(
            output.contains("build-binary-release:"),
            "build-binary-release job definition missing when no pre-existing jobs section:\n{output}"
        );
        assert!(
            output.contains("build-container:"),
            "build-container job definition missing when no pre-existing jobs section:\n{output}"
        );
        let jobs_pos = output.find("\njobs:").expect("jobs: section not created");
        let workflows_pos = output.find("\nworkflows:").expect("no workflows: section");
        let binary_release_pos = output
            .find("  build-binary-release:")
            .expect("no build-binary-release job definition");
        let container_pos = output
            .find("  build-container:")
            .expect("no build-container job definition");
        assert!(
            binary_release_pos > jobs_pos && binary_release_pos < workflows_pos,
            "build-binary-release definition must be in jobs: section, not workflows:\n{output}"
        );
        assert!(
            container_pos > jobs_pos && container_pos < workflows_pos,
            "build-container definition must be in jobs: section, not workflows:\n{output}"
        );
    }

    // ── new: build-binary-release job in release.yml ──────────────────────────

    #[test]
    fn patch_release_adds_build_binary_release_job() {
        let (output, _) = patch_release(RELEASE_FIXTURE, &make_opts());
        assert!(
            output.contains("  build-binary-release:"),
            "missing build-binary-release job definition:\n{output}"
        );
        let jobs_pos = output.find("\njobs:").expect("no jobs: section");
        let workflows_pos = output.find("\nworkflows:").expect("no workflows: section");
        let pos = output
            .find("  build-binary-release:")
            .expect("no build-binary-release job");
        assert!(
            pos > jobs_pos && pos < workflows_pos,
            "build-binary-release must be in the jobs section:\n{output}"
        );
    }

    #[test]
    fn build_binary_release_job_uses_rust_latest() {
        let (output, _) = patch_release(RELEASE_FIXTURE, &make_opts());
        let block = job_block(&output, "build-binary-release");
        assert!(
            block.contains("rust:latest"),
            "build-binary-release must use the public rust:latest image:\n{block}"
        );
    }

    #[test]
    fn build_binary_release_job_has_package_flag() {
        let (output, _) = patch_release(RELEASE_FIXTURE, &make_opts());
        let block = job_block(&output, "build-binary-release");
        assert!(
            block.contains("cargo build --release -p mytool"),
            "build-binary-release must compile with -p <binary> flag:\n{block}"
        );
    }

    #[test]
    fn build_binary_release_job_persists_binary_to_workspace() {
        let (output, _) = patch_release(RELEASE_FIXTURE, &make_opts());
        let block = job_block(&output, "build-binary-release");
        assert!(
            block.contains("persist_to_workspace"),
            "build-binary-release must persist binary to workspace:\n{block}"
        );
        assert!(
            block.contains("paths: [mytool]"),
            "build-binary-release must persist the binary by name:\n{block}"
        );
    }

    // ── new: build-container uses versions.env + workspace binary ─────────────

    #[test]
    fn build_container_sources_versions_env() {
        let (output, _) = patch_release(RELEASE_FIXTURE, &make_opts());
        let block = job_block(&output, "build-container");
        assert!(
            block.contains("source /tmp/release-versions/versions.env"),
            "build-container must source versions.env to get the release version:\n{block}"
        );
    }

    #[test]
    fn build_container_attaches_release_versions_workspace() {
        let (output, _) = patch_release(RELEASE_FIXTURE, &make_opts());
        let block = job_block(&output, "build-container");
        assert!(
            block.contains("at: /tmp/release-versions"),
            "build-container must attach the release-versions workspace:\n{block}"
        );
    }

    #[test]
    fn build_container_attaches_bin_workspace() {
        let (output, _) = patch_release(RELEASE_FIXTURE, &make_opts());
        let block = job_block(&output, "build-container");
        assert!(
            block.contains("at: /tmp/bin"),
            "build-container must attach the bin workspace to get the compiled binary:\n{block}"
        );
    }

    #[test]
    fn build_container_copies_workspace_binary() {
        let (output, _) = patch_release(RELEASE_FIXTURE, &make_opts());
        let block = job_block(&output, "build-container");
        assert!(
            block.contains("cp /tmp/bin/mytool orb/mytool"),
            "build-container must copy binary from workspace into the Docker build context:\n{block}"
        );
    }

    #[test]
    fn build_container_uses_crate_version_var() {
        let (output, _) = patch_release(RELEASE_FIXTURE, &make_opts());
        let block = job_block(&output, "build-container");
        // Variable name formula: CRATE_VERSION_ + binary.to_uppercase().replace('-', '_')
        // mytool → CRATE_VERSION_MYTOOL
        assert!(
            block.contains("CRATE_VERSION_MYTOOL"),
            "build-container must derive version from CRATE_VERSION_<BINARY> env var:\n{block}"
        );
    }

    // ── new: release workflow step ordering ──────────────────────────────────

    #[test]
    fn release_workflow_has_build_binary_release_step() {
        let (output, _) = patch_release(RELEASE_FIXTURE, &make_opts());
        assert!(
            output.contains("      - build-binary-release:"),
            "missing build-binary-release workflow step:\n{output}"
        );
    }

    #[test]
    fn release_workflow_binary_and_pack_both_require_approval_gate() {
        let (output, _) = patch_release(RELEASE_FIXTURE, &make_opts());
        // build-binary-release step block
        let after_binary = output
            .split("      - build-binary-release:")
            .nth(1)
            .expect("no build-binary-release workflow step");
        let binary_step = after_binary
            .split("      - ")
            .next()
            .unwrap_or(after_binary);
        assert!(
            binary_step.contains("requires: [approve-release]"),
            "build-binary-release workflow step must require the approval gate:\n{binary_step}"
        );
        // pack-orb-release step block
        let after_pack = output
            .split("name: pack-orb-release")
            .nth(1)
            .expect("no pack-orb-release step");
        let pack_step = after_pack.split("      - ").next().unwrap_or(after_pack);
        assert!(
            pack_step.contains("requires: [approve-release]"),
            "pack-orb-release step must require the approval gate (runs in parallel with build-binary-release):\n{pack_step}"
        );
    }

    #[test]
    fn release_workflow_container_requires_build_binary_release() {
        let (output, _) = patch_release(RELEASE_FIXTURE, &make_opts());
        let after_container = output
            .split("      - build-container:")
            .nth(1)
            .expect("no build-container workflow step");
        let step_block = after_container
            .split("      - ")
            .next()
            .unwrap_or(after_container);
        assert!(
            step_block.contains("requires: [build-binary-release]"),
            "build-container must require build-binary-release (not the approval gate directly):\n{step_block}"
        );
    }

    #[test]
    fn release_workflow_publish_has_pre_steps_circle_tag_injection() {
        let (output, _) = patch_release(RELEASE_FIXTURE, &make_opts());
        // orb-tools/publish requires CIRCLE_TAG for pub_type: production.
        // Since the pipeline is approval-triggered (not tag-triggered), CIRCLE_TAG must be
        // injected via pre-steps from versions.env.
        assert!(
            output.contains("pre-steps:"),
            "orb-tools/publish must have pre-steps to inject CIRCLE_TAG:\n{output}"
        );
        assert!(
            output.contains("Export orb version as CIRCLE_TAG"),
            "pre-steps must contain a named step that exports CIRCLE_TAG:\n{output}"
        );
        assert!(
            output.contains("CIRCLE_TAG"),
            "pre-steps must set CIRCLE_TAG in BASH_ENV:\n{output}"
        );
        assert!(
            output.contains("BASH_ENV"),
            "pre-steps must export CIRCLE_TAG via BASH_ENV:\n{output}"
        );
    }

    #[test]
    fn release_workflow_publish_pre_steps_use_versions_env() {
        let (output, _) = patch_release(RELEASE_FIXTURE, &make_opts());
        // The pre-steps must source versions.env from the workspace to get the version.
        // The workspace is attached first, then the env file sourced.
        assert!(
            output.contains("/tmp/release-versions"),
            "publish pre-steps must attach the release-versions workspace:\n{output}"
        );
        assert!(
            output.contains("source /tmp/release-versions/versions.env"),
            "publish pre-steps must source versions.env to read CRATE_VERSION_*:\n{output}"
        );
    }

    #[test]
    fn release_workflow_publish_pre_steps_inject_correct_variable() {
        let (output, _) = patch_release(RELEASE_FIXTURE, &make_opts());
        // Variable name: CRATE_VERSION_ + binary.to_uppercase().replace('-', '_')
        // mytool → CRATE_VERSION_MYTOOL
        // orb-tools/publish pub_type:production requires CIRCLE_TAG matching ^v[0-9]+\.[0-9]+\.[0-9]+$
        // so the v prefix is mandatory.
        assert!(
            output.contains("CIRCLE_TAG=v${CRATE_VERSION_MYTOOL}"),
            "CIRCLE_TAG must have a v prefix — orb-tools/publish requires tag pattern ^v\\d+\\.\\d+\\.\\d+$:\n{output}"
        );
    }

    // ── new: ensure-orb-registered pre-step ──────────────────────────────────

    #[test]
    fn release_workflow_publish_pre_steps_ensure_orb_registered() {
        let (output, _) = patch_release(RELEASE_FIXTURE, &make_opts());
        // orb-tools/publish fails with "Cannot find orb" on first release if the orb has
        // never been created. A pre-step checks with `circleci orb info` and creates it
        // via `circleci orb create` if missing. The orb-tools executor has the CLI available.
        assert!(
            output.contains("Ensure orb is registered"),
            "publish pre-steps must include an ensure-orb-registered step:\n{output}"
        );
        assert!(
            output.contains("circleci orb info"),
            "ensure step must check whether the orb exists before creating:\n{output}"
        );
        assert!(
            output.contains("circleci orb create"),
            "ensure step must create the orb if it does not exist:\n{output}"
        );
    }

    #[test]
    fn release_workflow_publish_ensure_orb_uses_idempotent_pattern() {
        let (output, _) = patch_release(RELEASE_FIXTURE, &make_opts());
        // Pattern: `circleci orb info <ns>/<bin> > /dev/null 2>&1 || circleci orb create ...`
        // First run: orb info exits non-zero → orb create runs.
        // Subsequent runs: orb info exits 0 → orb create is skipped.
        // Real failures (wrong token, wrong namespace) still surface because they
        // fail the `orb info` side, not a silent `|| true`.
        assert!(
            output.contains("circleci orb info my-org/mytool"),
            "ensure step must check the specific orb (namespace/binary):\n{output}"
        );
        assert!(
            output.contains("circleci orb create my-org/mytool --no-prompt"),
            "ensure step must create the specific orb with --no-prompt:\n{output}"
        );
    }

    #[test]
    fn release_workflow_publish_ensure_orb_not_silent_on_wrong_namespace() {
        let (output, _) = patch_release(RELEASE_FIXTURE, &make_opts());
        // `|| true` would hide wrong-namespace/wrong-token failures.
        // The correct pattern uses `orb info || orb create`, NOT `orb create || true`.
        // We verify by checking that `|| true` does NOT appear in the ensure step block.
        let after_ensure = output
            .split("Ensure orb is registered")
            .nth(1)
            .expect("no ensure step");
        let ensure_block = after_ensure.split("name:").next().unwrap_or(after_ensure);
        assert!(
            !ensure_block.contains("|| true"),
            "ensure step must not use `|| true` — real failures must surface:\n{ensure_block}"
        );
    }
}
