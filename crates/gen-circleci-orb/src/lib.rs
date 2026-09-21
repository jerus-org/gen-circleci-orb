//! # gen-circleci-orb
//!
//! Generate a CircleCI orb from a CLI program definition.

use anyhow::Result;
use clap::Parser;

pub mod ci_patcher;
pub mod commands;
mod fs_atomic;
pub mod help_parser;
pub mod orb_config;
pub mod orb_generator;
pub mod orb_wiring;
pub mod output_writer;

/// Command-line interface for gen-circleci-orb.
#[derive(Debug, Parser)]
#[command(
    author,
    version,
    about,
    long_about = "Generate a CircleCI orb from a CLI program's --help output, and manage the \
        generated orb and the CI wiring that publishes it. `init` scaffolds a new consumer \
        (gen-circleci-orb.toml + CI wiring); `generate` (re)builds the orb source from the \
        target binary's --help; `update` re-syncs an existing consumer's CI wiring to the \
        current generator flow without touching gen-circleci-orb.toml; `config` edits the saved \
        config directly; `ensure-orb-registered` is a small CI-internal helper used by the \
        generated orb-release workflow.",
    // Fixed rather than autodetected: help is frequently captured non-interactively
    // (trycmd's subprocess snapshots, CI logs, a pipe) where terminal-size detection
    // finds nothing and clap falls back to an unhelpfully wide, unwrapped render —
    // pinning this keeps `-h`/`--help` wrapped consistently everywhere (#410 review).
    term_width = 75
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

/// Available subcommands.
#[derive(Debug, clap::Subcommand)]
pub enum Commands {
    /// Manage the gen-circleci-orb.toml configuration file.
    ///
    /// Read, or make a small targeted edit to, the saved config without hand-editing TOML:
    /// suppress/unsuppress a subcommand's generated job, append a composed job group, or set a
    /// parameter default override. Each edit is a single command, safe to script.
    Config(commands::config::Config),
    /// Ensure a CircleCI orb is registered, creating it if it does not exist.
    ///
    /// A small CI-internal helper (not something a consumer normally runs by hand): the
    /// generated orb-release workflow calls this before `orb-tools/publish` so the very first
    /// publish of a brand-new orb namespace/name doesn't fail on a missing orb. Idempotent —
    /// a no-op once the orb already exists.
    EnsureOrbRegistered(commands::ensure_orb_registered::EnsureOrbRegistered),
    /// Generate orb source files from a CLI binary's --help output.
    ///
    /// Introspects the target binary (named by --binary or `[orb] binary`) via its own --help
    /// text, and (re)writes the full orb source tree (commands, jobs, scripts, executor,
    /// Dockerfile) under --output/--orb-dir. Safe to re-run: identical output is left untouched.
    /// Use --check in CI to gate on a drifted or hand-edited orb; --dry-run to preview with no
    /// writes.
    Generate(Box<commands::generate::Generate>),
    /// Wire orb generation into an existing repo's CI configuration.
    ///
    /// One-time (or re-run-to-update) setup: writes gen-circleci-orb.toml from the flags/prompts
    /// given, then patches .circleci/config.yml to add the generated-orb build/publish/release
    /// jobs. Prompts interactively for anything not supplied on the command line. Re-running
    /// against an existing config updates rather than duplicates the wiring.
    Init(Box<commands::init::Init>),
    /// Re-sync an existing repo's orb-managed CI wiring to the current flow.
    ///
    /// Reads the committed gen-circleci-orb.toml (never overwrites it) and rewrites only the
    /// gen-circleci-orb-managed blocks in .circleci/config.yml, preserving the consumer's own
    /// jobs and customizations. Run with --check in CI to fail when the wiring is out of date
    /// relative to the pinned orb version. It also checks the arguments of every gen-circleci-orb
    /// job invocation in the CI files against the orb's job parameters, removing arguments a job
    /// no longer declares.
    Update(commands::update::Update),
}

impl Cli {
    /// Execute the selected command.
    pub fn run(&self) -> Result<()> {
        match &self.command {
            Commands::Config(cmd) => cmd.run(),
            Commands::EnsureOrbRegistered(cmd) => cmd.run(),
            Commands::Generate(cmd) => cmd.run(),
            Commands::Init(cmd) => cmd.run(),
            Commands::Update(cmd) => cmd.run(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// gen-circleci-orb#410: `--help` and `-h` were identical everywhere —
    /// every subcommand's own doc comment provided only `about`, with
    /// `long_about` left unset (or explicitly `None` at the top level). A
    /// doc comment without a blank-line-separated second paragraph leaves
    /// `long_about` unset, so clap falls back to `about` for `--help` too —
    /// checking rendered `-h`/`--help` byte length is not a reliable proxy
    /// (clap's own argument-table wrapping can make `--help` render longer
    /// even when `about == long_about`), so this checks the parsed
    /// long_about directly instead (mirrors jci-audit's own structural
    /// test for the same pattern).
    #[test]
    fn every_subcommand_has_a_long_about_distinct_from_its_about() {
        let cmd = Cli::command();
        for name in [
            "config",
            "ensure-orb-registered",
            "generate",
            "init",
            "update",
        ] {
            let sub = cmd
                .find_subcommand(name)
                .unwrap_or_else(|| panic!("no '{name}' subcommand"));
            let about = sub.get_about().map(ToString::to_string);
            let long_about = sub.get_long_about().map(ToString::to_string);
            assert!(
                long_about.is_some() && long_about != about,
                "'{name}': long_about must be set and differ from about (about: {about:?})"
            );
        }
    }

    #[test]
    fn top_level_has_a_long_about_distinct_from_its_about() {
        let cmd = Cli::command();
        let about = cmd.get_about().map(ToString::to_string);
        let long_about = cmd.get_long_about().map(ToString::to_string);
        assert!(
            long_about.is_some() && long_about != about,
            "top-level long_about must be set and differ from about (about: {about:?})"
        );
    }

    /// gen-circleci-orb#410 (review): a flag's own `-h` text must be a real
    /// single, short sentence — not the whole multi-sentence doc comment
    /// merged into one paragraph. A doc comment split across consecutive
    /// `///` lines with NO blank line between them is still ONE clap
    /// paragraph: `get_help()` (short) returns the whole thing verbatim
    /// (clap only re-wraps it for display), so checking for an embedded
    /// `\n` catches nothing — the merge is invisible at the string level,
    /// only visible as "too many sentences in one 'line'" once rendered.
    /// Flags a short help containing a `. ` sentence boundary (a second
    /// sentence that should have been split into `--help`-only long help),
    /// excluding the abbreviations actually used in this codebase's doc
    /// comments (`e.g.`, `i.e.`) which are not sentence boundaries.
    #[test]
    fn every_flags_short_help_is_one_sentence() {
        let cmd = Cli::command();
        let mut violations = Vec::new();
        check_command_args(&cmd, "gen-circleci-orb", &mut violations);
        assert!(
            violations.is_empty(),
            "flags whose short help ('-h') looks like more than one sentence \
             merged together (move the rest into --help-only long help via a \
             blank-line-separated second paragraph):\n{}",
            violations.join("\n")
        );
    }

    fn has_a_second_sentence(help: &str) -> bool {
        const ABBREVIATIONS: &[&str] = &["e.g", "i.e", "etc"];
        let mut rest = help;
        while let Some(idx) = rest.find(". ") {
            let before = &rest[..idx];
            if !ABBREVIATIONS.iter().any(|a| before.ends_with(a)) {
                return true;
            }
            rest = &rest[idx + 2..];
        }
        false
    }

    fn check_command_args(cmd: &clap::Command, path: &str, violations: &mut Vec<String>) {
        for arg in cmd.get_arguments() {
            if let Some(help) = arg.get_help() {
                let help = help.to_string();
                if has_a_second_sentence(&help) {
                    violations.push(format!(
                        "{path} --{}: {help:?}",
                        arg.get_long().unwrap_or(arg.get_id().as_str())
                    ));
                }
            }
        }
        for sub in cmd.get_subcommands() {
            check_command_args(sub, &format!("{path} {}", sub.get_name()), violations);
        }
    }

    /// Review on #428: `-h`/`--help` rendered lines well past a standard
    /// terminal's width — e.g. a 146-character single line for
    /// `--circleci-cli-version`. Root cause: this crate's `clap` dependency
    /// only enabled the `derive` feature, never `wrap_help` — without it
    /// clap does not wrap help text AT ALL, regardless of `term_width`
    /// (confirmed by testing `term_width(20)`, which had zero effect until
    /// `wrap_help` was added to `Cargo.toml`). `Cli`'s `term_width = 75`
    /// (`lib.rs`'s `#[command(...)]`) now propagates to every subcommand
    /// and actually wraps, but nothing previously enforced it stays that
    /// way — this renders every subcommand's real `-h` (using the SAME
    /// `Cli::command()` a consumer actually gets, not an artificially wide
    /// override) and checks no line exceeds a small buffer over that
    /// budget (word-wrap can't break a single overlong token mid-word).
    #[test]
    fn short_help_never_wraps_on_an_80_column_terminal() {
        const MAX_WIDTH: usize = 80;
        let mut cmd = Cli::command();
        let mut violations = Vec::new();
        check_command_short_help_width(&mut cmd, "gen-circleci-orb", MAX_WIDTH, &mut violations);
        assert!(
            violations.is_empty(),
            "'-h' output exceeds {MAX_WIDTH} columns (shorten the flag's short \
             help; move detail into --help-only long help):\n{}",
            violations.join("\n")
        );
    }

    fn check_command_short_help_width(
        cmd: &mut clap::Command,
        path: &str,
        max_width: usize,
        violations: &mut Vec<String>,
    ) {
        // render_help() builds (and propagates settings like term_width into)
        // `cmd` IN PLACE — rendering a throwaway `cmd.clone()` instead would
        // leave the propagation on the clone only, so the recursive call
        // below would see each subcommand's term_width still unset.
        let rendered = cmd.render_help().to_string();
        for line in rendered.lines() {
            if line.chars().count() > max_width {
                violations.push(format!(
                    "{path} -h: {} chars: {line:?}",
                    line.chars().count()
                ));
            }
        }
        for sub in cmd.get_subcommands_mut() {
            check_command_short_help_width(
                sub,
                &format!("{path} {}", sub.get_name()),
                max_width,
                violations,
            );
        }
    }
}
