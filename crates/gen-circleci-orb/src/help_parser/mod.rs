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
