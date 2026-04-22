pub mod clap;
pub mod types;

pub use types::{CliDefinition, ParamType, Parameter, SubCommand};

use anyhow::{Context, Result};
use std::process::Command;

/// Execute `<binary> --help` (and recursively `<binary> <sub> --help`) to
/// build a `CliDefinition` from the program's help text.
pub fn parse_binary(binary: &str) -> Result<CliDefinition> {
    let top_help = run_help(binary, &[])?;
    clap::parse_top_level(binary, &top_help)
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
