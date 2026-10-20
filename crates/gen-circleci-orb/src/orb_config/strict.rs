//! Strict key checking for the rich-mode job-group tables (#502).
//!
//! serde drops a key it does not model, so a misspelt or unsupported key in a
//! `[[job_group.step]]` or `[[job_group.parameter]]` table is silently ignored
//! and the config looks as if it does something it does not. Those tables are
//! checked here, on the raw TOML, and every offending key is reported at once.

use super::{JobGroup, JobGroupParam, JobGroupStep};

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
        let toml = "[[job_group]]\nname = \"g\"\n[[job_group.step]]\ncommand = \"upload\"\nwhen = \"always\"\n";
        let errors = unknown_job_group_keys(toml);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("[[job_group.step]] #1"), "{errors:?}");
        assert!(errors[0].contains("job_group \"g\""), "{errors:?}");
        assert!(errors[0].contains("`when`"), "{errors:?}");
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
when = "always"
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
}
