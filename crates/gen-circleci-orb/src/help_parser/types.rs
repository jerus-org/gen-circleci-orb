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

#[derive(Debug, Clone, PartialEq)]
pub struct Parameter {
    /// Normalised parameter name: CLI `--orb-path` → `orb_path`.
    pub long_name: String,
    pub short: Option<char>,
    pub param_type: ParamType,
    pub default: Option<String>,
    /// True when the parameter has no default and is not boolean.
    pub required: bool,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParamType {
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
