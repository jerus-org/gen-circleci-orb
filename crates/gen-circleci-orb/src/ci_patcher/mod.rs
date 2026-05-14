use anyhow::Result;

pub struct PatchOpts {
    pub binary: String,
    /// One or more CircleCI namespaces to publish the orb under.
    /// Each namespace gets its own `orb-release-ensure-registered-<ns>` job and
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
    /// A namespace listed here gets `--private` in its `circleci orb create` command.
    /// Namespaces not listed are registered as public.
    /// Visibility is set at orb creation time and cannot be changed afterwards.
    pub private_namespaces: Vec<String>,
    pub mcp: bool,
    /// Version of the jerus-org/gen-orb-mcp orb to pin in the orbs section.
    /// Only used when `mcp` is true.
    pub gen_orb_mcp_version: String,
    /// CircleCI context providing push authority for MCP server publish + save steps.
    /// Only used when `mcp` is true.
    pub mcp_context: String,
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

    patch_step1_orb_tools(content, &mut lines, opts, &mut report);
    patch_step2_build_regen_jobs(content, &mut lines, opts, &mut report);
    patch_step3_pack_validate(content, &mut lines, opts, &mut report);
    patch_step4_orb_release_jobs(content, &mut lines, opts, &mut report);
    patch_step5_orb_release_workflow(content, &mut lines, opts, &mut report);
    patch_step6_mcp_orb(content, &mut lines, opts, &mut report);
    patch_step7_mcp_binary_job(content, &mut lines, opts, &mut report);

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
    lines: &mut Vec<String>,
    opts: &PatchOpts,
    report: &mut PatchReport,
) {
    if content.contains("build-binary:") && content.contains("regenerate-orb:") {
        report
            .skipped
            .push("build-binary and regenerate-orb jobs".to_string());
        return;
    }
    let build_block = build_binary_job(opts);
    let regen_block = regenerate_orb_job(opts);
    if let Some(pos) = find_section_end(lines, "jobs:") {
        insert_block_at(lines, pos, &build_block);
        insert_block_at(lines, pos + build_block.len(), &regen_block);
    } else {
        insert_jobs_before_workflows(lines, &build_block, &regen_block);
    }
    report
        .insertions
        .push("build-binary and regenerate-orb jobs".to_string());
}

fn insert_jobs_before_workflows(
    lines: &mut Vec<String>,
    build_block: &[String],
    regen_block: &[String],
) {
    let Some(wf_pos) = find_top_level(lines, "workflows:") else {
        return;
    };
    let mut block = vec!["jobs:".to_string()];
    block.extend_from_slice(build_block);
    block.extend_from_slice(regen_block);
    block.push(String::new());
    block.push(String::new());
    for (i, l) in block.into_iter().enumerate() {
        lines.insert(wf_pos + i, l);
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

fn patch_step4_orb_release_jobs(
    content: &str,
    lines: &mut Vec<String>,
    opts: &PatchOpts,
    report: &mut PatchReport,
) {
    let has_binary = content.contains("orb-release-binary:");
    let has_container = content.contains("orb-release-container:");
    let missing_ns: Vec<&String> = opts
        .namespaces
        .iter()
        .filter(|ns| !content.contains(&format!("  orb-release-ensure-registered-{ns}:")))
        .collect();
    if has_binary && has_container && missing_ns.is_empty() {
        report.skipped.push("orb-release jobs".to_string());
        return;
    }
    let Some(pos) = find_section_end(lines, "jobs:") else {
        return;
    };
    let mut insert_pos = pos;
    if !has_binary {
        let block = orb_release_binary_job(opts);
        let len = block.len();
        insert_block_at(lines, insert_pos, &block);
        insert_pos += len;
    }
    if !has_container {
        let block = orb_release_container_job(opts);
        let len = block.len();
        insert_block_at(lines, insert_pos, &block);
        insert_pos += len;
    }
    for ns in &missing_ns {
        let block = orb_release_ensure_registered_job_for(
            ns,
            &opts.binary,
            opts.private_namespaces.contains(ns),
        );
        let len = block.len();
        insert_block_at(lines, insert_pos, &block);
        insert_pos += len;
    }
    report.insertions.push("orb-release jobs".to_string());
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

fn patch_step6_mcp_orb(
    content: &str,
    lines: &mut Vec<String>,
    opts: &PatchOpts,
    report: &mut PatchReport,
) {
    if !opts.mcp {
        return;
    }
    let version = &opts.gen_orb_mcp_version;
    let orb_entry = format!("  gen-orb-mcp: jerus-org/gen-orb-mcp@{version}");
    if content.contains("gen-orb-mcp:") {
        report.skipped.push("gen-orb-mcp orb".to_string());
    } else if let Some(pos) = find_section_end(lines, "orbs:") {
        lines.insert(pos, orb_entry);
        report.insertions.push("gen-orb-mcp orb".to_string());
    }
}

fn patch_step7_mcp_binary_job(
    content: &str,
    lines: &mut Vec<String>,
    opts: &PatchOpts,
    report: &mut PatchReport,
) {
    if !opts.mcp {
        return;
    }
    if content.contains("  build-mcp-binary:") {
        report.skipped.push("build-mcp-binary job".to_string());
        return;
    }
    let Some(pos) = find_section_end(lines, "jobs:") else {
        return;
    };
    let block = build_mcp_binary_job(opts);
    insert_block_at(lines, pos, &block);
    report.insertions.push("build-mcp-binary job".to_string());
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
    let orb_dir = &opts.orb_dir;
    // gen-circleci-orb is pre-installed in its own Docker image (jerusdp/gen-circleci-orb).
    // The target binary is attached from the build-binary workspace — no runtime installs needed.
    let mut lines = vec![
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
    ];
    // Each namespace gets its own --orb-namespace flag (repeatable CLI arg).
    for ns in &opts.namespaces {
        lines.push(format!("              --orb-namespace {ns} \\"));
    }
    lines.push(format!("              --orb-dir {orb_dir}"));
    lines
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

// ── orb-release helpers (tag-triggered, lives in config.yml) ─────────────────

fn build_mcp_binary_job(opts: &PatchOpts) -> Vec<String> {
    let binary = &opts.binary;
    vec![
        "  build-mcp-binary:".to_string(),
        "    docker:".to_string(),
        "      - image: rust:latest".to_string(),
        "    steps:".to_string(),
        "      - attach_workspace:".to_string(),
        "          at: /tmp/mcp-src".to_string(),
        "      - run:".to_string(),
        "          name: Build MCP server binary".to_string(),
        "          command: cargo build --release".to_string(),
        "          working_directory: /tmp/mcp-src".to_string(),
        "      - persist_to_workspace:".to_string(),
        "          root: /tmp/mcp-src/target/release".to_string(),
        format!("          paths: [{binary}-mcp]"),
    ]
}

fn orb_release_binary_job(opts: &PatchOpts) -> Vec<String> {
    let binary = &opts.binary;
    vec![
        "  orb-release-binary:".to_string(),
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

fn orb_release_container_job(opts: &PatchOpts) -> Vec<String> {
    let binary = &opts.binary;
    let docker_ns = &opts.docker_namespace;
    let orb_dir = &opts.orb_dir;
    let prefix = &opts.crate_tag_prefix;
    // Tag-triggered pipeline: CIRCLE_TAG is set by CircleCI when the tag filter matches.
    // Strip the crate tag prefix to get the bare semver (e.g. "gen-orb-mcp-v0.1.5" → "0.1.5").
    vec![
        "  orb-release-container:".to_string(),
        "    docker:".to_string(),
        "      - image: cimg/base:stable".to_string(),
        "    steps:".to_string(),
        "      - checkout".to_string(),
        "      - setup_remote_docker".to_string(),
        "      - attach_workspace:".to_string(),
        "          at: /tmp/bin".to_string(),
        "      - run:".to_string(),
        "          name: Build and push Docker image".to_string(),
        "          command: |".to_string(),
        format!("            VERSION=${{CIRCLE_TAG#{prefix}}}"),
        format!("            cp /tmp/bin/{binary} {orb_dir}/{binary}"),
        format!("            docker build -t {docker_ns}/{binary}:${{VERSION}} -t {docker_ns}/{binary}:latest {orb_dir}"),
        "            echo \"${DOCKERHUB_PASSWORD}\" | docker login -u \"${DOCKERHUB_USERNAME}\" --password-stdin".to_string(),
        format!("            docker push {docker_ns}/{binary}:${{VERSION}}"),
        format!("            docker push {docker_ns}/{binary}:latest"),
    ]
}

/// Build the `orb-release-ensure-registered-<ns>` job definition for config.yml.
/// Identical logic to `ensure_orb_registered_job_for` but with the `orb-release-` prefix
/// so it doesn't conflict with any existing jobs in the same file.
fn orb_release_ensure_registered_job_for(ns: &str, binary: &str, private: bool) -> Vec<String> {
    let create_flags = if private {
        "--private --no-prompt"
    } else {
        "--no-prompt"
    };
    vec![
        format!("  orb-release-ensure-registered-{ns}:"),
        "    executor: orb-tools/default".to_string(),
        "    steps:".to_string(),
        "      - run:".to_string(),
        "          name: Ensure orb is registered".to_string(),
        "          command: |".to_string(),
        "            set +e".to_string(),
        "            circleci setup --token \"${CIRCLE_TOKEN}\" --host https://circleci.com --no-prompt".to_string(),
        "            setup_exit=$?".to_string(),
        "            set -e".to_string(),
        "            if [ \"${setup_exit}\" -ne 0 ] && [ \"${setup_exit}\" -ne 255 ]; then".to_string(),
        "              echo \"circleci setup failed with exit ${setup_exit}\" >&2".to_string(),
        "              exit \"${setup_exit}\"".to_string(),
        "            fi".to_string(),
        "            set +e".to_string(),
        format!("            circleci orb info {ns}/{binary}"),
        "            orb_info_exit=$?".to_string(),
        "            set -e".to_string(),
        "            echo \"orb info exit: ${orb_info_exit}\"".to_string(),
        "            if [ \"${orb_info_exit}\" -ne 0 ] && [ \"${orb_info_exit}\" -ne 255 ]; then".to_string(),
        "              set +e".to_string(),
        format!("              create_output=$(circleci orb create {ns}/{binary} {create_flags} 2>&1)"),
        "              create_exit=$?".to_string(),
        "              set -e".to_string(),
        "              echo \"${create_output}\"".to_string(),
        "              if [ \"${create_exit}\" -ne 0 ] && ! echo \"${create_output}\" | grep -q \"already exists\"; then".to_string(),
        "                exit \"${create_exit}\"".to_string(),
        "              fi".to_string(),
        "            fi".to_string(),
        "            echo \"Orb is registered.\"".to_string(),
    ]
}

fn push_tag_filters(lines: &mut Vec<String>, only_tag: &str, ignore_branches: &str) {
    lines.push("          filters:".to_string());
    lines.push("            tags:".to_string());
    lines.push(only_tag.to_string());
    lines.push("            branches:".to_string());
    lines.push(ignore_branches.to_string());
}

/// Generate the complete `orb-release:` workflow section for config.yml.
/// This is a tag-triggered workflow (filters on `crate_tag_prefix*`; ignores all branches).
fn orb_release_workflow_section(opts: &PatchOpts) -> Vec<String> {
    let prefix = &opts.crate_tag_prefix;
    let orb_dir = &opts.orb_dir;
    let docker_ctx = &opts.docker_context;
    let orb_ctx = &opts.orb_context;
    let binary = &opts.binary;

    let only_tag = format!("              only: /^{prefix}.*/");
    let ignore_branches = "              ignore: /.*/".to_string();

    let mut lines = vec![
        String::new(), // blank line before new workflow
        "  orb-release:".to_string(),
        "    jobs:".to_string(),
        "      - orb-release-binary:".to_string(),
    ];
    push_tag_filters(&mut lines, &only_tag, &ignore_branches);
    lines.push(String::new());

    // orb-tools/pack (checkout: false + pre-steps for version injection)
    lines.push("      - orb-tools/pack:".to_string());
    lines.push("          name: orb-release-pack".to_string());
    lines.push("          checkout: false".to_string());
    lines.push(format!("          source_dir: {orb_dir}/src"));
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

    // orb-release-container
    lines.push("      - orb-release-container:".to_string());
    lines.push("          requires: [orb-release-binary]".to_string());
    lines.push(format!("          context: [{docker_ctx}]"));
    push_tag_filters(&mut lines, &only_tag, &ignore_branches);
    lines.push(String::new());

    // Per-namespace: ensure + publish
    for ns in &opts.namespaces {
        lines.push(format!("      - orb-release-ensure-registered-{ns}:"));
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
    let mcp_ctx = &opts.mcp_context;

    // Build the requires list from all publish-orb-<ns> steps
    let requires: Vec<String> = opts
        .namespaces
        .iter()
        .map(|ns| format!("publish-orb-{ns}"))
        .collect();
    let requires_str = requires.join(", ");

    // gen-orb-mcp/generate — generates MCP server source; persists to workspace
    lines.push("      - gen-orb-mcp/generate:".to_string());
    lines.push("          name: generate-mcp-server".to_string());
    lines.push(format!("          orb_path: {orb_dir}/src/@orb.yml"));
    lines.push(format!("          generate_name: {binary}-mcp"));
    lines.push("          force: true".to_string());
    lines.push(format!("          tag_prefix: {prefix}"));
    lines.push("          post-steps:".to_string());
    lines.push("            - persist_to_workspace:".to_string());
    lines.push("                root: dist".to_string());
    lines.push("                paths: [.]".to_string());
    lines.push(format!("          requires: [{requires_str}]"));
    push_tag_filters(lines, only_tag, ignore_branches);
    lines.push(String::new());

    // build-mcp-binary — inline Rust compile job (uses rust:latest executor)
    lines.push("      - build-mcp-binary:".to_string());
    lines.push("          requires: [generate-mcp-server]".to_string());
    push_tag_filters(lines, only_tag, ignore_branches);
    lines.push(String::new());

    // gen-orb-mcp/publish — uploads binary to GitHub release
    lines.push("      - gen-orb-mcp/publish:".to_string());
    lines.push("          name: publish-mcp-server".to_string());
    lines.push(format!("          binary: /tmp/bin/{binary}-mcp"));
    lines.push(format!("          asset_name: {binary}-mcp-linux-x86_64"));
    lines.push("          pre-steps:".to_string());
    lines.push("            - attach_workspace:".to_string());
    lines.push("                at: /tmp/bin".to_string());
    lines.push("          requires: [build-mcp-binary]".to_string());
    lines.push(format!("          context: [{mcp_ctx}]"));
    push_tag_filters(lines, only_tag, ignore_branches);
    lines.push(String::new());

    // gen-orb-mcp/save — commits generated source back to the repo
    lines.push("      - gen-orb-mcp/save:".to_string());
    lines.push("          name: save-mcp-server".to_string());
    lines.push("          paths: dist".to_string());
    lines.push("          pre-steps:".to_string());
    lines.push("            - attach_workspace:".to_string());
    lines.push("                at: dist".to_string());
    lines.push("          requires: [generate-mcp-server]".to_string());
    lines.push(format!("          context: [{mcp_ctx}]"));
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
            mcp: false,
            gen_orb_mcp_version: "0.1.13".to_string(),
            mcp_context: "pcu-app".to_string(),
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

    #[test]
    fn patch_build_adds_orb_release_binary_job_to_jobs_section() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        assert!(
            output.contains("  orb-release-binary:"),
            "missing orb-release-binary job definition:\n{output}"
        );
        let jobs_pos = output.find("\njobs:").expect("no jobs: section");
        let workflows_pos = output.find("\nworkflows:").expect("no workflows: section");
        let pos = output
            .find("  orb-release-binary:")
            .expect("no orb-release-binary job");
        assert!(
            pos > jobs_pos && pos < workflows_pos,
            "orb-release-binary must be defined in the jobs section:\n{output}"
        );
    }

    #[test]
    fn orb_release_binary_job_uses_cargo_build_with_package_flag() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let block = job_block(&output, "orb-release-binary");
        assert!(
            block.contains("cargo build --release -p mytool"),
            "orb-release-binary must compile with -p <binary>:\n{block}"
        );
        assert!(
            block.contains("rust:latest"),
            "orb-release-binary must use public rust:latest image:\n{block}"
        );
    }

    #[test]
    fn orb_release_binary_job_persists_binary_to_workspace() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let block = job_block(&output, "orb-release-binary");
        assert!(
            block.contains("persist_to_workspace"),
            "orb-release-binary must persist binary to workspace:\n{block}"
        );
        assert!(
            block.contains("paths: [mytool]"),
            "orb-release-binary must persist the binary by name:\n{block}"
        );
    }

    #[test]
    fn patch_build_adds_orb_release_container_job_to_jobs_section() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        assert!(
            output.contains("  orb-release-container:"),
            "missing orb-release-container job definition:\n{output}"
        );
        let jobs_pos = output.find("\njobs:").expect("no jobs: section");
        let workflows_pos = output.find("\nworkflows:").expect("no workflows: section");
        let pos = output
            .find("  orb-release-container:")
            .expect("no orb-release-container job");
        assert!(
            pos > jobs_pos && pos < workflows_pos,
            "orb-release-container must be in jobs section:\n{output}"
        );
    }

    #[test]
    fn orb_release_container_uses_circle_tag_not_versions_env() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let block = job_block(&output, "orb-release-container");
        // Tag-triggered pipeline: version comes from CIRCLE_TAG, not versions.env
        assert!(
            block.contains("CIRCLE_TAG"),
            "orb-release-container must derive version from CIRCLE_TAG:\n{block}"
        );
        assert!(
            !block.contains("versions.env"),
            "orb-release-container must NOT use versions.env (approval-triggered pattern):\n{block}"
        );
        // Strip the crate tag prefix to get the bare semver
        assert!(
            block.contains("CIRCLE_TAG#mytool-v"),
            "orb-release-container must strip crate_tag_prefix from CIRCLE_TAG:\n{block}"
        );
    }

    #[test]
    fn orb_release_container_copies_workspace_binary() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let block = job_block(&output, "orb-release-container");
        assert!(
            block.contains("attach_workspace"),
            "orb-release-container must attach workspace to get the binary:\n{block}"
        );
        assert!(
            block.contains("at: /tmp/bin"),
            "orb-release-container must attach workspace at /tmp/bin:\n{block}"
        );
        assert!(
            block.contains("cp /tmp/bin/mytool orb/mytool"),
            "orb-release-container must copy binary from workspace into Docker build context:\n{block}"
        );
    }

    #[test]
    fn orb_release_container_pushes_versioned_and_latest_tags() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let block = job_block(&output, "orb-release-container");
        assert!(
            block.contains("docker push my-docker-org/mytool"),
            "orb-release-container must push Docker image:\n{block}"
        );
        assert!(
            block.contains(":latest"),
            "orb-release-container must push a :latest tag:\n{block}"
        );
        assert!(
            block.contains("docker login -u \"${DOCKERHUB_USERNAME}\""),
            "orb-release-container must login to Docker Hub:\n{block}"
        );
        assert!(
            block.contains("--password-stdin"),
            "orb-release-container must use --password-stdin:\n{block}"
        );
    }

    #[test]
    fn patch_build_adds_orb_release_ensure_registered_jobs() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        assert!(
            output.contains("  orb-release-ensure-registered-my-org:"),
            "missing orb-release-ensure-registered-my-org job definition:\n{output}"
        );
        let jobs_pos = output.find("\njobs:").expect("no jobs: section");
        let workflows_pos = output.find("\nworkflows:").expect("no workflows: section");
        let pos = output
            .find("  orb-release-ensure-registered-my-org:")
            .expect("no orb-release-ensure-registered-my-org job");
        assert!(
            pos > jobs_pos && pos < workflows_pos,
            "orb-release-ensure-registered-my-org must be in jobs section:\n{output}"
        );
    }

    #[test]
    fn orb_release_ensure_registered_job_has_correct_circleci_commands() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        let block = job_block(&output, "orb-release-ensure-registered-my-org");
        assert!(
            block.contains("executor: orb-tools/default"),
            "orb-release-ensure-registered must use orb-tools/default executor:\n{block}"
        );
        assert!(
            block.contains("circleci setup") && block.contains("CIRCLE_TOKEN"),
            "ensure job must run circleci setup --token ${{CIRCLE_TOKEN}}:\n{block}"
        );
        assert!(
            block.contains("circleci orb info my-org/mytool"),
            "ensure job must check if orb is registered:\n{block}"
        );
        assert!(
            block.contains("set +e") && block.contains("set -e"),
            "ensure job must use set +e / set -e for circleci CLI exit-255 handling:\n{block}"
        );
        assert!(
            block.contains("-ne 255"),
            "ensure job must accept exit 255 as non-failure:\n{block}"
        );
        assert!(
            block.contains("already exists"),
            "ensure job must handle 'already exists' gracefully:\n{block}"
        );
    }

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
            output.contains("  orb-release-binary:"),
            "orb-release-binary job missing when no pre-existing jobs section:\n{output}"
        );
        assert!(
            output.contains("  orb-release-container:"),
            "orb-release-container job missing when no pre-existing jobs section:\n{output}"
        );
        assert!(
            output.contains("  orb-release:"),
            "orb-release workflow section missing when no pre-existing jobs section:\n{output}"
        );
    }

    #[test]
    fn patch_build_per_namespace_orb_release_ensure_and_publish() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts_two_ns());
        // Both namespaces get their own ensure job and publish step
        assert!(
            output.contains("  orb-release-ensure-registered-my-org:"),
            "missing ensure job for my-org:\n{output}"
        );
        assert!(
            output.contains("  orb-release-ensure-registered-other-org:"),
            "missing ensure job for other-org:\n{output}"
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

    // ── --mcp: gen-orb-mcp orb integration ───────────────────────────────────

    #[test]
    fn patch_build_mcp_disabled_does_not_add_gen_orb_mcp_orb() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts());
        assert!(
            !output.contains("gen-orb-mcp: jerus-org"),
            "gen-orb-mcp orb must not appear when --mcp is false:\n{output}"
        );
    }

    #[test]
    fn patch_build_mcp_adds_gen_orb_mcp_to_orbs_section() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts_mcp());
        assert!(
            output.contains("gen-orb-mcp: jerus-org/gen-orb-mcp@0.1.13"),
            "missing gen-orb-mcp orb entry:\n{output}"
        );
        let orbs_pos = output.find("orbs:").expect("no orbs: section");
        let jobs_pos = output.find("\njobs:").expect("no jobs: section");
        let pos = output
            .find("gen-orb-mcp: jerus-org")
            .expect("no gen-orb-mcp entry");
        assert!(
            pos > orbs_pos && pos < jobs_pos,
            "gen-orb-mcp orb must be inside orbs section:\n{output}"
        );
    }

    #[test]
    fn patch_build_mcp_orb_entry_is_idempotent() {
        let (first, _) = patch_build(BUILD_FIXTURE, &make_opts_mcp());
        let (second, second_report) = patch_build(&first, &make_opts_mcp());
        assert_eq!(first, second, "second mcp patch changed content");
        assert!(
            second_report
                .skipped
                .iter()
                .any(|s| s.contains("gen-orb-mcp")),
            "expected gen-orb-mcp skipped on second run:\n{:?}",
            second_report.skipped
        );
    }

    #[test]
    fn patch_build_mcp_adds_build_mcp_binary_job_to_jobs_section() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts_mcp());
        assert!(
            output.contains("  build-mcp-binary:"),
            "missing build-mcp-binary job definition:\n{output}"
        );
        let jobs_pos = output.find("\njobs:").expect("no jobs: section");
        let workflows_pos = output.find("\nworkflows:").expect("no workflows: section");
        let pos = output
            .find("  build-mcp-binary:")
            .expect("no build-mcp-binary job");
        assert!(
            pos > jobs_pos && pos < workflows_pos,
            "build-mcp-binary must be in the jobs section:\n{output}"
        );
    }

    #[test]
    fn patch_build_mcp_build_binary_job_uses_rust_image() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts_mcp());
        let block = job_block(&output, "build-mcp-binary");
        assert!(
            block.contains("rust:latest"),
            "build-mcp-binary must use rust:latest (gen-orb-mcp executor has no cargo):\n{block}"
        );
    }

    #[test]
    fn patch_build_mcp_adds_generate_step_to_orb_release_workflow() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts_mcp());
        let after_wf = output
            .split("  orb-release:")
            .nth(1)
            .expect("no orb-release workflow");
        assert!(
            after_wf.contains("gen-orb-mcp/generate:"),
            "missing generate-mcp-server step:\n{after_wf}"
        );
        assert!(
            after_wf.contains("name: generate-mcp-server"),
            "generate step must be named generate-mcp-server:\n{after_wf}"
        );
        assert!(
            after_wf.contains("force: true"),
            "generate step must set force: true for non-interactive CI:\n{after_wf}"
        );
        assert!(
            after_wf.contains("tag_prefix: mytool-v"),
            "generate step must pass crate_tag_prefix as tag_prefix:\n{after_wf}"
        );
        assert!(
            after_wf.contains("orb/src/@orb.yml"),
            "generate step must reference orb_path:\n{after_wf}"
        );
    }

    #[test]
    fn patch_build_mcp_generate_requires_all_publish_orb_steps() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts_mcp());
        let after_generate = output
            .split("name: generate-mcp-server")
            .nth(1)
            .expect("no generate step");
        let step_block = after_generate
            .split("\n      - ")
            .next()
            .unwrap_or(after_generate);
        assert!(
            step_block.contains("requires: [publish-orb-my-org]"),
            "generate step must require publish-orb steps:\n{step_block}"
        );
    }

    #[test]
    fn patch_build_mcp_generate_requires_all_namespaces_multi_ns() {
        let opts = PatchOpts {
            mcp: true,
            namespaces: vec!["ns-a".to_string(), "ns-b".to_string()],
            ..make_opts()
        };
        let (output, _) = patch_build(BUILD_FIXTURE, &opts);
        let after_generate = output
            .split("name: generate-mcp-server")
            .nth(1)
            .expect("no generate step");
        let step_block = after_generate
            .split("\n      - ")
            .next()
            .unwrap_or(after_generate);
        assert!(
            step_block.contains("publish-orb-ns-a") && step_block.contains("publish-orb-ns-b"),
            "generate step must require all publish-orb steps:\n{step_block}"
        );
    }

    #[test]
    fn patch_build_mcp_adds_build_workflow_step() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts_mcp());
        let after_wf = output
            .split("  orb-release:")
            .nth(1)
            .expect("no orb-release workflow");
        assert!(
            after_wf.contains("- build-mcp-binary:"),
            "missing build-mcp-binary workflow step:\n{after_wf}"
        );
        let after_build_step = after_wf
            .split("- build-mcp-binary:")
            .nth(1)
            .expect("no build-mcp-binary step");
        let step_block = after_build_step
            .split("\n      - ")
            .next()
            .unwrap_or(after_build_step);
        assert!(
            step_block.contains("requires: [generate-mcp-server]"),
            "build-mcp-binary must require generate-mcp-server:\n{step_block}"
        );
    }

    #[test]
    fn patch_build_mcp_adds_publish_workflow_step() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts_mcp());
        let after_wf = output
            .split("  orb-release:")
            .nth(1)
            .expect("no orb-release workflow");
        assert!(
            after_wf.contains("gen-orb-mcp/publish:"),
            "missing publish-mcp-server step:\n{after_wf}"
        );
        assert!(
            after_wf.contains("name: publish-mcp-server"),
            "publish step must be named publish-mcp-server:\n{after_wf}"
        );
        let after_publish = after_wf
            .split("name: publish-mcp-server")
            .nth(1)
            .expect("no publish step");
        let step_block = after_publish
            .split("\n      - ")
            .next()
            .unwrap_or(after_publish);
        assert!(
            step_block.contains("requires: [build-mcp-binary]"),
            "publish step must require build-mcp-binary:\n{step_block}"
        );
        assert!(
            step_block.contains("context: [pcu-app]"),
            "publish step must use mcp_context:\n{step_block}"
        );
        assert!(
            step_block.contains("asset_name: mytool-mcp-linux-x86_64"),
            "publish step must set asset_name:\n{step_block}"
        );
    }

    #[test]
    fn patch_build_mcp_adds_save_workflow_step() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts_mcp());
        let after_wf = output
            .split("  orb-release:")
            .nth(1)
            .expect("no orb-release workflow");
        assert!(
            after_wf.contains("gen-orb-mcp/save:"),
            "missing save-mcp-server step:\n{after_wf}"
        );
        assert!(
            after_wf.contains("name: save-mcp-server"),
            "save step must be named save-mcp-server:\n{after_wf}"
        );
        let after_save = after_wf
            .split("name: save-mcp-server")
            .nth(1)
            .expect("no save step");
        let step_block = after_save.split("\n      - ").next().unwrap_or(after_save);
        assert!(
            step_block.contains("requires: [generate-mcp-server]"),
            "save step must require generate-mcp-server:\n{step_block}"
        );
        assert!(
            step_block.contains("context: [pcu-app]"),
            "save step must use mcp_context:\n{step_block}"
        );
    }

    #[test]
    fn patch_build_mcp_steps_have_tag_filters() {
        let (output, _) = patch_build(BUILD_FIXTURE, &make_opts_mcp());
        let after_wf = output
            .split("  orb-release:")
            .nth(1)
            .expect("no orb-release workflow");
        for name in &[
            "generate-mcp-server",
            "build-mcp-binary",
            "publish-mcp-server",
            "save-mcp-server",
        ] {
            let after_step = after_wf
                .split(&format!("name: {name}"))
                .nth(1)
                .or_else(|| after_wf.split(&format!("- {name}:")).nth(1))
                .unwrap_or_else(|| panic!("no {name} step in orb-release workflow"));
            let step = after_step.split("\n      - ").next().unwrap_or(after_step);
            assert!(
                step.contains("tags:"),
                "step {name} must have tags: filter:\n{step}"
            );
            assert!(
                step.contains("/^mytool-v.*/"),
                "step {name} must have tag pattern:\n{step}"
            );
        }
    }
}
