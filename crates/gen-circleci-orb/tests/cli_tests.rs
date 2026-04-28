#[test]
fn cli_tests() {
    trycmd::TestCases::new()
        .case("tests/cmd/*.trycmd")
        .register_bin(
            "gen-circleci-orb",
            std::path::Path::new(env!("CARGO_BIN_EXE_gen-circleci-orb")),
        );
}
