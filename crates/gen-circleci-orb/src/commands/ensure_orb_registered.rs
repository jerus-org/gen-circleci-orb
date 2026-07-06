use anyhow::{Context, Result};

/// Ensure a CircleCI orb is registered, creating it if it does not exist.
///
/// Authenticates the CircleCI CLI via the `CIRCLE_TOKEN` environment variable
/// (exported as `CIRCLECI_CLI_TOKEN`) rather than calling `circleci setup`,
/// which was removed in newer CLI releases.
#[derive(Debug, clap::Args)]
pub struct EnsureOrbRegistered {
    /// The orb name to check/register (e.g. my-org/my-orb).
    #[arg(long)]
    pub orb_name: String,

    /// Register the orb as private when creating it.
    ///
    /// Must be set correctly on first creation — orb visibility cannot be
    /// changed after the orb is created.
    #[arg(long)]
    pub private: bool,
}

/// Abstraction over circleci CLI invocation for testability.
pub(crate) trait CliRunner {
    fn run(&self, args: &[&str], token: &str) -> Result<(i32, String, String)>;
}

pub(crate) struct ProcessRunner;

impl CliRunner for ProcessRunner {
    fn run(&self, args: &[&str], token: &str) -> Result<(i32, String, String)> {
        let (program, rest) = args.split_first().context("empty args")?;
        let output = std::process::Command::new(program)
            .args(rest)
            .env("CIRCLECI_CLI_TOKEN", token)
            .output()
            .with_context(|| format!("failed to run {program}"))?;
        let exit = output.status.code().unwrap_or(1);
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        Ok((exit, stdout, stderr))
    }
}

impl EnsureOrbRegistered {
    pub fn run(&self) -> Result<()> {
        let token =
            std::env::var("CIRCLE_TOKEN").context("CIRCLE_TOKEN environment variable not set")?;
        self.run_with_runner(&ProcessRunner, &token)
    }

    pub(crate) fn run_with_runner<R: CliRunner>(&self, runner: &R, token: &str) -> Result<()> {
        // `circleci orb info` exits 0 only when the orb exists. A missing orb exits
        // non-zero (255, "no Orb '…' was found"). 255 must NOT be treated as
        // "registered": that inverts the check and means a missing orb is never
        // created — defeating the whole purpose of this command.
        let (info_exit, _, _) = runner.run(&["circleci", "orb", "info", &self.orb_name], token)?;

        if info_exit == 0 {
            println!("Orb is registered.");
            return Ok(());
        }

        let mut args = vec!["circleci", "orb", "create", &self.orb_name, "--no-prompt"];
        if self.private {
            args.push("--private");
        }
        let (create_exit, create_out, create_err) = runner.run(&args, token)?;
        let combined = format!("{create_out}{create_err}");

        // Success on a clean create (exit 0) or when the orb already exists
        // (idempotent / race between info and create). Any other non-zero — auth,
        // missing namespace, network — must surface; 255 is the CLI's generic error
        // code, so it is NOT silently accepted.
        if create_exit != 0 && !combined.contains("already exists") {
            anyhow::bail!("circleci orb create failed (exit {create_exit}): {combined}");
        }

        println!("Orb is registered.");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    struct FakeRunner {
        responses: RefCell<VecDeque<(i32, String, String)>>,
        calls: RefCell<Vec<Vec<String>>>,
    }

    impl FakeRunner {
        fn new(responses: Vec<(i32, &str, &str)>) -> Self {
            FakeRunner {
                responses: RefCell::new(
                    responses
                        .into_iter()
                        .map(|(e, o, er)| (e, o.to_string(), er.to_string()))
                        .collect(),
                ),
                calls: RefCell::new(Vec::new()),
            }
        }
        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.borrow().clone()
        }
    }

    impl CliRunner for FakeRunner {
        fn run(&self, args: &[&str], _token: &str) -> Result<(i32, String, String)> {
            self.calls
                .borrow_mut()
                .push(args.iter().map(|s| s.to_string()).collect());
            self.responses
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("FakeRunner: no more responses"))
        }
    }

    fn cmd(orb_name: &str) -> EnsureOrbRegistered {
        EnsureOrbRegistered {
            orb_name: orb_name.to_string(),
            private: false,
        }
    }

    #[test]
    fn orb_exists_exit_0_returns_ok() {
        let runner = FakeRunner::new(vec![(0, "", "")]);
        assert!(cmd("my-org/my-orb").run_with_runner(&runner, "tok").is_ok());
    }

    #[test]
    fn info_255_missing_orb_triggers_create() {
        // `circleci orb info` exits 255 ("no Orb ... was found") when the orb does
        // NOT exist. That must trigger creation — it must NOT be read as
        // "already registered" (the old inverted behaviour that meant a missing
        // orb was never created).
        let runner = FakeRunner::new(vec![
            (255, "", "no Orb 'my-org/my-orb@volatile' was found"),
            (0, "", ""),
        ]);
        cmd("my-org/my-orb")
            .run_with_runner(&runner, "tok")
            .unwrap();
        let calls = runner.calls();
        assert_eq!(calls.len(), 2, "info (255) must be followed by create");
        assert!(calls[1].contains(&"create".to_string()));
    }

    #[test]
    fn orb_exists_only_calls_info_not_create() {
        let runner = FakeRunner::new(vec![(0, "", "")]);
        cmd("my-org/my-orb")
            .run_with_runner(&runner, "tok")
            .unwrap();
        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].contains(&"info".to_string()));
        assert!(!calls[0].iter().any(|a| a == "create"));
    }

    #[test]
    fn orb_not_found_calls_create() {
        let runner = FakeRunner::new(vec![(1, "", ""), (0, "", "")]);
        cmd("my-org/my-orb")
            .run_with_runner(&runner, "tok")
            .unwrap();
        assert_eq!(runner.calls().len(), 2);
        assert!(runner.calls()[1].contains(&"create".to_string()));
    }

    #[test]
    fn info_and_create_both_receive_orb_name() {
        let runner = FakeRunner::new(vec![(1, "", ""), (0, "", "")]);
        cmd("my-org/my-orb")
            .run_with_runner(&runner, "tok")
            .unwrap();
        let calls = runner.calls();
        assert!(
            calls[0].contains(&"my-org/my-orb".to_string()),
            "info must include orb name"
        );
        assert!(
            calls[1].contains(&"my-org/my-orb".to_string()),
            "create must include orb name"
        );
    }

    #[test]
    fn create_includes_no_prompt_flag() {
        let runner = FakeRunner::new(vec![(1, "", ""), (0, "", "")]);
        cmd("my-org/my-orb")
            .run_with_runner(&runner, "tok")
            .unwrap();
        assert!(runner.calls()[1].contains(&"--no-prompt".to_string()));
    }

    #[test]
    fn private_flag_adds_private_to_create() {
        let runner = FakeRunner::new(vec![(1, "", ""), (0, "", "")]);
        EnsureOrbRegistered {
            orb_name: "my-org/my-orb".to_string(),
            private: true,
        }
        .run_with_runner(&runner, "tok")
        .unwrap();
        assert!(runner.calls()[1].contains(&"--private".to_string()));
    }

    #[test]
    fn public_orb_create_omits_private_flag() {
        let runner = FakeRunner::new(vec![(1, "", ""), (0, "", "")]);
        cmd("my-org/my-orb")
            .run_with_runner(&runner, "tok")
            .unwrap();
        assert!(!runner.calls()[1].contains(&"--private".to_string()));
    }

    #[test]
    fn create_exit_255_without_already_exists_returns_error() {
        // 255 is the CLI's generic error code (auth, missing namespace, network).
        // Without an "already exists" marker it must surface, not be masked as success.
        let runner = FakeRunner::new(vec![
            (255, "", "not found"),
            (255, "", "Error: permission denied"),
        ]);
        assert!(cmd("my-org/my-orb")
            .run_with_runner(&runner, "tok")
            .is_err());
    }

    #[test]
    fn create_already_exists_in_output_treated_as_success() {
        // Idempotent / race: the orb already exists. Accept regardless of exit code.
        let runner = FakeRunner::new(vec![(255, "", "not found"), (1, "orb already exists", "")]);
        assert!(cmd("my-org/my-orb").run_with_runner(&runner, "tok").is_ok());
    }

    #[test]
    fn create_exit_0_is_success() {
        let runner = FakeRunner::new(vec![(255, "", "not found"), (0, "", "")]);
        assert!(cmd("my-org/my-orb").run_with_runner(&runner, "tok").is_ok());
    }

    #[test]
    fn create_other_failure_returns_error() {
        let runner = FakeRunner::new(vec![(1, "", ""), (1, "some other error", "")]);
        assert!(cmd("my-org/my-orb")
            .run_with_runner(&runner, "tok")
            .is_err());
    }
}
