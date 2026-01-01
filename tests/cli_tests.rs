/// Tests for the command line interface (e.g. `leguinvim --no-fork foo.txt`)

#[test]
fn cli_tests() {
    trycmd::TestCases::new()
        .env("LEGUINVIM_CLI_TEST_MODE", "1")
        .case("tests/cmd/*.trycmd");
    #[cfg(unix)]
    trycmd::TestCases::new()
        .env("LEGUINVIM_CLI_TEST_MODE", "1")
        .case("tests/cmd_unix/*.trycmd");
}
