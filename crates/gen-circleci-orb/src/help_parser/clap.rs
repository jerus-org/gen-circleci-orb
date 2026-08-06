use std::collections::{HashMap, HashSet};

use super::types::{CliDefinition, ParamKind, ParamType, Parameter, SubCommand};
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
        let parsed = parse_parameters_detailed(help_text, name, opts);
        check_naming(name, &parsed.errors)?;
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
///
/// `help` is excluded because clap synthesises it for every command that has
/// subcommands; it is not part of the CLI's own surface and has no place in the
/// generated orb.
pub fn extract_subcommand_names(text: &str) -> Vec<String> {
    commands_block(text)
        // Each command line starts with the name, optionally followed by a
        // description.
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| *name != "help")
        .map(str::to_string)
        .collect()
}

/// The non-empty lines of the `Commands:` block, in order.
///
/// Empty when the help text has no such block. Locating the block is kept
/// separate from reading names out of it so neither has to be understood while
/// following the other — the two were interleaved in one walk, and the nesting
/// that produced was the whole of its complexity (#257).
fn commands_block(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        // Everything up to and including the header is preamble. When the
        // header is absent this consumes the lot, which is the empty case.
        .skip_while(|line| line.trim() != "Commands:")
        .skip(1)
        // Blank lines are spacing, not the end of the block.
        .filter(|line| !line.trim().is_empty())
        // An entry in the block is indented; a section header and any
        // after-help prose are at column 0. Indentation is therefore the whole
        // rule, and testing it on the *untrimmed* line is the point — trimming
        // first discards exactly the thing that distinguishes them, which is
        // how a description ending in a colon came to look like a header and
        // truncate the block (#278), and how prose following a trailing
        // `Commands:` section came to be read as subcommands (#280).
        .take_while(|line| leading_spaces(line) > 0)
        .map(str::trim)
}

/// Parse the Usage line to find flags listed outside any `[...]` group.
/// Those flags are truly required (clap will reject invocations that omit them).
fn extract_required_flags(text: &str) -> HashSet<String> {
    let Some(cleaned) = usage_line_without_optional_groups(text) else {
        return HashSet::new();
    };

    // Every --flag remaining after bracket removal is required.
    let re_flags = regex::Regex::new(r"--([a-zA-Z][a-zA-Z0-9-]*)").unwrap();
    re_flags
        .captures_iter(&cleaned)
        .map(|cap| cap[1].to_string())
        .collect()
}

/// Parse the Usage line for short flags listed outside any `[...]` group.
///
/// An option with a long form is found by [`extract_required_flags`]; this is
/// the same signal for one that has only a short form, so a required `-f <VAL>`
/// generates a parameter CircleCI insists on rather than one the script quietly
/// omits.
fn extract_required_shorts(text: &str) -> HashSet<char> {
    let Some(cleaned) = usage_line_without_optional_groups(text) else {
        return HashSet::new();
    };
    // `--long` also matches a naive `-x` pattern, so require the dash to open
    // the token and to be followed by exactly one letter.
    let re = regex::Regex::new(r"(?:^|\s)-([a-zA-Z])(?:\s|$)").unwrap();
    re.captures_iter(&cleaned)
        .filter_map(|cap| cap[1].chars().next())
        .collect()
}

/// The `Usage:` line with every `[...]` group removed, so what remains is
/// exactly what clap requires. `None` when the help text has no usage line.
fn usage_line_without_optional_groups(text: &str) -> Option<String> {
    let usage_line = text
        .lines()
        .find(|l| l.trim().starts_with("Usage:"))?
        .to_string();

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
    Some(cleaned)
}

/// Parse the Usage line for positionals listed outside any `[...]` group,
/// returning their normalised parameter names.
///
/// clap renders an optional positional as `[TARGET]`, which the same
/// bracket-stripping that finds required flags also removes — so whatever
/// `<NAME>` survives is required.
fn extract_required_positionals(text: &str) -> HashSet<String> {
    let Some(cleaned) = usage_line_without_optional_groups(text) else {
        return HashSet::new();
    };

    let re_positional = regex::Regex::new(r"<([A-Za-z][A-Za-z0-9_-]*)>").unwrap();
    re_positional
        .captures_iter(&cleaned)
        .map(|cap| normalize_metavar(&cap[1]))
        .collect()
}

/// `<ADVISORY_DB>` / `[TARGET]` → `advisory_db` / `target`.
fn normalize_metavar(metavar: &str) -> String {
    Parameter::normalize_name(&metavar.trim().to_lowercase())
}

/// Outcome of parsing one command's help text.
pub struct ParsedHelp {
    pub parameters: Vec<Parameter>,
    /// Declaration lines that produced no parameter. Non-empty means the
    /// generated orb would silently be missing an input the CLI accepts.
    pub unparsed: Vec<String>,
    /// Declarations the parser understood but cannot name safely — a short-only
    /// option with no derivable name, or two inputs claiming the same name.
    /// Unlike [`ParsedHelp::unparsed`] these are not waved through by
    /// `allow_unparsed_help`: the fix is to name the parameter, not to drop it.
    pub errors: Vec<String>,
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

/// True for a line that declares a positional argument: `<VERSION>` (required)
/// or `[TARGET]` (optional), as rendered under `Arguments:`.
fn is_positional_decl(trimmed: &str) -> bool {
    let re = regex::Regex::new(r"^[<\[][A-Za-z][A-Za-z0-9_-]*[>\]]").unwrap();
    re.is_match(trimmed)
}

/// True for any line that opens a new declaration block, of either kind.
fn is_decl(trimmed: &str) -> bool {
    is_option_decl(trimmed) || is_positional_decl(trimmed)
}

/// Words that name nothing on their own, so cannot seed a parameter name.
const UNUSABLE_NAME_WORDS: &[&str] = &[
    "a", "an", "the", "this", "that", "these", "those", "how", "when", "if", "whether", "and",
    "or", "for", "of", "to", "in", "on", "at", "by", "with", "is", "are", "be", "do", "does",
];

/// A parameter name must be usable as a shell variable, because that is what it
/// becomes: the generated script reads the value from `${NAME}`. A name opening
/// with a digit would render `${60}` — a positional-parameter read, not a
/// variable — so the script would be silently wrong rather than fail to
/// generate.
fn is_usable_param_name(name: &str) -> bool {
    name.len() >= 2
        && name.starts_with(|c: char| c.is_ascii_alphabetic())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !UNUSABLE_NAME_WORDS.contains(&name)
}

/// Name a short-only option: the consumer's chosen name, else the first word of
/// its description. `None` when neither yields something usable.
fn resolve_short_only_name(
    short: char,
    description: &str,
    configured: Option<&HashMap<char, String>>,
) -> Option<String> {
    // The configured name is held to the same rule as a derived one — it ends up
    // in the same shell variable.
    if let Some(name) = configured
        .and_then(|m| m.get(&short))
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        let name = Parameter::normalize_name(&name.to_lowercase());
        return is_usable_param_name(&name).then_some(name);
    }
    let first: String = description
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase();
    is_usable_param_name(&first).then_some(first)
}

/// Declarations with no matching parameter, in help-text order.
fn find_unparsed(decls: &[(usize, String)], produced: &HashSet<usize>) -> Vec<String> {
    decls
        .iter()
        .filter(|(idx, _)| !produced.contains(idx))
        .map(|(_, text)| text.clone())
        .collect()
}

/// Fail generation when an input cannot be named safely.
///
/// Deliberately NOT covered by `allow_unparsed_help`: the parser understood
/// these declarations, so the fix is to name the parameter (in config or in the
/// CLI), not to let the orb go out without the input.
fn check_naming(subcommand: &str, errors: &[String]) -> Result<()> {
    if errors.is_empty() {
        return Ok(());
    }
    let listed = errors
        .iter()
        .map(|e| format!("  {e}"))
        .collect::<Vec<_>>()
        .join("\n");
    anyhow::bail!("`{subcommand} --help` declares input that cannot be named:\n{listed}")
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
    parse_parameters_detailed(text, "", &ParseOptions::default()).parameters
}

/// Parse the `Options:` / `Arguments:` sections, also reporting declarations
/// that produced no parameter (see [`ParsedHelp::unparsed`]) and ones that
/// cannot be named (see [`ParsedHelp::errors`]).
pub fn parse_parameters_detailed(text: &str, subcommand: &str, opts: &ParseOptions) -> ParsedHelp {
    let required = RequiredInputs {
        flags: extract_required_flags(text),
        shorts: extract_required_shorts(text),
        positionals: extract_required_positionals(text),
    };
    let short_names = opts.short_param_names.get(subcommand);
    let lines: Vec<&str> = text.lines().collect();
    let mut params = Vec::new();
    // Every declaration line seen, and the subset that yielded a parameter.
    let mut decls: Vec<(usize, String)> = Vec::new();
    let mut produced: HashSet<usize> = HashSet::new();
    let mut errors: Vec<String> = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let trimmed = lines[i].trim();

        if !is_decl(trimmed) || is_top_level_section(lines[i]) {
            i += 1;
            continue;
        }
        // Skip the clap built-ins: they are not consumers' inputs.
        if is_builtin_decl(trimmed) {
            i += 1;
            continue;
        }

        decls.push((i, trimmed.to_string()));
        let (block, next) = collect_block(&lines, i);
        let possible_values = extract_possible_values_from_block(&block);

        let parsed = if is_positional_decl(trimmed) {
            parse_positional_block(&block, possible_values, &required.positionals)
        } else {
            parse_option_block(&block, possible_values, &required, short_names, &mut errors)
        };
        if let Some(param) = parsed {
            params.push(param);
            produced.insert(i);
        }

        i = next;
    }

    errors.extend(name_collisions(&params));
    let unparsed = find_unparsed(&decls, &produced);
    ParsedHelp {
        parameters: params,
        unparsed,
        errors,
    }
}

/// Gather one declaration's full block, starting at `start`, and report the
/// line the caller should resume from.
///
/// The block runs to the next declaration or section header — NOT to an
/// indentation boundary: clap aligns long-only options deeper than
/// short-flagged ones, so an indent-based rule let a short-flagged option
/// swallow the options after it (#240). Blank separator lines and wrapped
/// description lines belong to the block.
fn collect_block(lines: &[&str], start: usize) -> (String, usize) {
    let mut block_lines: Vec<&str> = vec![lines[start].trim()];
    let mut j = start + 1;

    while j < lines.len() {
        let next_trimmed = lines[j].trim();

        if next_trimmed.is_empty() {
            // A blank line only ends the block if what follows starts a new one.
            match peek_next_non_blank(lines, j + 1) {
                Some((_, peek)) if !ends_block(peek) => {
                    block_lines.push(next_trimmed); // include the blank
                    j += 1;
                }
                _ => {
                    j += 1;
                    break;
                }
            }
        } else if ends_block(lines[j]) {
            break;
        } else {
            block_lines.push(next_trimmed);
            j += 1;
        }
    }

    (block_lines.join(" "), j)
}

/// True when `line` opens something new, so the block before it is complete.
fn ends_block(line: &str) -> bool {
    is_decl(line.trim()) || is_top_level_section(line)
}

/// Report parameters that resolved to the same orb parameter name — only one of
/// them would survive into the generated orb.
fn name_collisions(params: &[Parameter]) -> Vec<String> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut clashes: Vec<String> = Vec::new();
    for p in params {
        if !seen.insert(p.long_name.as_str()) && !clashes.iter().any(|c| c.contains(&p.long_name)) {
            clashes.push(format!(
                "two inputs both resolve to the parameter name `{}` — rename one, \
                 or set `[subcommand.<name>.short_param]` to name a short-only option \
                 differently",
                p.long_name
            ));
        }
    }
    clashes
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

/// What the `Usage:` line lists outside any `[...]` group — i.e. what clap
/// rejects an invocation for omitting.
pub(crate) struct RequiredInputs {
    pub flags: HashSet<String>,
    pub shorts: HashSet<char>,
    pub positionals: HashSet<String>,
}

/// Parse a single collected option block string into a `Parameter`.
///
/// An option with no long form is named from `short_names` or from its
/// description; when neither yields a usable name the block produces no
/// parameter and the reason is pushed to `errors` (#241).
fn parse_option_block(
    block: &str,
    possible_values: Vec<String>,
    required: &RequiredInputs,
    short_names: Option<&HashMap<char, String>>,
    errors: &mut Vec<String>,
) -> Option<Parameter> {
    let short = extract_short_flag(block);
    let Some(long_flag) = extract_long_flag(block) else {
        return parse_short_only_block(
            block,
            possible_values,
            short,
            required,
            short_names,
            errors,
        );
    };

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
    let required = !is_boolean && required.flags.contains(&long_flag);

    // Description: everything after the flags portion
    let description = extract_param_description(block);

    let long_name = Parameter::normalize_name(&long_flag);

    Some(Parameter {
        long_name,
        short,
        kind: ParamKind::Long,
        param_type,
        default,
        required,
        description,
    })
}

/// Build the parameter for an option that has only a short form (`-f`).
fn parse_short_only_block(
    block: &str,
    possible_values: Vec<String>,
    short: Option<char>,
    required: &RequiredInputs,
    short_names: Option<&HashMap<char, String>>,
    errors: &mut Vec<String>,
) -> Option<Parameter> {
    let short = short?;
    let description = extract_short_only_description(block);
    let Some(long_name) = resolve_short_only_name(short, &description, short_names) else {
        errors.push(format!(
            "`-{short}` has no long form and no name can be derived from its \
             description ({description:?}) — name it with \
             `[subcommand.<name>.short_param] {short} = \"some_name\"` in \
             gen-circleci-orb.toml"
        ));
        return None;
    };

    // A short-only option takes a value when a metavar follows it.
    let is_boolean = !has_short_value_metavar(block, short);
    let param_type = if !possible_values.is_empty() {
        ParamType::Enum(possible_values)
    } else if is_boolean {
        ParamType::Boolean
    } else {
        ParamType::String
    };

    Some(Parameter {
        long_name,
        short: Some(short),
        kind: ParamKind::ShortOnly,
        param_type,
        default: extract_default(block),
        // Required on the same signal as every other input: listed in the Usage
        // line outside any `[...]` group.
        required: !is_boolean && required.shorts.contains(&short),
        description,
    })
}

/// Parse a `<VERSION>` / `[TARGET]` block from the `Arguments:` section (#242).
fn parse_positional_block(
    block: &str,
    possible_values: Vec<String>,
    required_positionals: &HashSet<String>,
) -> Option<Parameter> {
    let re = regex::Regex::new(r"^[<\[]([A-Za-z][A-Za-z0-9_-]*)[>\]](\.\.\.)?").ok()?;
    let cap = re.captures(block.trim())?;
    let long_name = normalize_metavar(&cap[1]);

    let param_type = if possible_values.is_empty() {
        ParamType::String
    } else {
        ParamType::Enum(possible_values)
    };

    let description = extract_positional_description(block);
    let default = extract_default(block);
    // `<NAME>` is required unless the Usage line brackets it; a default makes it
    // optional whatever the brackets say.
    let required = default.is_none() && required_positionals.contains(&long_name);

    Some(Parameter {
        long_name,
        short: None,
        kind: ParamKind::Positional,
        param_type,
        default,
        required,
        description,
    })
}

/// True when a value metavar follows the short flag: `-n <COUNT>`.
fn has_short_value_metavar(block: &str, short: char) -> bool {
    let needle = format!("-{short}");
    let Some(pos) = block.find(&needle) else {
        return false;
    };
    let after = block[pos + needle.len()..].trim_start_matches([',', ' ']);
    after.starts_with('<') || after.starts_with('[')
}

/// Description of a short-only option: everything after `-f` and its metavar.
fn extract_short_only_description(block: &str) -> String {
    let re = regex::Regex::new(r"^-[a-zA-Z](?:\.\.\.)?(?:\s+[<\[][A-Z][A-Z0-9_]*[>\]])?").unwrap();
    let rest = match re.find(block.trim()) {
        Some(m) => block.trim()[m.end()..].trim(),
        None => block.trim(),
    };
    clean_annotations(rest)
}

/// Description of a positional: everything after the `<NAME>` declaration.
fn extract_positional_description(block: &str) -> String {
    let re = regex::Regex::new(r"^[<\[][A-Za-z][A-Za-z0-9_-]*[>\]](?:\.\.\.)?").unwrap();
    let trimmed = block.trim();
    let rest = match re.find(trimmed) {
        Some(m) => trimmed[m.end()..].trim(),
        None => trimmed,
    };
    clean_annotations(rest)
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

    clean_annotations(&candidate)
}

/// Strip the annotations clap appends to a description: `[default: …]`,
/// `[possible values: …]`, and the indented `Possible values:` block.
///
/// Bracket-counting removal so values containing nested `[...]` go whole.
fn clean_annotations(text: &str) -> String {
    let candidate = strip_bracket_annotation(text, "[default:");
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

    // ── #257: the edges of the Commands: block ─────────────────────────────

    #[test]
    fn no_commands_section_yields_nothing() {
        let help = r#"Usage: tool [OPTIONS]

Options:
  -h, --help  Print help
"#;
        assert!(extract_subcommand_names(help).is_empty());
    }

    /// The block ends at the next section header, so anything after it — an
    /// option, a value — must never be mistaken for a subcommand.
    #[test]
    fn the_block_ends_at_the_next_section_header() {
        let help = r#"Usage: tool <COMMAND>

Commands:
  run   Run the thing

Options:
  -h, --help     Print help
  -V, --version  Print version
"#;
        assert_eq!(extract_subcommand_names(help), vec!["run"]);
    }

    /// A blank line inside the block is spacing, not a terminator: clap emits
    /// one between groups of commands.
    #[test]
    fn a_blank_line_inside_the_block_does_not_end_it() {
        let help = r#"Usage: tool <COMMAND>

Commands:
  run     Run the thing

  verify  Verify the thing
"#;
        assert_eq!(extract_subcommand_names(help), vec!["run", "verify"]);
    }

    /// Only the name is taken, however widely the description is spaced.
    #[test]
    fn only_the_first_word_of_each_line_is_taken() {
        let help = r#"Usage: tool <COMMAND>

Commands:
  run             Run the thing
  verify   Verify it
"#;
        assert_eq!(extract_subcommand_names(help), vec!["run", "verify"]);
    }

    /// A description ending in a colon must not be mistaken for a section
    /// header (#278). It was: the terminator test ran on the trimmed line, so
    /// the indentation that tells a command line from a header had already been
    /// removed — truncating the block and silently dropping that subcommand and
    /// every one after it from the generated orb.
    #[test]
    fn a_description_ending_in_a_colon_does_not_end_the_block() {
        let help = r#"Usage: tool <COMMAND>

Commands:
  run     Run the thing
  verify  Verify it, including:
  deploy  Deploy the thing
"#;
        assert_eq!(
            extract_subcommand_names(help),
            vec!["run", "verify", "deploy"]
        );
    }

    /// The guard that distinguishes them is indentation, so a real header at
    /// column 0 must still end the block.
    #[test]
    fn an_unindented_header_still_ends_the_block() {
        let help = r#"Usage: tool <COMMAND>

Commands:
  run   Run the thing
Options:
  -h, --help  Print help
"#;
        assert_eq!(extract_subcommand_names(help), vec!["run"]);
    }

    /// When `Commands:` is the last section, clap's after-help line follows it
    /// at column 0. Reading that as a command produced a fully-formed bogus job
    /// in the published orb (#280) — the prose's first word, `Run`, became a
    /// subcommand whose help text was clap's own "unrecognized subcommand"
    /// error.
    #[test]
    fn after_help_prose_following_the_block_is_not_read_as_commands() {
        let help = r#"A tool

Usage: tool <COMMAND>

Commands:
  run   Run the thing

Run `tool help <cmd>` for more information.
"#;
        assert_eq!(extract_subcommand_names(help), vec!["run"]);
    }

    /// A `Commands:` block that runs to the end of the help text has no
    /// terminating header, so the walk has to stop at end of input.
    #[test]
    fn a_block_running_to_end_of_input_is_read_whole() {
        let help = "Usage: tool <COMMAND>\n\nCommands:\n  run   Run it\n  stop  Stop it";
        assert_eq!(extract_subcommand_names(help), vec!["run", "stop"]);
    }

    /// Everything before the header is skipped, even a line that looks like a
    /// command listing.
    #[test]
    fn lines_before_the_commands_header_are_ignored() {
        let help = r#"A tool that does things

  looks-like-a-command  but is prose

Usage: tool <COMMAND>

Commands:
  run   Run the thing
"#;
        assert_eq!(extract_subcommand_names(help), vec!["run"]);
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
        let parsed = parse_parameters_detailed(help, "cmd", &ParseOptions::default());
        assert!(
            parsed.unparsed.is_empty(),
            "fully parseable help must report nothing unparsed, got: {:?}",
            parsed.unparsed
        );
        assert_eq!(parsed.parameters.len(), 3);
    }

    /// An unparsed declaration fails generation, naming the line, and the
    /// escape hatch downgrades that to a warning.
    #[test]
    fn unparsed_declaration_fails_generation_unless_allowed() {
        let unparsed = vec!["--mystery-shape <X>  Something new".to_string()];

        let err = check_coverage("cmd", &unparsed, &ParseOptions::default())
            .expect_err("unparsed declaration must fail generation");
        let msg = err.to_string();
        assert!(
            msg.contains("--mystery-shape") && msg.contains("allow_unparsed_help"),
            "error must name the declaration and the escape hatch, got: {msg}"
        );

        let opts = ParseOptions {
            allow_unparsed_help: true,
            ..ParseOptions::default()
        };
        check_coverage("cmd", &unparsed, &opts)
            .expect("allow_unparsed_help must downgrade the failure to a warning");
    }
    // ── positional arguments (#242) ────────────────────────────────────────

    /// A required positional is the natural shape for a single mandatory
    /// argument (`pcu release version <VERSION>`). It must reach the orb.
    #[test]
    fn required_positional_generates_parameter() {
        let help = r#"Re-verify a past release

Usage: jci-audit verify [OPTIONS] <VERSION>

Arguments:
  <VERSION>  The released version to verify (e.g. "1.2.0")

Options:
      --advisory-db <ADVISORY_DB>  Advisory-db root
  -h, --help                       Print help
"#;
        let params = parse_parameters(help);
        let version = params
            .iter()
            .find(|p| p.long_name == "version")
            .unwrap_or_else(|| panic!("no `version` parameter in {params:?}"));
        assert_eq!(version.kind, ParamKind::Positional);
        assert_eq!(version.param_type, ParamType::String);
        assert!(
            version.required,
            "<VERSION> outside [...] in Usage is required"
        );
        assert_eq!(
            version.description,
            r#"The released version to verify (e.g. "1.2.0")"#
        );
        assert!(params.iter().any(|p| p.long_name == "advisory_db"));
    }

    /// `[NAME]` is optional, and so is a `<NAME>` that the Usage line brackets.
    #[test]
    fn optional_positional_is_not_required() {
        let help = r#"Do something

Usage: tool cmd [OPTIONS] [TARGET]

Arguments:
  [TARGET]  Where to write the output

Options:
  -h, --help  Print help
"#;
        let params = parse_parameters(help);
        let target = params.iter().find(|p| p.long_name == "target").unwrap();
        assert_eq!(target.kind, ParamKind::Positional);
        assert!(!target.required);
    }

    /// Positionals carry `[default: …]` and possible values like any option.
    #[test]
    fn positional_default_and_possible_values_extracted() {
        let help = r#"Do something

Usage: tool cmd [OPTIONS] [FORMAT]

Arguments:
  [FORMAT]  Output format [default: source] [possible values: binary, source]

Options:
  -h, --help  Print help
"#;
        let params = parse_parameters(help);
        let format = params.iter().find(|p| p.long_name == "format").unwrap();
        assert_eq!(format.default.as_deref(), Some("source"));
        assert_eq!(
            format.param_type,
            ParamType::Enum(vec!["binary".to_string(), "source".to_string()])
        );
    }

    /// A variadic positional keeps its name; the repeat marker is not part of it.
    #[test]
    fn variadic_positional_strips_repeat_marker() {
        let help = r#"Do something

Usage: tool cmd [OPTIONS] <PATHS>...

Arguments:
  <PATHS>...  Files to process

Options:
  -h, --help  Print help
"#;
        let params = parse_parameters(help);
        let paths = params.iter().find(|p| p.long_name == "paths").unwrap();
        assert_eq!(paths.kind, ParamKind::Positional);
        assert_eq!(paths.description, "Files to process");
    }

    /// Multi-paragraph help renders arguments over several lines.
    #[test]
    fn positional_parsed_in_long_help_layout() {
        let help = r#"Do something

Usage: tool cmd [OPTIONS] <VERSION>

Arguments:
  <VERSION>
          The version to release

Options:
  -h, --help
          Print help
"#;
        let params = parse_parameters(help);
        let version = params.iter().find(|p| p.long_name == "version").unwrap();
        assert_eq!(version.description, "The version to release");
        assert!(version.required);
    }

    /// Positionals must not be counted as unparsed by the coverage guard.
    #[test]
    fn positional_is_covered_by_the_guard() {
        let help = r#"Do something

Usage: tool cmd [OPTIONS] <VERSION>

Arguments:
  <VERSION>  The version to release

Options:
  -h, --help  Print help
"#;
        let parsed = parse_parameters_detailed(help, "cmd", &ParseOptions::default());
        assert!(parsed.unparsed.is_empty(), "got: {:?}", parsed.unparsed);
        assert!(parsed.errors.is_empty(), "got: {:?}", parsed.errors);
    }

    // ── short-only options (#241) ──────────────────────────────────────────

    /// With no long form to name it after, the parameter takes its name from
    /// the first word of the description.
    #[test]
    fn short_only_option_named_from_description() {
        let help = r#"Do something

Usage: tool cmd [OPTIONS]

Options:
  -f          Force the operation
  -h, --help  Print help
"#;
        let parsed = parse_parameters_detailed(help, "cmd", &ParseOptions::default());
        assert!(parsed.errors.is_empty(), "got: {:?}", parsed.errors);
        let force = parsed
            .parameters
            .iter()
            .find(|p| p.long_name == "force")
            .unwrap_or_else(|| panic!("no `force` parameter in {:?}", parsed.parameters));
        assert_eq!(force.kind, ParamKind::ShortOnly);
        assert_eq!(force.short, Some('f'));
        assert_eq!(force.param_type, ParamType::Boolean);
        assert!(parsed.unparsed.is_empty());
    }

    /// The consumer's chosen name wins over the derived one.
    #[test]
    fn short_only_option_named_from_config() {
        let help = r#"Do something

Usage: tool cmd [OPTIONS]

Options:
  -n <COUNT>  How many times
  -h, --help  Print help
"#;
        let opts = ParseOptions {
            short_param_names: [(
                "cmd".to_string(),
                [('n', "repeat_count".to_string())].into_iter().collect(),
            )]
            .into_iter()
            .collect(),
            ..ParseOptions::default()
        };
        let parsed = parse_parameters_detailed(help, "cmd", &opts);
        assert!(parsed.errors.is_empty(), "got: {:?}", parsed.errors);
        let p = parsed
            .parameters
            .iter()
            .find(|p| p.long_name == "repeat_count")
            .unwrap_or_else(|| panic!("no `repeat_count` in {:?}", parsed.parameters));
        assert_eq!(p.short, Some('n'));
        assert_eq!(p.param_type, ParamType::String);
    }

    /// "How many times" yields `how` — not a name worth generating. Fail and
    /// say which config key names it, rather than guess.
    #[test]
    fn short_only_option_with_underivable_name_is_an_error() {
        let help = r#"Do something

Usage: tool cmd [OPTIONS]

Options:
  -n <COUNT>  How many times
  -h, --help  Print help
"#;
        let parsed = parse_parameters_detailed(help, "cmd", &ParseOptions::default());
        assert_eq!(parsed.errors.len(), 1, "got: {:?}", parsed.errors);
        assert!(
            parsed.errors[0].contains("-n") && parsed.errors[0].contains("short_param"),
            "error must name the flag and the config key, got: {}",
            parsed.errors[0]
        );
    }

    /// A description opening with a number derives a name that is not a legal
    /// shell identifier: the generated script would reference `${60}`, which is
    /// a positional-parameter read, not a variable — silently broken output of
    /// exactly the kind this parser exists to prevent.
    #[test]
    fn short_only_option_named_from_a_number_is_an_error() {
        let help = r#"Do something

Usage: tool cmd [OPTIONS]

Options:
  -t <SECONDS>  60 second timeout applied to the operation
  -h, --help    Print help
"#;
        let parsed = parse_parameters_detailed(help, "cmd", &ParseOptions::default());
        assert!(
            parsed.parameters.iter().all(|p| p.long_name != "60"),
            "a name starting with a digit must never reach the orb: {:?}",
            parsed.parameters
        );
        assert_eq!(parsed.errors.len(), 1, "got: {:?}", parsed.errors);
        assert!(parsed.errors[0].contains("-t") && parsed.errors[0].contains("short_param"));
    }

    /// The consumer's own name is held to the same rule — it ends up in a shell
    /// variable just the same.
    #[test]
    fn configured_short_only_name_must_be_a_usable_identifier() {
        let help = r#"Do something

Usage: tool cmd [OPTIONS]

Options:
  -t <SECONDS>  Timeout
  -h, --help    Print help
"#;
        let opts = ParseOptions {
            short_param_names: [(
                "cmd".to_string(),
                [('t', "60".to_string())].into_iter().collect(),
            )]
            .into_iter()
            .collect(),
            ..ParseOptions::default()
        };
        let parsed = parse_parameters_detailed(help, "cmd", &opts);
        assert_eq!(parsed.errors.len(), 1, "got: {:?}", parsed.errors);
        assert!(parsed.errors[0].contains("-t"));
    }

    /// A short-only option the Usage line lists outside `[...]` is required, and
    /// must generate a parameter CircleCI insists on — like every other required
    /// input here.
    #[test]
    fn required_short_only_option_is_required() {
        let help = r#"Do something

Usage: tool cmd [OPTIONS] -f <FILE>

Options:
  -f <FILE>   File to process
  -n <COUNT>  Repeat count
  -h, --help  Print help
"#;
        let opts = ParseOptions {
            short_param_names: [(
                "cmd".to_string(),
                [('f', "file".to_string()), ('n', "repeat_count".to_string())]
                    .into_iter()
                    .collect(),
            )]
            .into_iter()
            .collect(),
            ..ParseOptions::default()
        };
        let parsed = parse_parameters_detailed(help, "cmd", &opts);
        assert!(parsed.errors.is_empty(), "got: {:?}", parsed.errors);
        let file = parsed
            .parameters
            .iter()
            .find(|p| p.long_name == "file")
            .unwrap();
        assert!(
            file.required,
            "`-f <FILE>` outside [...] in the Usage line is required"
        );
        let count = parsed
            .parameters
            .iter()
            .find(|p| p.long_name == "repeat_count")
            .unwrap();
        assert!(!count.required, "`-n` only in [OPTIONS] is not required");
    }

    /// A description with no usable first word cannot name anything.
    #[test]
    fn short_only_option_without_description_is_an_error() {
        let help = r#"Do something

Usage: tool cmd [OPTIONS]

Options:
  -f
  -h, --help  Print help
"#;
        let parsed = parse_parameters_detailed(help, "cmd", &ParseOptions::default());
        assert_eq!(parsed.errors.len(), 1, "got: {:?}", parsed.errors);
        assert!(parsed.errors[0].contains("-f"));
    }

    /// Two parameters cannot share a name — the orb would silently keep one.
    #[test]
    fn name_collision_is_an_error() {
        let help = r#"Do something

Usage: tool cmd [OPTIONS]

Options:
  -f              Force the operation
      --force     Force it the long way
  -h, --help      Print help
"#;
        let parsed = parse_parameters_detailed(help, "cmd", &ParseOptions::default());
        assert_eq!(parsed.errors.len(), 1, "got: {:?}", parsed.errors);
        assert!(
            parsed.errors[0].contains("force"),
            "error must name the clashing parameter, got: {}",
            parsed.errors[0]
        );
    }

    /// A naming failure is not something `allow_unparsed_help` can wave through:
    /// the fix is to name the parameter, not to drop it.
    #[test]
    fn naming_error_is_not_silenced_by_allow_unparsed_help() {
        let help = r#"Do something

Usage: tool cmd [OPTIONS]

Options:
  -n <COUNT>  How many times
  -h, --help  Print help
"#;
        let opts = ParseOptions {
            allow_unparsed_help: true,
            ..ParseOptions::default()
        };
        let err = parse_subcommand("cmd", help, "tool", &opts)
            .expect_err("a naming failure must still fail generation");
        assert!(err.to_string().contains("short_param"));
    }
}
