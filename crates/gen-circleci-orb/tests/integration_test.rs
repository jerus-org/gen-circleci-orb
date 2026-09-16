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
