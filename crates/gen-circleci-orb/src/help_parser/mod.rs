pub mod clap;
pub mod types;

pub use types::{CliDefinition, ParamKind, ParamType, Parameter, SubCommand};

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::process::Command;

/// Caller-supplied parser settings, sourced from `gen-circleci-orb.toml`.
#[derive(Debug, Default, Clone)]
pub struct ParseOptions {
    /// Downgrade the coverage guard from a hard failure to a warning.
    ///
    /// The guard fails generation when a declaration in `--help` produced no
    /// orb parameter, because the alternative — the historical behaviour — is a
    /// job that silently cannot supply an input the CLI requires (#240/#241/#242).
    /// Set `[orb] allow_unparsed_help = true` to ship anyway while the parser is
    /// taught the shape.
    pub allow_unparsed_help: bool,
    /// Names for short-only options, keyed by subcommand then by short flag,
    /// from `[subcommand.<name>.short_param]`.
    ///
    /// A `-f` with no long form has nothing to name its orb parameter after.
    /// The parser derives one from the description where it can; this is how a
    /// consumer overrides that, or supplies a name where none can be derived.
    pub short_param_names: HashMap<String, HashMap<char, String>>,
}

/// `major.minor` of `<binary> --version`, for pinning the orb in generated
/// examples; `None` when the binary prints no recognisable version.
pub fn binary_orb_version_pin(binary: &str) -> Option<String> {
    let output = Command::new(binary).arg("--version").output().ok()?;
    orb_version_pin(&String::from_utf8_lossy(&output.stdout))
}

/// `major.minor` from a `--version` line such as clap's `mytool 1.2.3`.
/// The version is the last token that starts with `<digits>.<digits>`; any
/// pre-release or build suffix is dropped.
pub(crate) fn orb_version_pin(version_output: &str) -> Option<String> {
    version_output.split_whitespace().rev().find_map(|token| {
        let mut parts = token.split('.');
        let major = parts.next()?;
        let minor = parts.next()?;
        let minor: String = minor.chars().take_while(char::is_ascii_digit).collect();
        (!major.is_empty() && major.chars().all(|c| c.is_ascii_digit()) && !minor.is_empty())
            .then(|| format!("{major}.{minor}"))
    })
}

/// Execute `<binary> --help` (and recursively `<binary> <sub> --help`) to
/// build a `CliDefinition` from the program's help text.
pub fn parse_binary(binary: &str, opts: &ParseOptions) -> Result<CliDefinition> {
    let top_help = run_help(binary, &[])?;
    clap::parse_top_level(binary, &top_help, opts)
}

pub(crate) fn run_help(binary: &str, subcommand: &[&str]) -> Result<String> {
    let mut args: Vec<&str> = subcommand.to_vec();
    args.push("--help");
    let output = Command::new(binary)
        .args(&args)
        .output()
        .with_context(|| format!("failed to run `{binary} {args:?}`"))?;
    // clap writes --help to stdout; tolerate non-zero exit
    let text = if output.stdout.is_empty() {
        String::from_utf8_lossy(&output.stderr).into_owned()
    } else {
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    Ok(text)
}

#[cfg(test)]
mod orb_version_pin_tests {
    use super::orb_version_pin;

    #[test]
    fn takes_major_minor_from_a_clap_version_line() {
        assert_eq!(orb_version_pin("jci-coverage 0.0.4\n"), Some("0.0".into()));
        assert_eq!(orb_version_pin("mytool 1.12.3"), Some("1.12".into()));
    }

    #[test]
    fn ignores_pre_release_and_build_metadata() {
        assert_eq!(
            orb_version_pin("mytool 2.3.0-beta.1+abc"),
            Some("2.3".into())
        );
    }

    #[test]
    fn none_when_no_version_is_present() {
        assert_eq!(orb_version_pin(""), None);
        assert_eq!(
            orb_version_pin("error: unexpected argument '--version'"),
            None
        );
        assert_eq!(orb_version_pin("mytool v"), None);
    }
}
