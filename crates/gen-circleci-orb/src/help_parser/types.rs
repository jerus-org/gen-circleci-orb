/// Language-agnostic intermediate representation of a CLI tool's interface.
#[derive(Debug, Clone, PartialEq)]
pub struct CliDefinition {
    pub binary_name: String,
    pub description: String,
    pub subcommands: Vec<SubCommand>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubCommand {
    pub name: String,
    pub description: String,
    /// True when this subcommand has no children (leaf node in the command tree).
    pub is_leaf: bool,
    pub parameters: Vec<Parameter>,
    pub subcommands: Vec<SubCommand>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Parameter {
    /// Orb parameter name. For a long flag this is the normalised flag
    /// (`--orb-path` → `orb_path`); for a positional, the lowercased metavar
    /// (`<VERSION>` → `version`); for a short-only option, the name resolved
    /// from config or derived from the description.
    pub long_name: String,
    pub short: Option<char>,
    /// How the CLI accepts this input — decides how the generated script
    /// passes it (`--flag value`, `-f value`, or bare positional).
    pub kind: ParamKind,
    pub param_type: ParamType,
    pub default: Option<String>,
    /// True when the parameter has no default and is not boolean.
    pub required: bool,
    pub description: String,
}

/// How an input is written on the command line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ParamKind {
    /// `--force` (possibly with a short alias in [`Parameter::short`]).
    #[default]
    Long,
    /// `-f`, with no long form to name it after.
    ShortOnly,
    /// A bare value, position-dependent: `tool verify <VERSION>`.
    Positional,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum ParamType {
    #[default]
    String,
    Boolean,
    Integer,
    Enum(Vec<String>),
}

impl Parameter {
    /// Convert CLI long-flag name (e.g. `orb-path`) to orb parameter name (`orb_path`).
    pub fn normalize_name(flag: &str) -> String {
        flag.replace('-', "_")
    }
}
