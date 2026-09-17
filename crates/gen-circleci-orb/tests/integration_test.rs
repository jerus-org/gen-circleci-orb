use std::process::Command;
use tempfile::TempDir;

/// Capability-goal test: generate an orb for the real gen-orb-mcp binary
/// and verify the output structure passes gen-orb-mcp validate.
#[test]
#[cfg_attr(not(feature = "integration"), ignore)]
fn generate_kdeets_orb() {
    let out = TempDir::new().unwrap();
    let binary = env!("CARGO_BIN_EXE_gen-circleci-orb");

    let status = Command::new(binary)
        .args([
            "generate",
            "--binary",
            "kdeets",
            "--orb-namespace",
            "jerus-org",
            "--output",
            out.path().to_str().unwrap(),
            // No signing material in the test environment; auto-record is
            // config-driven and off here, but pass --no-record to be explicit.
            "--no-record",
        ])
        .status()
        .expect("gen-circleci-orb binary not found");

    assert!(status.success(), "generate command failed: {status}");

    // Files are written to <output>/orb/ (the default --orb-dir)
    let orb_root = out.path().join("orb");
    let src = orb_root.join("src");
    assert!(src.join("@orb.yml").exists(), "missing @orb.yml");
    assert!(
        src.join("executors/default.yml").exists(),
        "missing executors/default.yml"
    );
    assert!(orb_root.join("Dockerfile").exists(), "missing Dockerfile");

    for name in &["crate", "rust", "setup"] {
        assert!(
            src.join(format!("commands/{name}.yml")).exists(),
            "missing commands/{name}.yml"
        );
        assert!(
            src.join(format!("jobs/{name}.yml")).exists(),
            "missing jobs/{name}.yml"
        );
        assert!(
            src.join(format!("scripts/{name}.sh")).exists(),
            "missing scripts/{name}.sh"
        );
    }

    // RC003: examples directory with at least one file
    assert!(
        src.join("examples/example.yml").exists(),
        "missing examples/example.yml"
    );

    // Verify @orb.yml has no commands/jobs/executors keys
    let orb_yml = std::fs::read_to_string(src.join("@orb.yml")).unwrap();
    assert!(
        !orb_yml.contains("commands:"),
        "@orb.yml must not list commands"
    );
    assert!(!orb_yml.contains("jobs:"), "@orb.yml must not list jobs");
    assert!(
        !orb_yml.contains("executors:"),
        "@orb.yml must not list executors"
    );
    assert!(
        orb_yml.contains("version: 2.1"),
        "@orb.yml must have float version"
    );

    // Verify command file uses script include (RC009) and script has binary name
    let crate_cmd = std::fs::read_to_string(src.join("commands/crate.yml")).unwrap();
    assert!(
        crate_cmd.contains("<<include(scripts/crate.sh)>>"),
        "command YAML must use script include:\n{crate_cmd}"
    );
    let crate_script = std::fs::read_to_string(src.join("scripts/crate.sh")).unwrap();
    assert!(
        crate_script.contains("kdeets crate"),
        "script must include binary name:\n{crate_script}"
    );
}

/// Smoke test: re-running generate on identical output changes nothing.
#[test]
#[cfg_attr(not(feature = "integration"), ignore)]
fn generate_is_idempotent() {
    let out = TempDir::new().unwrap();
    let binary = env!("CARGO_BIN_EXE_gen-circleci-orb");
    let args = [
        "generate",
        "--binary",
        "gen-changelog",
        "--orb-namespace",
        "jerus-org",
        "--output",
        out.path().to_str().unwrap(),
        "--no-record",
    ];

    let first = Command::new(binary).args(args).output().unwrap();
    assert!(first.status.success());

    let second = Command::new(binary).args(args).output().unwrap();
    assert!(second.status.success());

    let second_stdout = String::from_utf8_lossy(&second.stdout);
    assert!(
        second_stdout.contains("0 created") || second_stdout.contains("0 updated"),
        "second run should produce no changes:\n{second_stdout}"
    );
}

/// gen-circleci-orb#358 redesign, prerequisite bug: `parse_subcommand`'s own
/// recursion (`help_parser::clap`) re-derives `run_help(binary, &[name,
/// child_name])` at each level -- only the immediate parent, never the full
/// accumulated ancestor chain. For a genuinely 3-level-deep CLI (`a b c`),
/// parsing leaf `c` actually runs `fixture-cli-nested b c --help` (missing
/// `a`), which clap rejects as "unrecognized subcommand 'b'" -- and that
/// error text gets silently absorbed as `c`'s description, with an EMPTY
/// parameter list, no hard failure. This proves the real --help-parsing
/// pipeline correctly captures a depth-3 leaf's real parameters.
#[test]
#[cfg(feature = "test-fixtures")]
fn parse_binary_handles_three_levels_of_subcommand_nesting() {
    let binary = env!("CARGO_BIN_EXE_fixture-cli-nested");
    let cli = gen_circleci_orb::help_parser::parse_binary(
        binary,
        &gen_circleci_orb::help_parser::ParseOptions::default(),
    )
    .expect("parse_binary must succeed against a real, well-formed nested CLI");

    let a = cli
        .subcommands
        .iter()
        .find(|s| s.name == "a")
        .expect("top-level 'a' must be discovered");
    let b = a
        .subcommands
        .iter()
        .find(|s| s.name == "b")
        .expect("nested 'a b' must be discovered");
    let c = b
        .subcommands
        .iter()
        .find(|s| s.name == "c")
        .expect("nested 'a b c' must be discovered");

    assert!(
        c.is_leaf,
        "'a b c' has no children of its own, must be a leaf"
    );
    assert!(
        !c.description.to_lowercase().contains("unrecognized"),
        "a clap parse error must never leak into the leaf's description: {:?}",
        c.description
    );
    assert!(
        c.parameters.iter().any(|p| p.long_name == "value"),
        "'a b c --value' must be captured as a real parameter, not lost \
         to a mis-parsed --help call: {:?}",
        c.parameters
    );
}

/// gen-circleci-orb#358 redesign (superseding the reject-based first pass,
/// per PR #416 review: rejecting pushed the generator's own bare-name-
/// addressing limitation onto the CLI author instead of fixing it).
/// fixture-cli-collision has a genuine top-level `release` and a nested `ci
/// release` sharing a bare name; this runs `generate` against it as a real
/// subprocess and confirms it now SUCCEEDS, with the root occurrence kept
/// under its bare name and the nested occurrence qualified by its full path
/// — never a rejection, and no user-visible workaround required.
#[test]
#[cfg(feature = "test-fixtures")]
fn generate_qualifies_a_real_ambiguous_subcommand_name() {
    let out = TempDir::new().unwrap();
    let binary = env!("CARGO_BIN_EXE_gen-circleci-orb");

    // "fixture-cli-collision" is invoked by gen-circleci-orb as a bare-name
    // subprocess (a real PATH lookup), so its directory must be on the
    // spawned process's PATH — same requirement/reasoning as cli_tests.rs's
    // fixture-cli PATH injection.
    let fixture_bin = std::path::Path::new(env!("CARGO_BIN_EXE_fixture-cli-collision"));
    let fixture_dir = fixture_bin
        .parent()
        .expect("fixture-cli-collision binary path has a parent directory");
    let existing_path = std::env::var_os("PATH").unwrap_or_default();
    let mut paths: Vec<_> = std::env::split_paths(&existing_path).collect();
    paths.insert(0, fixture_dir.to_path_buf());
    let new_path = std::env::join_paths(paths).expect("PATH entries are valid");

    let output = Command::new(binary)
        .args([
            "generate",
            "--binary",
            "fixture-cli-collision",
            "--orb-namespace",
            "jerus-org",
            "--output",
            out.path().to_str().unwrap(),
            "--no-record",
        ])
        .env("PATH", new_path)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "generate must qualify a colliding name and succeed, not reject:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let src = out.path().join("orb").join("src");
    assert!(
        src.join("commands/release.yml").exists(),
        "the ROOT-level 'release' must keep its bare name (its own path IS \
         its bare name, nothing to qualify)"
    );
    assert!(
        src.join("jobs/release.yml").exists(),
        "the root job must also keep its bare name"
    );
    assert!(
        src.join("commands/ci_release.yml").exists(),
        "the NESTED 'ci release' must be qualified by its full path, not \
         collide with the root one"
    );
    assert!(
        src.join("jobs/ci_release.yml").exists(),
        "the nested job must also be qualified"
    );

    // Both must have genuinely distinct, correct content -- not one
    // clobbering the other.
    let root_script = std::fs::read_to_string(src.join("scripts/release.sh")).unwrap();
    assert!(
        root_script.starts_with("set -- fixture-cli-collision release\n"),
        "root script must invoke the top-level path:\n{root_script}"
    );
    let nested_script = std::fs::read_to_string(src.join("scripts/ci_release.sh")).unwrap();
    assert!(
        nested_script.starts_with("set -- fixture-cli-collision ci release\n"),
        "nested script must invoke the full nested path:\n{nested_script}"
    );
}

/// gen-circleci-orb#412: fixture-cli-param-collision's `generate` subcommand
/// has a restricted `--name` (auto-renames to `generate_name`) and an
/// unrelated, genuinely-named `--generate-name` flag whose own normalized
/// name already equals that rename target — two individually valid clap
/// flags that collide only because of gen-circleci-orb's own rename. With no
/// config override, `generate` must fail loudly instead of silently letting
/// one clobber the other.
#[test]
#[cfg(feature = "test-fixtures")]
fn generate_rejects_a_real_param_key_collision() {
    let out = TempDir::new().unwrap();
    let binary = env!("CARGO_BIN_EXE_gen-circleci-orb");
    let new_path = fixture_param_collision_path_env();

    let output = Command::new(binary)
        .args([
            "generate",
            "--binary",
            "fixture-cli-param-collision",
            "--orb-namespace",
            "jerus-org",
            "--output",
            out.path().to_str().unwrap(),
            "--no-record",
        ])
        .env("PATH", new_path)
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "generate must reject an unresolved param-key collision, not silently \
         clobber one param with the other"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("generate_name"), "got: {stderr}");
    assert!(stderr.contains("orb_name"), "got: {stderr}");
}

/// Same collision as above, resolved via an `orb_name` override on the
/// RESTRICTED `name` param (the reviewer-preferred pattern for #412: the
/// rename that CREATES the collision is `--name`'s own, so override ITS key
/// rather than the unrelated `--generate-name` flag's — one change instead
/// of two, and `--generate-name` keeps its own natural derived key).
#[test]
#[cfg(feature = "test-fixtures")]
fn generate_resolves_a_real_param_key_collision_via_orb_name_override() {
    let out = TempDir::new().unwrap();
    let binary = env!("CARGO_BIN_EXE_gen-circleci-orb");
    let new_path = fixture_param_collision_path_env();

    std::fs::write(
        out.path().join("gen-circleci-orb.toml"),
        r#"
[subcommand.generate.param.name]
orb_name = "output_name"
"#,
    )
    .unwrap();

    let output = Command::new(binary)
        .args([
            "generate",
            "--binary",
            "fixture-cli-param-collision",
            "--orb-namespace",
            "jerus-org",
            "--output",
            out.path().to_str().unwrap(),
            "--no-record",
        ])
        .env("PATH", new_path)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "generate must succeed once the collision is resolved via an \
         orb_name override:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let command =
        std::fs::read_to_string(out.path().join("orb/src/commands/generate.yml")).unwrap();
    let job = std::fs::read_to_string(out.path().join("orb/src/jobs/generate.yml")).unwrap();
    for rendered in [&command, &job] {
        assert!(
            rendered.contains("output_name:"),
            "the restricted 'name' param must render under its 'orb_name' \
             override 'output_name':\n{rendered}"
        );
        assert!(
            rendered.contains("generate_name:"),
            "the genuinely-named 'generate_name' flag must keep its own \
             natural derived key, untouched by 'name''s override:\n{rendered}"
        );
    }
}

/// Prepends fixture-cli-param-collision's directory to `PATH` — the same
/// PATH-injection gen-circleci-orb needs to invoke a fixture as a bare-name
/// subprocess (see `generate_qualifies_a_real_ambiguous_subcommand_name`'s
/// identical pattern for fixture-cli-collision above).
#[cfg(feature = "test-fixtures")]
fn fixture_param_collision_path_env() -> std::ffi::OsString {
    let fixture_bin = std::path::Path::new(env!("CARGO_BIN_EXE_fixture-cli-param-collision"));
    let fixture_dir = fixture_bin
        .parent()
        .expect("fixture-cli-param-collision binary path has a parent directory");
    let existing_path = std::env::var_os("PATH").unwrap_or_default();
    let mut paths: Vec<_> = std::env::split_paths(&existing_path).collect();
    paths.insert(0, fixture_dir.to_path_buf());
    std::env::join_paths(paths).expect("PATH entries are valid")
}

/// docs/configuration-guide.md's "Worked example" embeds the ACTUAL output of
/// running `generate` against `fixture-cli-param-collision` with the
/// documented `orb_name` override, not hand-copied YAML that can silently
/// drift from what the generator really produces. This regenerates that
/// exact scenario and asserts the doc's embedded blocks (delimited by
/// `<!-- worked-example:* -->` / `<!-- /worked-example:* -->` markers) are
/// byte-identical to the real output.
#[test]
#[cfg(feature = "test-fixtures")]
fn configuration_guide_worked_example_matches_real_generated_output() {
    let out = TempDir::new().unwrap();
    let binary = env!("CARGO_BIN_EXE_gen-circleci-orb");
    let new_path = fixture_param_collision_path_env();

    std::fs::write(
        out.path().join("gen-circleci-orb.toml"),
        r#"
[subcommand.generate.param.name]
orb_name = "output_name"
"#,
    )
    .unwrap();

    let output = Command::new(binary)
        .args([
            "generate",
            "--binary",
            "fixture-cli-param-collision",
            "--orb-namespace",
            "jerus-org",
            "--output",
            out.path().to_str().unwrap(),
            "--no-record",
        ])
        .env("PATH", new_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "generate failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let command =
        std::fs::read_to_string(out.path().join("orb/src/commands/generate.yml")).unwrap();
    let job = std::fs::read_to_string(out.path().join("orb/src/jobs/generate.yml")).unwrap();
    let job_invoke: String = job
        .lines()
        .skip_while(|l| !l.starts_with("- generate:"))
        .collect::<Vec<_>>()
        .join("\n");

    let docs = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/configuration-guide.md"
    ))
    .unwrap();

    assert_eq!(
        extract_marked_yaml_block(&docs, "worked-example:command").trim(),
        command.trim(),
        "docs/configuration-guide.md's embedded command YAML has drifted \
         from what generate() actually produces — regenerate the fixture's \
         output and paste it back into the docs"
    );
    assert_eq!(
        extract_marked_yaml_block(&docs, "worked-example:job-invoke").trim(),
        job_invoke.trim(),
        "docs/configuration-guide.md's embedded job invoke-step YAML has \
         drifted from what generate() actually produces — regenerate the \
         fixture's output and paste it back into the docs"
    );
}

/// Extracts and un-fences the YAML content between
/// `<!-- <marker> -->` / `<!-- /<marker> -->` HTML-comment delimiters in a
/// Markdown document.
#[cfg(feature = "test-fixtures")]
fn extract_marked_yaml_block(markdown: &str, marker: &str) -> String {
    let start_marker = format!("<!-- {marker} -->");
    let end_marker = format!("<!-- /{marker} -->");
    let start = markdown
        .find(&start_marker)
        .unwrap_or_else(|| panic!("start marker {start_marker:?} not found in docs"))
        + start_marker.len();
    let end = markdown[start..]
        .find(&end_marker)
        .unwrap_or_else(|| panic!("end marker {end_marker:?} not found in docs"))
        + start;
    markdown[start..end]
        .trim()
        .trim_start_matches("```yaml")
        .trim_end_matches("```")
        .trim()
        .to_string()
}
