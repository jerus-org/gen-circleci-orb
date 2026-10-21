//! Strict key checking for the rich-mode job-group tables (#502).
//!
//! serde drops a key it does not model, so a misspelt or unsupported key in a
//! `[[job_group.step]]` or `[[job_group.parameter]]` table is silently ignored
//! and the config looks as if it does something it does not. Those tables are
//! checked here, on the raw TOML, and every offending key is reported at once.
//!
//! The other tables are checked too (#449), but more gently: a key the
//! generator no longer reads is *retired* and is reported with its replacement,
//! while a key it has never heard of is *unknown*. Neither stops `generate` or
//! `update` (an older config must keep working); `config validate` fails on both.

use super::{
    CiSection, ExtraJob, JobGroup, JobGroupParam, JobGroupStep, OrbConfig, OrbSection,
    ParamOverride, PostMergeCheckConfig, PostMergeRegenConfig, RecordConfig, SubcommandConfig,
};

/// The field names serde recognises for `T`, taken from its `Deserialize`
/// derive so they cannot drift from the struct (renames such as `type` and
/// `enum` included).
///
/// A derived `Deserialize` hands the full field list to
/// `Deserializer::deserialize_struct`; this deserializer stops there and
/// returns it.
fn known_keys<T: serde::de::DeserializeOwned>() -> &'static [&'static str] {
    match T::deserialize(FieldNames) {
        Err(Fields(fields)) => fields,
        Ok(_) => &[],
    }
}

#[derive(Debug)]
struct Fields(&'static [&'static str]);

impl std::fmt::Display for Fields {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "struct fields: {}", self.0.join(", "))
    }
}

impl std::error::Error for Fields {}

impl serde::de::Error for Fields {
    fn custom<T: std::fmt::Display>(_: T) -> Self {
        Fields(&[])
    }
}

struct FieldNames;

impl<'de> serde::Deserializer<'de> for FieldNames {
    type Error = Fields;

    fn deserialize_any<V: serde::de::Visitor<'de>>(self, _: V) -> Result<V::Value, Fields> {
        Err(Fields(&[]))
    }

    fn deserialize_struct<V: serde::de::Visitor<'de>>(
        self,
        _name: &'static str,
        fields: &'static [&'static str],
        _: V,
    ) -> Result<V::Value, Fields> {
        Err(Fields(fields))
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map enum identifier ignored_any
    }
}

/// A config key the generator once read and no longer does.
struct Retired {
    table: &'static str,
    key: &'static str,
    /// What to do instead, shown to the author.
    advice: &'static str,
}

/// Retired keys and what replaced them. Add a row when a field is dropped or
/// renamed, rather than only deleting it from the struct, so a consumer that
/// still carries it is told why it does nothing.
const RETIRED: &[Retired] = &[
    Retired {
        table: "ci",
        key: "rust_image",
        advice: "replaced by `[ci].build_executor` (an orb executor reference, not an image string)",
    },
    Retired {
        table: "ci",
        key: "live_regenerate",
        advice: "renamed to `[ci].test_generation`, which now gates the whole generation self-test chain",
    },
    Retired {
        table: "record",
        key: "write_token_env",
        advice: "no longer used: the record step relies on ambient authorization, so delete it",
    },
];

/// What a config file's keys say about it, beyond what serde reads.
#[derive(Debug, Default, PartialEq)]
pub struct KeyReport {
    /// Keys the generator no longer reads, with what replaced them.
    pub retired: Vec<String>,
    /// Keys it has never heard of, outside the job-group tables.
    pub unknown: Vec<String>,
    /// Unknown keys in the job-group tables (these are errors, see [`unknown_job_group_keys`]).
    pub job_group_unknown: Vec<String>,
    /// Why the file does not load as a config (bad syntax, or a value of the
    /// wrong type), when it does not. Set by `config validate`, so that it
    /// reports this together with the key problems rather than instead of them.
    pub parse_error: Option<String>,
}

impl KeyReport {
    /// Retired and unknown keys outside the job-group tables: reported as
    /// warnings by `generate` and `update`.
    pub fn warnings(&self) -> impl Iterator<Item = &String> {
        self.retired.iter().chain(&self.unknown)
    }

    /// Print the warnings to stderr, one per line.
    pub fn print_warnings(&self) {
        for warning in self.warnings() {
            eprintln!("warning: {warning}");
        }
    }

    pub fn is_clean(&self) -> bool {
        self.parse_error.is_none()
            && self.retired.is_empty()
            && self.unknown.is_empty()
            && self.job_group_unknown.is_empty()
    }
}

/// Check every table of `content` for retired and unknown keys.
///
/// Returns an empty report when `content` does not parse as TOML: the real
/// parse reports syntax errors.
pub fn config_key_report(content: &str) -> KeyReport {
    let Ok(doc) = content.parse::<toml::Table>() else {
        return KeyReport::default();
    };
    let mut report = KeyReport {
        job_group_unknown: unknown_job_group_keys(content),
        ..Default::default()
    };
    check_known(&doc, "", "", known_keys::<OrbConfig>(), &mut report);
    check_section::<OrbSection>(&doc, "orb", &mut report);
    check_section::<CiSection>(&doc, "ci", &mut report);
    check_section::<RecordConfig>(&doc, "record", &mut report);
    check_section::<PostMergeRegenConfig>(&doc, "post_merge_regen", &mut report);
    check_section::<PostMergeCheckConfig>(&doc, "post_merge_check", &mut report);
    check_array::<ExtraJob>(&doc, "extra_job", &mut report);

    if let Some(subcommands) = doc.get("subcommand").and_then(|v| v.as_table()) {
        for (name, entry) in subcommands
            .iter()
            .filter_map(|(n, v)| Some((n, v.as_table()?)))
        {
            let path = format!("subcommand.{name}");
            check_known(
                entry,
                &path,
                "subcommand",
                known_keys::<SubcommandConfig>(),
                &mut report,
            );
            if let Some(params) = entry.get("param").and_then(|v| v.as_table()) {
                for (flag, over) in params.iter().filter_map(|(n, v)| Some((n, v.as_table()?))) {
                    let path = format!("{path}.param.{flag}");
                    check_known(
                        over,
                        &path,
                        "subcommand.param",
                        known_keys::<ParamOverride>(),
                        &mut report,
                    );
                }
            }
        }
    }
    report
}

fn check_section<T: serde::de::DeserializeOwned>(
    doc: &toml::Table,
    table: &str,
    report: &mut KeyReport,
) {
    if let Some(section) = doc.get(table).and_then(|v| v.as_table()) {
        check_known(section, table, table, known_keys::<T>(), report);
    }
}

fn check_array<T: serde::de::DeserializeOwned>(
    doc: &toml::Table,
    table: &str,
    report: &mut KeyReport,
) {
    let Some(entries) = doc.get(table).and_then(|v| v.as_array()) else {
        return;
    };
    for (index, entry) in entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| e.as_table().map(|t| (i, t)))
    {
        let path = format!("{table}[{}]", index + 1);
        check_known(entry, &path, table, known_keys::<T>(), report);
    }
}

/// Sort each key of `table` that is not in `known` into retired or unknown.
/// `path` labels the table in messages; `scope` is the name its [`RETIRED`]
/// rows use (`ci`, `record`, `subcommand`, ...), independent of where it sits.
fn check_known(
    table: &toml::Table,
    path: &str,
    scope: &str,
    known: &[&str],
    report: &mut KeyReport,
) {
    let label = if path.is_empty() {
        "top level".to_string()
    } else {
        format!("[{path}]")
    };
    for key in table.keys().filter(|k| !known.contains(&k.as_str())) {
        if let Some(row) = RETIRED.iter().find(|r| r.table == scope && r.key == key) {
            report
                .retired
                .push(format!("{label}: `{key}` is retired — {}", row.advice));
        } else {
            report.unknown.push(unknown_key_message(&label, key, known));
        }
    }
}

/// Every unknown key in the job-group tables of `content`, one message each.
///
/// Returns an empty list when `content` does not parse as TOML or has no job
/// groups: the real parse reports syntax errors, and there is nothing to check.
pub fn unknown_job_group_keys(content: &str) -> Vec<String> {
    let Ok(doc) = content.parse::<toml::Table>() else {
        return Vec::new();
    };
    let Some(groups) = doc.get("job_group").and_then(|g| g.as_array()) else {
        return Vec::new();
    };

    let (group_keys, step_keys, param_keys) = (
        known_keys::<JobGroup>(),
        known_keys::<JobGroupStep>(),
        known_keys::<JobGroupParam>(),
    );
    let mut errors = Vec::new();
    for (index, group) in groups
        .iter()
        .enumerate()
        .filter_map(|(i, g)| g.as_table().map(|t| (i, t)))
    {
        let name = group
            .get("name")
            .and_then(|n| n.as_str())
            .map(|n| format!("job_group \"{n}\""))
            .unwrap_or_else(|| format!("job_group #{}", index + 1));
        for key in group.keys().filter(|k| !group_keys.contains(&k.as_str())) {
            errors.push(unknown_key_message(
                &format!("[[job_group]] #{} ({name})", index + 1),
                key,
                group_keys,
            ));
        }
        check_tables(group, "step", step_keys, &name, &mut errors);
        check_tables(group, "parameter", param_keys, &name, &mut errors);
    }
    errors
}

fn check_tables(
    group: &toml::Table,
    table: &str,
    known: &[&str],
    group_name: &str,
    errors: &mut Vec<String>,
) {
    let Some(entries) = group.get(table).and_then(|v| v.as_array()) else {
        return;
    };
    for (index, entry) in entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| e.as_table().map(|t| (i, t)))
    {
        for key in entry.keys().filter(|k| !known.contains(&k.as_str())) {
            let location = format!("[[job_group.{table}]] #{} in {group_name}", index + 1);
            errors.push(unknown_key_message(&location, key, known));
        }
    }
}

fn unknown_key_message(location: &str, key: &str, known: &[&str]) -> String {
    let mut message = format!("{location}: unknown key `{key}`");
    if let Some(near) = nearest(key, known) {
        message.push_str(&format!(" (did you mean `{near}`?)"));
    }
    message.push_str(&format!(" — known keys: {}", known.join(", ")));
    message
}

/// The known key closest to `key`, if one is close enough to be a likely typo.
///
/// Keys under three characters are never matched: at that length one edit
/// reaches almost any known key, so a suggestion would only mislead.
fn nearest<'a>(key: &str, known: &[&'a str]) -> Option<&'a str> {
    let length = key.chars().count();
    if length < 3 {
        return None;
    }
    let limit = (length / 3).max(1);
    known
        .iter()
        .map(|k| (edit_distance(key, k), *k))
        .filter(|(distance, _)| *distance <= limit)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, k)| k)
}

/// Optimal string alignment distance: insertions, deletions, substitutions and
/// adjacent transpositions (`wiht` -> `with`) each cost one edit.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut grid = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in grid.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in grid[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (grid[i - 1][j] + 1)
                .min(grid[i][j - 1] + 1)
                .min(grid[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(grid[i - 2][j - 2] + 1);
            }
            grid[i][j] = best;
        }
    }
    grid[a.len()][b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(keys: &[&str]) -> Vec<String> {
        let mut keys: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
        keys.sort();
        keys
    }

    #[test]
    fn known_keys_come_from_the_structs() {
        assert_eq!(
            sorted(known_keys::<JobGroupStep>()),
            sorted(&[
                "builtin",
                "command",
                "orb",
                "run",
                "script",
                "when",
                "with",
                "environment",
                "requires_git_auth",
            ])
        );
        // The serde names, not the Rust field names (`param_type`, `enum_values`).
        assert_eq!(
            sorted(known_keys::<JobGroupParam>()),
            sorted(&["name", "type", "default", "description", "enum"])
        );
        assert_eq!(
            sorted(known_keys::<JobGroup>()),
            sorted(&[
                "name",
                "description",
                "steps",
                "params",
                "executor",
                "parameter",
                "step",
            ])
        );
    }

    #[test]
    fn valid_job_group_has_no_errors() {
        let toml = r#"
[[job_group]]
name = "g"
[[job_group.parameter]]
name = "p"
type = "enum"
enum = ["a"]
[[job_group.step]]
command = "x"
requires_git_auth = true
[job_group.step.with]
anything = "goes"
"#;
        assert!(unknown_job_group_keys(toml).is_empty());
    }

    #[test]
    fn unsupported_step_key_is_reported_with_table_and_key() {
        let toml = "[[job_group]]\nname = \"g\"\n[[job_group.step]]\ncommand = \"upload\"\nretry = \"always\"\n";
        let errors = unknown_job_group_keys(toml);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("[[job_group.step]] #1"), "{errors:?}");
        assert!(errors[0].contains("job_group \"g\""), "{errors:?}");
        assert!(errors[0].contains("`retry`"), "{errors:?}");
        assert!(!errors[0].contains("did you mean"), "{errors:?}");
    }

    #[test]
    fn misspelt_key_suggests_nearest() {
        let toml = "[[job_group]]\nname = \"g\"\n[[job_group.step]]\nrun = \"r\"\nrequire_git_auth = true\n";
        let errors = unknown_job_group_keys(toml);
        assert!(
            errors[0].contains("did you mean `requires_git_auth`?"),
            "{errors:?}"
        );
    }

    #[test]
    fn parameter_table_is_checked() {
        let toml =
            "[[job_group]]\nname = \"g\"\n[[job_group.parameter]]\nname = \"p\"\ndefalt = \"x\"\n";
        let errors = unknown_job_group_keys(toml);
        assert!(errors[0].contains("[[job_group.parameter]]"), "{errors:?}");
        assert!(errors[0].contains("did you mean `default`?"), "{errors:?}");
    }

    #[test]
    fn all_problems_across_groups_are_collected() {
        let toml = r#"
[[job_group]]
name = "a"
[[job_group.step]]
run = "r"
retry = "always"
[[job_group.step]]
run = "s"
bogus = 1
[[job_group]]
name = "b"
[[job_group.parameter]]
name = "p"
extra = 1
"#;
        assert_eq!(unknown_job_group_keys(toml).len(), 3);
    }

    #[test]
    fn invalid_toml_or_no_groups_yield_nothing() {
        assert!(unknown_job_group_keys("not = [valid").is_empty());
        assert!(unknown_job_group_keys("[orb]\nbinary = \"x\"\n").is_empty());
    }

    #[test]
    fn job_group_level_unknown_key_is_reported() {
        let toml = "[[job_group]]\nname = \"g\"\nparameters = []\nexecuter = \"x\"\n";
        let errors = unknown_job_group_keys(toml);
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(
            errors[0].contains("[[job_group]] #1 (job_group \"g\")"),
            "{errors:?}"
        );
        assert!(errors[0].contains("did you mean `executor`?"), "{errors:?}");
        // Keys come out in the TOML table's (alphabetical) order.
        assert!(
            errors[1].contains("did you mean `parameter`?"),
            "{errors:?}"
        );
    }

    #[test]
    fn position_counts_non_table_entries() {
        let toml = "[[job_group]]\nname = \"g\"\nstep = [\"stray\", { run = \"r\", bogus = 1 }]\n";
        let errors = unknown_job_group_keys(toml);
        assert!(errors[0].contains("[[job_group.step]] #2"), "{errors:?}");
    }

    #[test]
    fn suggestion_handles_transpositions_and_ignores_short_keys() {
        assert_eq!(nearest("wiht", known_keys::<JobGroupStep>()), Some("with"));
        assert_eq!(nearest("rn", known_keys::<JobGroupStep>()), None);
        assert_eq!(
            nearest("completely_different", known_keys::<JobGroupStep>()),
            None
        );
    }

    const STALE: &str = r#"
[ci]
build_workflow = "validation"
rust_image = "rust:latest"
live_regenerate = true
build_executer = "toolkit/rust_env_rolling"

[record]
enabled = true
write_token_env = "TOKEN"

[subcommand.build]
generate_job = true
labell = "x"

[subcommand.build.param.flag]
default = "1"
defalt = "2"

[[extra_job]]
name = "e"
yaml = "y"
bogus = 1

[[job_group]]
name = "g"
[[job_group.step]]
run = "r"
retry = "always"
"#;

    #[test]
    fn retired_keys_name_their_replacement() {
        let report = config_key_report(STALE);
        assert_eq!(report.retired.len(), 3, "{:?}", report.retired);
        let all = report.retired.join("\n");
        assert!(all.contains("[ci]: `rust_image` is retired"), "{all}");
        assert!(all.contains("`[ci].build_executor`"), "{all}");
        assert!(all.contains("`live_regenerate`"), "{all}");
        assert!(all.contains("`[ci].test_generation`"), "{all}");
        assert!(all.contains("[record]: `write_token_env`"), "{all}");
    }

    #[test]
    fn unknown_keys_are_found_in_every_table_with_suggestions() {
        let report = config_key_report(STALE);
        let all = report.unknown.join("\n");
        assert_eq!(report.unknown.len(), 4, "{all}");
        assert!(
            all.contains("[ci]: unknown key `build_executer` (did you mean `build_executor`?)"),
            "{all}"
        );
        assert!(
            all.contains("[subcommand.build]: unknown key `labell` (did you mean `label`?)"),
            "{all}"
        );
        assert!(
            all.contains(
                "[subcommand.build.param.flag]: unknown key `defalt` (did you mean `default`?)"
            ),
            "{all}"
        );
        assert!(all.contains("[extra_job[1]]: unknown key `bogus`"), "{all}");
        assert_eq!(report.job_group_unknown.len(), 1);
    }

    #[test]
    fn a_retired_key_is_not_also_reported_as_unknown() {
        let report = config_key_report(STALE);
        assert!(!report.unknown.iter().any(|u| u.contains("rust_image")));
    }

    #[test]
    fn a_current_config_is_clean() {
        let toml = r#"
[orb]
binary = "x"
[ci]
build_executor = "e"
test_generation = false
[record]
enabled = true
[subcommand.build.param.flag]
default = "1"
[orbs]
anything = "goes@1"
"#;
        assert!(config_key_report(toml).is_clean());
    }

    #[test]
    fn warnings_cover_retired_and_unknown_but_not_job_group_errors() {
        let report = config_key_report(STALE);
        assert_eq!(report.warnings().count(), 7);
    }

    #[test]
    fn invalid_toml_yields_an_empty_report() {
        assert!(config_key_report("not = [valid").is_clean());
    }

    #[test]
    fn retired_keys_match_their_table_not_a_path_prefix() {
        // A subcommand that happens to be called `ci` is not the `[ci]` table.
        let report = config_key_report("[subcommand.ci]\nrust_image = \"x\"\n");
        assert!(report.retired.is_empty(), "{:?}", report.retired);
        assert_eq!(report.unknown.len(), 1);
    }
}
