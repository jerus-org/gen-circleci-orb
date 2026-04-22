use super::run_help;
use super::types::{CliDefinition, ParamType, Parameter, SubCommand};
use anyhow::Result;

/// Parse the top-level `--help` output for `binary` and recursively fetch
/// help for each discovered subcommand.
pub fn parse_top_level(binary: &str, help_text: &str) -> Result<CliDefinition> {
    let description = extract_description(help_text);
    let sub_names = extract_subcommand_names(help_text);

    let mut subcommands = Vec::new();
    for name in sub_names {
        let sub_help = run_help(binary, &[&name])?;
        let sub = parse_subcommand(&name, &sub_help, binary)?;
        subcommands.push(sub);
    }

    Ok(CliDefinition {
        binary_name: binary.to_string(),
        description,
        subcommands,
    })
}

fn parse_subcommand(name: &str, help_text: &str, binary: &str) -> Result<SubCommand> {
    let description = extract_description(help_text);
    let child_names = extract_subcommand_names(help_text);
    let is_leaf = child_names.is_empty();

    let mut subcommands = Vec::new();
    for child_name in &child_names {
        let child_help = run_help(binary, &[name, child_name])?;
        let child = parse_subcommand(child_name, &child_help, binary)?;
        subcommands.push(child);
    }

    let parameters = if is_leaf {
        parse_parameters(help_text)
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

/// Parse the `Options:` / `Arguments:` sections to build `Parameter` list.
pub fn parse_parameters(text: &str) -> Vec<Parameter> {
    let lines: Vec<&str> = text.lines().collect();
    let mut params = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // Detect section headers; skip non-option lines
        if is_top_level_section(line) || trimmed.is_empty() {
            i += 1;
            continue;
        }

        // Only process lines that look like flags (start with - after trimming)
        if !trimmed.starts_with('-') {
            i += 1;
            continue;
        }

        // Skip -h/--help and -V/--version built-ins
        if trimmed.contains("--help") || trimmed.contains("--version") {
            i += 1;
            continue;
        }

        // Determine indentation of this flag line so we can collect its
        // full description block, which may contain blank separator lines.
        let flag_indent = leading_spaces(line);

        // Collect the full option block using indentation: gather all lines
        // until we hit a non-blank line that is at flag_indent or less AND
        // starts a new flag or section header.
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
                        let peek_indent = leading_spaces(peek_line);
                        let peek_trimmed = peek_line.trim();
                        // If the next non-blank line is indented MORE than the flag
                        // it belongs to this block; otherwise the block is done.
                        if peek_indent > flag_indent
                            && !peek_trimmed.starts_with('-')
                            && !is_top_level_section(peek_line)
                        {
                            block_lines.push(next_trimmed); // include the blank
                            j += 1;
                        } else {
                            j += 1;
                            break;
                        }
                    }
                }
            } else {
                let indent = leading_spaces(next);
                if indent <= flag_indent
                    && (next_trimmed.starts_with('-') || is_top_level_section(next))
                {
                    break;
                }
                block_lines.push(next_trimmed);
                j += 1;
            }
        }

        let block = block_lines.join(" ");

        // Extract possible values from within the block text
        let possible_values = extract_possible_values_from_block(&block);

        if let Some(param) = parse_option_block(&block, possible_values) {
            params.push(param);
        }

        i = j;
    }
    params
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
    // Find "Possible values:" in the block text
    if let Some(pos) = block.find("Possible values:") {
        let after = &block[pos + "Possible values:".len()..];
        let mut values = Vec::new();
        // Values appear as "- name: description" or "- name"
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
        values
    } else {
        Vec::new()
    }
}

/// Parse a single collected option block string into a `Parameter`.
fn parse_option_block(block: &str, possible_values: Vec<String>) -> Option<Parameter> {
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
    let required = !is_boolean && default.is_none();

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
    let re = regex::Regex::new(r"\[default:\s*([^\]]+)\]").ok()?;
    let cap = re.captures(block)?;
    Some(cap[1].trim().to_string())
}

fn extract_param_description(block: &str) -> String {
    // For multi-line blocks joined with spaces, description comes:
    // 1. After the closing `>` of a metavar (e.g. `--flag <VALUE>  description`)
    // 2. After the flag itself for boolean flags (e.g. `--force  description`)
    // 3. Via double-space separator on single-line help text
    let candidate = if let Some(pos) = block.rfind('>') {
        block[pos + 1..].trim().to_string()
    } else if let Some(pos) = block.find("  ") {
        block[pos..].trim().to_string()
    } else {
        // Find content after the last flag token
        let re = regex::Regex::new(r"--[a-zA-Z][a-zA-Z0-9-]*").unwrap();
        let last_end = re
            .find_iter(block)
            .last()
            .map(|m| m.end())
            .unwrap_or(block.len());
        block[last_end..].trim().to_string()
    };

    // Remove [default: ...] annotations from description
    let re = regex::Regex::new(r"\s*\[default:[^\]]*\]").unwrap();
    // Also remove "Possible values: ..." section if still present
    let candidate = if let Some(pv) = candidate.find("Possible values:") {
        candidate[..pv].trim().to_string()
    } else {
        candidate
    };
    re.replace_all(&candidate, "").trim().to_string()
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
    fn required_param_detected_when_no_default() {
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
        assert!(p.required);
        assert_eq!(p.default, None);
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
}
