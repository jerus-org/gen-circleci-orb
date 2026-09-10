#[test]
fn cli_tests() {
    let cases = trycmd::TestCases::new();
    cases.case("tests/cmd/*.trycmd").register_bin(
        "gen-circleci-orb",
        std::path::Path::new(env!("CARGO_BIN_EXE_gen-circleci-orb")),
    );

    // fixture-cli is a test-only binary (required-features = ["test-fixtures"]) —
    // env!("CARGO_BIN_EXE_fixture-cli") only resolves when that feature built it,
    // so this whole case (and the bin registration it depends on) is compiled out
    // otherwise rather than breaking a plain `cargo test`.
    #[cfg(feature = "test-fixtures")]
    {
        // `generate` invokes `fixture-cli --help` as a bare-name subprocess (a real
        // PATH lookup, not something trycmd's register_bin propagates into a
        // grandchild process) — so fixture-cli's own directory has to be on the
        // spawned gen-circleci-orb process's PATH for that nested exec to resolve.
        // `TestCases::env` sets this per-command (via Command::envs, inherited by
        // default) rather than mutating the whole test binary's process
        // environment, so no unsafe std::env::set_var is needed.
        let fixture_bin = std::path::Path::new(env!("CARGO_BIN_EXE_fixture-cli"));
        let fixture_dir = fixture_bin
            .parent()
            .expect("fixture-cli binary path has a parent directory");
        let existing_path = std::env::var_os("PATH").unwrap_or_default();
        let mut paths: Vec<_> = std::env::split_paths(&existing_path).collect();
        paths.insert(0, fixture_dir.to_path_buf());
        let new_path = std::env::join_paths(paths).expect("PATH entries are valid");
        cases.env("PATH", new_path.to_string_lossy().into_owned());

        cases.case("tests/cmd/generate_fixture.toml").register_bin(
            "fixture-cli",
            std::path::Path::new(env!("CARGO_BIN_EXE_fixture-cli")),
        );
    }
}
