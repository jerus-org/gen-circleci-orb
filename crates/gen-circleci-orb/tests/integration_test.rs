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
