use std::process::Command;
use tempfile::TempDir;

/// Capability-goal test: generate an orb for the real gen-orb-mcp binary
/// and verify the output structure passes gen-orb-mcp validate.
#[test]
#[cfg_attr(not(feature = "integration"), ignore)]
fn generate_gen_orb_mcp_orb() {
    let out = TempDir::new().unwrap();
    let binary = env!("CARGO_BIN_EXE_gen-circleci-orb");

    let status = Command::new(binary)
        .args([
            "generate",
            "--binary",
            "gen-orb-mcp",
            "--namespace",
            "jerus-org",
            "--output",
            out.path().to_str().unwrap(),
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

    for name in &["generate", "validate", "diff", "migrate", "prime"] {
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

    // Verify gen-orb-mcp validate passes
    let validate = Command::new("gen-orb-mcp")
        .args([
            "validate",
            "--orb-path",
            src.join("@orb.yml").to_str().unwrap(),
        ])
        .output()
        .expect("gen-orb-mcp not found");

    assert!(
        validate.status.success(),
        "gen-orb-mcp validate failed:\n{}",
        String::from_utf8_lossy(&validate.stderr)
    );

    // Verify command file uses script include (RC009) and script has binary name
    let generate_cmd = std::fs::read_to_string(src.join("commands/generate.yml")).unwrap();
    assert!(
        generate_cmd.contains("<<include(scripts/generate.sh)>>"),
        "command YAML must use script include:\n{generate_cmd}"
    );
    let generate_script = std::fs::read_to_string(src.join("scripts/generate.sh")).unwrap();
    assert!(
        generate_script.contains("gen-orb-mcp generate"),
        "script must include binary name:\n{generate_script}"
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
        "gen-orb-mcp",
        "--namespace",
        "jerus-org",
        "--output",
        out.path().to_str().unwrap(),
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
