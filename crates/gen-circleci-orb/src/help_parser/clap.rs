use std::collections::HashSet;

use super::types::{CliDefinition, ParamType, Parameter, SubCommand};
use super::{run_help, ParseOptions};
use anyhow::Result;

/// Parse the top-level `--help` output for `binary` and recursively fetch
/// help for each discovered subcommand.
pub fn parse_top_level(
    binary: &str,
    help_text: &str,
    opts: &ParseOptions,
) -> Result<CliDefinition> {
    let description = extract_description(help_text);
    let sub_names = extract_subcommand_names(help_text);

    let mut subcommands = Vec::new();
    for name in sub_names {
        let sub_help = run_help(binary, &[&name])?;
        let sub = parse_subcommand(&name, &sub_help, binary, opts)?;
        subcommands.push(sub);
    }

    Ok(CliDefinition {
        binary_name: normalize_binary_name(binary),
        description,
        subcommands,
    })
}

/// Extract the filename stem from a binary path, returning just the bare name.
///
/// `./target/release/gen-orb-mcp` → `gen-orb-mcp`
/// `/usr/local/bin/gen-orb-mcp`   → `gen-orb-mcp`
/// `gen-orb-mcp`                  → `gen-orb-mcp`
fn normalize_binary_name(binary: &str) -> String {
    std::path::Path::new(binary)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(binary)
        .to_string()
}

fn parse_subcommand(
    name: &str,
    help_text: &str,
    binary: &str,
    opts: &ParseOptions,
) -> Result<SubCommand> {
    let description = extract_description(help_text);
    let child_names = extract_subcommand_names(help_text);
    let is_leaf = child_names.is_empty();

    let mut subcommands = Vec::new();
    for child_name in &child_names {
        let child_help = run_help(binary, &[name, child_name])?;
        let child = parse_subcommand(child_name, &child_help, binary, opts)?;
        subcommands.push(child);
    }

    let parameters = if is_leaf {
        let parsed = parse_parameters_detailed(help_text);
        check_coverage(name, &parsed.unparsed, opts)?;
        parsed.parameters
    } else {
        Vec::new()
    };

    Ok(SubCommand {
        name: name.to_string(),
        description,
        is_leaf,
        parameters,
        subcommands,
    })
}

/// Extract the first non-empty paragraph before any section header as the description.
fn extract_description(text: &str) -> String {
    let mut lines = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        // Stop at section headers (capitalised word followed by colon)
        if is_section_header(trimmed) {
            break;
        }
        // Skip "Usage:" lines
        if trimmed.starts_with("Usage:") {
            break;
        }
        lines.push(trimmed.to_string());
    }
    // Drop leading/trailing blanks and join
    let joined: Vec<&str> = lines
        .iter()
        .map(|s| s.as_str())
        .skip_while(|s| s.is_empty())
        .collect();
    // Trim trailing empty lines
    let end = joined
        .iter()
        .rposition(|s| !s.is_empty())
        .map_or(0, |i| i + 1);
    joined[..end].join(" ")
}

/// Extract subcommand names from the `Commands:` section, skipping `help`.
pub fn extract_subcommand_names(text: &str) -> Vec<String> {
    let mut in_commands = false;
    let mut names = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "Commands:" {
            in_commands = true;
            continue;
        }
        if in_commands {
            if trimmed.is_empty() {
                continue;
            }
            // A new section header ends the commands block
            if is_section_header(trimmed) {
                break;
            }
            // Each command line starts with the command name, optionally followed by description
            if let Some(name) = trimmed.split_whitespace().next() {
                if name != "help" {
                    names.push(name.to_string());
                }
            }
        }
    }
    names
}

/// Parse the Usage line to find flags listed outside any `[...]` group.
/// Those flags are truly required (clap will reject invocations that omit them).
fn extract_required_flags(text: &str) -> HashSet<String> {
    let usage_line = match text.lines().find(|l| l.trim().starts_with("Usage:")) {
        Some(l) => l.to_string(),
        None => return HashSet::new(),
    };

    // Remove all [...] groups (optional items) iteratively to handle nesting.
    let re_brackets = regex::Regex::new(r"\[[^\[\]]*\]").unwrap();
    let mut cleaned = usage_line;
    loop {
        let next = re_brackets.replace_all(&cleaned, "").to_string();
        if next == cleaned {
            break;
        }
        cleaned = next;
    }

    // Every --flag remaining after bracket removal is required.
    let re_flags = regex::Regex::new(r"--([a-zA-Z][a-zA-Z0-9-]*)").unwrap();
    re_flags
        .captures_iter(&cleaned)
        .map(|cap| cap[1].to_string())
        .collect()
}

/// Outcome of parsing one command's help text.
pub struct ParsedHelp {
    pub parameters: Vec<Parameter>,
    /// Declaration lines that produced no parameter. Non-empty means the
    /// generated orb would silently be missing an input the CLI accepts.
    pub unparsed: Vec<String>,
}

/// True for a line that declares an option: `-f`, `-f, --force`, `--force`.
///
/// Deliberately requires a letter immediately after the dashes so that a
/// `Possible values:` continuation line (`- binary: ...` — dash, then a space) is
/// not mistaken for a new option. Description continuation lines never start
/// this way, so a line matching this always begins a new declaration no matter
/// how deeply clap indented it (#240).
fn is_option_decl(trimmed: &str) -> bool {
    let rest = trimmed
        .strip_prefix("--")
        .or_else(|| trimmed.strip_prefix('-'));
    rest.is_some_and(|r| r.starts_with(|c: char| c.is_ascii_alphabetic()))
}

/// True for the clap built-ins `-h/--help` and `-V/--version`, which are
/// deliberately excluded from the generated orb.
///
/// The built-in `--version` has no `<VALUE>` metavar; an application flag also
/// named `--version` that accepts a value must NOT be excluded — the metavar
/// tells them apart.
fn is_builtin_decl(trimmed: &str) -> bool {
    if trimmed.contains("--help") {
        return true;
    }
    if let Some(pos) = trimmed.find("--version") {
        let after = trimmed[pos + "--version".len()..].trim_start();
        return !after.starts_with('<') && !after.starts_with('[');
    }
    false
}

/// Declarations with no matching parameter, in help-text order.
fn find_unparsed(decls: &[(usize, String)], produced: &HashSet<usize>) -> Vec<String> {
    decls
        .iter()
        .filter(|(idx, _)| !produced.contains(idx))
        .map(|(_, text)| text.clone())
        .collect()
}

/// Fail generation when `--help` declared something the parser could not turn
/// into a parameter, unless the consumer opted out.
fn check_coverage(subcommand: &str, unparsed: &[String], opts: &ParseOptions) -> Result<()> {
    if unparsed.is_empty() {
        return Ok(());
    }
    let listed = unparsed
        .iter()
        .map(|d| format!("  {d}"))
        .collect::<Vec<_>>()
        .join("\n");
    if opts.allow_unparsed_help {
        tracing::warn!(
            "`{subcommand} --help` declares input that produced no orb parameter \
             (allowed by `[orb] allow_unparsed_help`):\n{listed}"
        );
        return Ok(());
    }
    anyhow::bail!(
        "`{subcommand} --help` declares input that produced no orb parameter, so the \
         generated job could not supply it:\n{listed}\n\
         Fix the CLI\'s help shape, or set `[orb] allow_unparsed_help = true` in \
         gen-circleci-orb.toml to generate anyway."
    )
}

/// Parse the `Options:` / `Arguments:` sections to build `Parameter` list.
pub fn parse_parameters(text: &str) -> Vec<Parameter> {
    parse_parameters_detailed(text).parameters
}

/// Parse the `Options:` / `Arguments:` sections, also reporting any declaration
/// that produced no parameter (see [`ParsedHelp::unparsed`]).
pub fn parse_parameters_detailed(text: &str) -> ParsedHelp {
    let required_flags = extract_required_flags(text);
    let lines: Vec<&str> = text.lines().collect();
    let mut params = Vec::new();
    // Every declaration line seen, and the subset that yielded a parameter.
    let mut decls: Vec<(usize, String)> = Vec::new();
    let mut produced: HashSet<usize> = HashSet::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // Detect section headers; skip non-option lines
        if is_top_level_section(line) || trimmed.is_empty() {
            i += 1;
            continue;
        }

        // Only process lines that declare an option
        if !is_option_decl(trimmed) {
            i += 1;
            continue;
        }

        // Skip the clap built-ins: they are not consumers' inputs.
        if is_builtin_decl(trimmed) {
            i += 1;
            continue;
        }
        decls.push((i, trimmed.to_string()));

        // Collect the full option block, which may contain blank separator
        // lines and wrapped description lines. It ends at the next declaration
        // or section header — NOT at an indentation boundary: clap aligns
        // long-only options deeper than short-flagged ones, so an indent-based
        // rule made a short-flagged option swallow the options after it (#240).
        let mut block_lines: Vec<&str> = vec![trimmed];
        let mut j = i + 1;
        while j < lines.len() {
            let next = lines[j];
            let next_trimmed = next.trim();

            if next_trimmed.is_empty() {
                // Blank lines within the block are fine — peek ahead to decide
                // whether the block continues
                let peek = peek_next_non_blank(lines.as_slice(), j + 1);
                match peek {
                    None => {
                        j += 1;
                        break;
                    }
                    Some((_, peek_line)) => {
                        let peek_trimmed = peek_line.trim();
                        if is_option_decl(peek_trimmed) || is_top_level_section(peek_line) {
                            j += 1;
                            break;
                        }
                        block_lines.push(next_trimmed); // include the blank
                        j += 1;
                    }
                }
            } else {
                if is_option_decl(next_trimmed) || is_top_level_section(next) {
                    break;
                }
                block_lines.push(next_trimmed);
                j += 1;
            }
        }

        let block = block_lines.join(" ");

        // Extract possible values from within the block text
        let possible_values = extract_possible_values_from_block(&block);

        if let Some(param) = parse_option_block(&block, possible_values, &required_flags) {
            params.push(param);
            produced.insert(i);
        }

        i = j;
    }

    let unparsed = find_unparsed(&decls, &produced);
    ParsedHelp {
        parameters: params,
        unparsed,
    }
}

fn leading_spaces(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn peek_next_non_blank<'a>(lines: &[&'a str], from: usize) -> Option<(usize, &'a str)> {
    for (offset, line) in lines[from..].iter().enumerate() {
        if !line.trim().is_empty() {
            return Some((from + offset, line));
        }
    }
    None
}

/// Extract possible values from within a collected block string.
fn extract_possible_values_from_block(block: &str) -> Vec<String> {
    // Indented block format:  "Possible values:  - name: description"
    if let Some(pos) = block.find("Possible values:") {
        let after = &block[pos + "Possible values:".len()..];
        let mut values = Vec::new();
        for part in after.split("- ") {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let val = part.split(':').next().unwrap_or(part).trim();
            if !val.is_empty() {
                values.push(val.to_string());
            }
        }
        return values;
    }
    // Inline bracket format:  "[possible values: a, b]"
    if let Some(pos) = block.find("[possible values:") {
        let after = &block[pos + "[possible values:".len()..];
        let content = after.find(']').map_or(after, |end| &after[..end]);
        return content
            .split(',')
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .collect();
    }
    Vec::new()
}

/// Parse a single collected option block string into a `Parameter`.
fn parse_option_block(
    block: &str,
    possible_values: Vec<String>,
    required_flags: &HashSet<String>,
) -> Option<Parameter> {
    // Extract long flag: look for --word
    let long_flag = extract_long_flag(block)?;
    let short = extract_short_flag(block);

    // Determine if boolean: no <VALUE> metavar after the flag
    let is_boolean = !has_value_metavar(block, &long_flag);

    let param_type = if !possible_values.is_empty() {
        ParamType::Enum(possible_values)
    } else if is_boolean {
        ParamType::Boolean
    } else {
        ParamType::String
    };

    let default = extract_default(block);
    // A flag is required only if the Usage line lists it outside any [...] group.
    let required = !is_boolean && required_flags.contains(&long_flag);

    // Description: everything after the flags portion
    let description = extract_param_description(block);

    let long_name = Parameter::normalize_name(&long_flag);

    Some(Parameter {
        long_name,
        short,
        param_type,
        default,
        required,
        description,
    })
}

fn extract_long_flag(block: &str) -> Option<String> {
    // Match --word or --word-word patterns
    let re = regex::Regex::new(r"--([a-zA-Z][a-zA-Z0-9-]*)").ok()?;
    let cap = re.captures(block)?;
    Some(cap[1].to_string())
}

fn extract_short_flag(block: &str) -> Option<char> {
    // Match -x (single char) at word boundary
    let re = regex::Regex::new(r"(?:^|[ ,])-([a-zA-Z])(?:\b|,| )").ok()?;
    let cap = re.captures(block)?;
    cap[1].chars().next()
}

fn has_value_metavar(block: &str, long_flag: &str) -> bool {
    // After --flag, is there a <VALUE> or [VALUE] metavar?
    let flag_pos = block.find(&format!("--{long_flag}"));
    if let Some(pos) = flag_pos {
        let after = &block[pos + 2 + long_flag.len()..];
        let after = after.trim_start_matches([',', ' ']);
        after.starts_with('<') || after.starts_with('[')
    } else {
        false
    }
}

fn extract_default(block: &str) -> Option<String> {
    // Find `[default:` then locate the matching outer `]` using bracket depth counting
    // so that default values containing `[...]` (e.g. "[skip ci]") are captured whole.
    let marker = "[default:";
    let start = block.find(marker)?;
    let value_start = start + marker.len();
    let mut depth = 1usize;
    let mut end = None;
    for (i, ch) in block[value_start..].char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(value_start + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end?;
    let raw = block[value_start..end].trim();
    // clap wraps string defaults in double quotes: [default: "value"].
    // Strip them so the stored default is the bare value, not `"value"`.
    let value = raw
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(raw);
    Some(value.to_string())
}

/// Remove all occurrences of `marker`...matching-`]` from `text`, handling nested brackets.
fn strip_bracket_annotation(text: &str, marker: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;
    while let Some(start) = remaining.find(marker) {
        result.push_str(remaining[..start].trim_end());
        let after = &remaining[start + marker.len()..];
        let mut depth = 1usize;
        let mut end = after.len();
        for (i, ch) in after.char_indices() {
            match ch {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        remaining = &after[end..];
    }
    result.push_str(remaining);
    result
}

fn extract_param_description(block: &str) -> String {
    // Clap metavars are UPPERCASE (e.g. <OUTPUT>, <ORB_PATH>).
    // Description text may contain lowercase angle-bracket references like
    // `<output>/<orb-dir>/` — these must NOT truncate the description.
    // Strategy: find the flag declaration (--flag [<UPPER_METAVAR>]) and take
    // everything after it as the candidate description. A repeatable option is
    // rendered `--verbose...` / `--include <PATH>...` — the marker belongs to the
    // declaration, so absorb it rather than leaving it to open the description
    // (#240).
    let re_decl = regex::Regex::new(
        r"--[a-zA-Z][a-zA-Z0-9-]*(?:\.\.\.)?(?:\s+<[A-Z][A-Z0-9_]*>(?:\.\.\.)?)?",
    )
    .unwrap();
    let candidate = if let Some(m) = re_decl.find(block) {
        block[m.end()..].trim().to_string()
    } else if let Some(pos) = block.find("  ") {
        block[pos..].trim().to_string()
    } else {
        block.to_string()
    };

    // Strip annotations: [default: ...], [possible values: ...], "Possible values: ..."
    // Use bracket-counting removal so values containing nested `[...]` are removed whole.
    let candidate = strip_bracket_annotation(&candidate, "[default:");
    let candidate = strip_bracket_annotation(&candidate, "[possible values:");
    let candidate = if let Some(pv) = candidate.find("Possible values:") {
        candidate[..pv].trim().to_string()
    } else {
        candidate
    };
    candidate.trim().to_string()
}

/// True only for top-level section headers (no leading whitespace).
/// `Possible values:` appears indented inside option blocks and is NOT a section header.
fn is_section_header(line: &str) -> bool {
    line.ends_with(':') && !line.starts_with(' ') && !line.starts_with('-')
}

/// True when the untrimmed `line` is a top-level section header.
fn is_top_level_section(line: &str) -> bool {
    is_section_header(line.trim()) && leading_spaces(line) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── top-level parsing ──────────────────────────────────────────────────

    #[test]
    fn top_level_extracts_description() {
        let help = r#"Generate MCP servers from CircleCI orb definitions

Usage: gen-orb-mcp <COMMAND>

Commands:
  generate  Generate an MCP server from an orb definition
  validate  Validate an orb definition without generating
  help      Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
"#;
        let desc = extract_description(help);
        assert_eq!(desc, "Generate MCP servers from CircleCI orb definitions");
    }

    #[test]
    fn top_level_extracts_subcommand_names() {
        let help = r#"Generate MCP servers from CircleCI orb definitions

Usage: gen-orb-mcp <COMMAND>

Commands:
  generate  Generate an MCP server from an orb definition
  validate  Validate an orb definition without generating
  diff      Compute conformance rules by diffing two orb versions
  migrate   Apply conformance-based migration
  prime     Populate prior-versions/ and migrations/ from git history
  help      Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
"#;
        let names = extract_subcommand_names(help);
        assert_eq!(
            names,
            vec!["generate", "validate", "diff", "migrate", "prime"]
        );
    }

    #[test]
    fn help_subcommand_is_skipped() {
        let help = r#"Usage: tool <COMMAND>

Commands:
  run   Run the thing
  help  Print this message
"#;
        let names = extract_subcommand_names(help);
        assert_eq!(names, vec!["run"]);
    }

    // ── parameter parsing ──────────────────────────────────────────────────

    #[test]
    fn string_default_strips_clap_double_quote_wrapper() {
        // clap renders string defaults as [default: "value"] — the outer double
        // quotes are formatting, not part of the value.  The parser must strip them.
        let help = r#"Do something

Usage: tool cmd [OPTIONS]

Options:
  -m, --message <MESSAGE>
          Commit message

          [default: "chore: update artifacts [skip ci]"]

  -h, --help
          Print help
"#;
        let params = parse_parameters(help);
        let p = params.iter().find(|p| p.long_name == "message").unwrap();
        assert_eq!(
            p.default.as_deref(),
            Some("chore: update artifacts [skip ci]"),
            "surrounding clap double-quote wrapper must be stripped from string default"
        );
    }

    #[test]
    fn boolean_flag_detected() {
        let help = r#"Run the tool

Usage: tool run [OPTIONS]

Options:
      --force
          Overwrite existing files without confirmation

  -h, --help
          Print help
"#;
        let params = parse_parameters(help);
        let force = params.iter().find(|p| p.long_name == "force").unwrap();
        assert_eq!(force.param_type, ParamType::Boolean);
        assert!(!force.required);
    }

    #[test]
    fn enum_type_detected_from_possible_values() {
        let help = r#"Generate something

Usage: tool generate [OPTIONS]

Options:
  -f, --format <FORMAT>
          Output format

          Possible values:
          - binary: Compile to native binary
          - source: Generate Rust source code

  -h, --help
          Print help
"#;
        let params = parse_parameters(help);
        let fmt = params.iter().find(|p| p.long_name == "format").unwrap();
        assert_eq!(
            fmt.param_type,
            ParamType::Enum(vec!["binary".to_string(), "source".to_string()])
        );
    }

    #[test]
    fn default_value_with_nested_brackets_extracted() {
        // clap renders defaults that contain `[...]` inside the outer [default: ...] annotation.
        // The regex [^\]]+ stops at the first `]`, truncating the value.  The bracket-counting
        // parser must find the correct matching outer `]`.
        let help = r#"Save artifacts

Usage: tool save [OPTIONS] --paths <PATHS>

Options:
  -m, --message <MESSAGE>
          Commit message

          [default: "chore: update generated MCP server artifacts [skip ci]"]

  -h, --help
          Print help
"#;
        let params = parse_parameters(help);
        let p = params.iter().find(|p| p.long_name == "message").unwrap();
        assert_eq!(
            p.default.as_deref(),
            Some("chore: update generated MCP server artifacts [skip ci]"),
            "default must be the bare value: brackets preserved, surrounding clap quotes stripped"
        );
        assert_eq!(
            p.description, "Commit message",
            "description must not contain the stray `]` from the annotation"
        );
    }

    #[test]
    fn default_value_extracted() {
        let help = r#"Generate something

Usage: tool generate [OPTIONS]

Options:
  -o, --output <OUTPUT>
          Output directory

          [default: ./dist]

  -h, --help
          Print help
"#;
        let params = parse_parameters(help);
        let out = params.iter().find(|p| p.long_name == "output").unwrap();
        assert_eq!(out.default, Some("./dist".to_string()));
        assert!(!out.required);
    }

    #[test]
    fn optional_no_default_is_not_required() {
        // A param inside [OPTIONS] with no default is optional — the CLI accepts omitting it.
        // The orb must use a mustache conditional, not pass an empty string.
        let help = r#"Validate something

Usage: tool validate [OPTIONS]

Options:
  -p, --orb-path <ORB_PATH>
          Path to the orb YAML file

  -h, --help
          Print help
"#;
        let params = parse_parameters(help);
        let p = params.iter().find(|p| p.long_name == "orb_path").unwrap();
        assert!(
            !p.required,
            "param inside [OPTIONS] with no default must not be required"
        );
        assert_eq!(p.default, None);
    }

    #[test]
    fn truly_required_detected_from_usage_line() {
        // A param listed in the Usage line outside [OPTIONS] is truly required.
        let help = r#"Generate something

Usage: tool generate [OPTIONS] --orb-path <ORB_PATH>

Options:
  -p, --orb-path <ORB_PATH>
          Path to the orb YAML file

      --name <NAME>
          Optional name

  -h, --help
          Print help
"#;
        let params = parse_parameters(help);
        let orb_path = params.iter().find(|p| p.long_name == "orb_path").unwrap();
        let name = params.iter().find(|p| p.long_name == "name").unwrap();
        assert!(orb_path.required, "flag in usage line must be required");
        assert!(
            !name.required,
            "flag only in [OPTIONS] must not be required"
        );
    }

    #[test]
    fn long_name_normalised_kebab_to_snake() {
        let help = r#"Usage: tool cmd [OPTIONS]

Options:
      --orb-path <ORB_PATH>  Path to orb
  -h, --help                 Print help
"#;
        let params = parse_parameters(help);
        assert!(
            params.iter().any(|p| p.long_name == "orb_path"),
            "expected orb_path, got: {:?}",
            params.iter().map(|p| &p.long_name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn short_flag_extracted() {
        let help = r#"Usage: tool cmd [OPTIONS]

Options:
  -p, --orb-path <ORB_PATH>  Path to orb
  -h, --help                 Print help
"#;
        let params = parse_parameters(help);
        let p = params.iter().find(|p| p.long_name == "orb_path").unwrap();
        assert_eq!(p.short, Some('p'));
    }

    #[test]
    fn help_and_version_flags_excluded() {
        let help = r#"Usage: tool cmd [OPTIONS]

Options:
  -p, --orb-path <ORB_PATH>  Path to orb
  -h, --help                 Print help
  -V, --version              Print version
"#;
        let params = parse_parameters(help);
        assert!(!params.iter().any(|p| p.long_name == "help"));
        assert!(!params.iter().any(|p| p.long_name == "version"));
    }

    #[test]
    fn app_version_flag_with_metavar_is_included() {
        // Some tools use --version as an application-level flag (e.g. "version string
        // to embed in output"). This has a <VALUE> metavar and must NOT be excluded —
        // only the clap built-in (no metavar, "Print version") should be skipped.
        let help = r#"Generate something

Usage: tool generate [OPTIONS]

Options:
  -V, --version <VERSION>
          Version string to embed in the generated output (e.g. "1.0.0")

  -h, --help
          Print help

  --other-flag
          A boolean flag for comparison
"#;
        let params = parse_parameters(help);
        assert!(
            params.iter().any(|p| p.long_name == "version"),
            "app --version <VALUE> flag must be included, got: {:?}",
            params.iter().map(|p| &p.long_name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn description_not_truncated_by_lowercase_angle_brackets_in_text() {
        // Angle brackets in description text (e.g. `<output>/<orb-dir>/`) must not
        // cause extract_param_description to truncate the description.
        let help = r#"Do something

Usage: tool cmd [OPTIONS]

Options:
      --output <OUTPUT>
          Project root directory (orb source is written to <output>/<orb-dir>/) [default: .]

  -h, --help
          Print help
"#;
        let params = parse_parameters(help);
        let p = params.iter().find(|p| p.long_name == "output").unwrap();
        assert!(
            p.description.contains("Project root directory"),
            "description truncated to {:?}",
            p.description
        );
    }

    #[test]
    fn enum_type_detected_from_inline_possible_values() {
        // clap can render possible values inline: `[possible values: a, b]`
        let help = r#"Do something

Usage: tool cmd [OPTIONS]

Options:
      --install-method <INSTALL_METHOD>
          How the binary is installed [default: binstall] [possible values: binstall, apt]

  -h, --help
          Print help
"#;
        let params = parse_parameters(help);
        let p = params
            .iter()
            .find(|p| p.long_name == "install_method")
            .unwrap();
        assert_eq!(
            p.param_type,
            ParamType::Enum(vec!["binstall".to_string(), "apt".to_string()])
        );
        assert_eq!(p.default, Some("binstall".to_string()));
    }

    #[test]
    fn clap_builtin_version_without_metavar_is_excluded() {
        // The clap built-in -V / --version has no <VALUE> metavar and prints the
        // binary version.  It must be excluded from generated orb parameters.
        let help = r#"Usage: tool cmd [OPTIONS]

Options:
  -p, --flag <FLAG>  Some flag
  -h, --help         Print help
  -V, --version      Print version
"#;
        let params = parse_parameters(help);
        assert!(
            !params.iter().any(|p| p.long_name == "version"),
            "clap built-in --version must be excluded"
        );
    }

    #[test]
    fn enum_default_combined() {
        let help = r#"Generate something

Usage: tool generate [OPTIONS]

Options:
  -f, --format <FORMAT>
          Output format

          [default: source]

          Possible values:
          - binary: Compile to native binary
          - source: Generate Rust source code

  -h, --help
          Print help
"#;
        let params = parse_parameters(help);
        let fmt = params.iter().find(|p| p.long_name == "format").unwrap();
        assert_eq!(
            fmt.param_type,
            ParamType::Enum(vec!["binary".to_string(), "source".to_string()])
        );
        assert_eq!(fmt.default, Some("source".to_string()));
        assert!(!fmt.required);
    }

    // ── binary name normalisation ──────────────────────────────────────────

    #[test]
    fn normalize_binary_name_relative_path_extracts_stem() {
        assert_eq!(
            normalize_binary_name("./target/release/gen-orb-mcp"),
            "gen-orb-mcp"
        );
    }

    #[test]
    fn normalize_binary_name_absolute_path_extracts_stem() {
        assert_eq!(
            normalize_binary_name("/usr/local/bin/gen-orb-mcp"),
            "gen-orb-mcp"
        );
    }

    #[test]
    fn normalize_binary_name_plain_name_unchanged() {
        assert_eq!(normalize_binary_name("gen-orb-mcp"), "gen-orb-mcp");
    }

    #[test]
    fn normalize_binary_name_nested_relative_path() {
        assert_eq!(normalize_binary_name("../../some/path/mytool"), "mytool");
    }

    // ── mixed-indent option blocks (#240) ──────────────────────────────────

    /// clap's compact help aligns long-only options DEEPER than options that
    /// also carry a short form.  Terminating a block on indentation alone made
    /// a short-flagged option swallow every long-only option after it, so those
    /// produced no parameter at all — silently.
    #[test]
    fn long_only_option_after_short_flagged_option_is_not_swallowed() {
        let help = r#"Re-verify a past release

Usage: jci-audit verify [OPTIONS] --release-version <VERSION>

Options:
  -v, --verbose...                 Increase logging verbosity
      --release-version <VERSION>  The released version to verify (e.g. "1.2.0")
      --advisory-db <ADVISORY_DB>  Advisory-db root; cargo-deny's checkout is moved to the recorded commit beneath it
  -q, --quiet...                   Decrease logging verbosity
      --deny-warnings              Fail if the tools report any warning
  -h, --help                       Print help
"#;
        let names: Vec<String> = parse_parameters(help)
            .iter()
            .map(|p| p.long_name.clone())
            .collect();
        for expected in [
            "verbose",
            "release_version",
            "advisory_db",
            "quiet",
            "deny_warnings",
        ] {
            assert!(
                names.iter().any(|n| n == expected),
                "expected parameter `{expected}`, got: {names:?}"
            );
        }
    }

    /// The repeat marker on `--verbose...` must be treated as part of the flag
    /// token, not as the start of the description.
    #[test]
    fn repeat_marker_does_not_leak_into_description() {
        let help = r#"Do something

Usage: tool cmd [OPTIONS]

Options:
  -v, --verbose...  Increase logging verbosity
  -h, --help        Print help
"#;
        let params = parse_parameters(help);
        let verbose = params.iter().find(|p| p.long_name == "verbose").unwrap();
        assert_eq!(verbose.description, "Increase logging verbosity");
        assert_eq!(verbose.param_type, ParamType::Boolean);
    }

    /// A repeatable option that DOES take a value keeps its metavar handling.
    #[test]
    fn repeat_marker_after_metavar_does_not_leak_into_description() {
        let help = r#"Do something

Usage: tool cmd [OPTIONS]

Options:
  -I, --include <PATH>...  Directory to search
  -h, --help               Print help
"#;
        let params = parse_parameters(help);
        let include = params.iter().find(|p| p.long_name == "include").unwrap();
        assert_eq!(include.description, "Directory to search");
        assert_eq!(include.param_type, ParamType::String);
    }

    // ── coverage guard ─────────────────────────────────────────────────────

    /// The guard reports every declaration line that produced no parameter.
    #[test]
    fn unparsed_reports_declaration_that_yielded_no_parameter() {
        let decls = vec![
            (5usize, "      --alpha <A>  The alpha".to_string()),
            (6usize, "      --beta <B>   The beta".to_string()),
        ];
        let produced: HashSet<usize> = [5usize].into_iter().collect();
        let unparsed = find_unparsed(&decls, &produced);
        assert_eq!(unparsed.len(), 1, "one declaration produced no parameter");
        assert!(
            unparsed[0].contains("--beta"),
            "guard must name the offending declaration, got: {unparsed:?}"
        );
    }

    /// Ordinary help text is fully covered — the guard must not false-positive
    /// on the clap built-ins or on `Possible values:` continuation lines.
    #[test]
    fn unparsed_is_empty_for_fully_parsed_help() {
        let help = r#"Generate something

Usage: tool generate [OPTIONS] --orb-path <ORB_PATH>

Options:
  -p, --orb-path <ORB_PATH>
          Path to the orb YAML file

  -f, --format <FORMAT>
          Output format

          [default: source]

          Possible values:
          - binary: Compile to native binary
          - source: Generate Rust source code

      --force
          Overwrite existing files

  -h, --help
          Print help

  -V, --version
          Print version
"#;
        let parsed = parse_parameters_detailed(help);
        assert!(
            parsed.unparsed.is_empty(),
            "fully parseable help must report nothing unparsed, got: {:?}",
            parsed.unparsed
        );
        assert_eq!(parsed.parameters.len(), 3);
    }

    /// A short-only option produces no parameter today (#241).  The guard's job
    /// is to make that visible instead of letting generation report success.
    #[test]
    fn unparsed_reports_short_only_option() {
        let help = r#"Do something

Usage: tool cmd [OPTIONS]

Options:
  -f          Force the operation
  -h, --help  Print help
"#;
        let parsed = parse_parameters_detailed(help);
        assert_eq!(
            parsed.unparsed.len(),
            1,
            "short-only option must be reported, got: {:?}",
            parsed.unparsed
        );
        assert!(parsed.unparsed[0].contains("-f"));
    }

    /// `parse_subcommand` fails loudly when a declaration produced no
    /// parameter, and the escape hatch downgrades that to a warning.
    #[test]
    fn subcommand_parse_fails_on_unparsed_declaration() {
        let help = r#"Do something

Usage: tool cmd [OPTIONS]

Options:
  -f          Force the operation
  -h, --help  Print help
"#;
        let err = parse_subcommand("cmd", help, "tool", &ParseOptions::default())
            .expect_err("unparsed declaration must fail generation");
        let msg = err.to_string();
        assert!(
            msg.contains("-f") && msg.contains("allow_unparsed_help"),
            "error must name the declaration and the escape hatch, got: {msg}"
        );

        let opts = ParseOptions {
            allow_unparsed_help: true,
        };
        parse_subcommand("cmd", help, "tool", &opts)
            .expect("allow_unparsed_help must downgrade the failure to a warning");
    }
}
