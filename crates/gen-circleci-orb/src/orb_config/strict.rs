//! Strict key checking for the rich-mode job-group tables (#502).
//!
//! serde drops a key it does not model, so a misspelt or unsupported key in a
//! `[[job_group.step]]` or `[[job_group.parameter]]` table is silently ignored
//! and the config looks as if it does something it does not. Those tables are
//! checked here, on the raw TOML, and every offending key is reported at once.

/// Keys `[[job_group]]` itself understands. Kept in step with [`JobGroup`] by
/// the `known_keys_match_struct_fields` test.
const GROUP_KEYS: &[&str] = &[
    "name",
    "description",
    "steps",
    "params",
    "executor",
    "parameter",
    "step",
];

/// Keys `[[job_group.step]]` understands. Kept in step with [`JobGroupStep`] by
/// the `known_keys_match_struct_fields` test.
const STEP_KEYS: &[&str] = &[
    "builtin",
    "command",
    "orb",
    "run",
    "script",
    "with",
    "environment",
    "requires_git_auth",
];

/// Keys `[[job_group.parameter]]` understands. Kept in step with
/// [`JobGroupParam`] by the `known_keys_match_struct_fields` test.
const PARAM_KEYS: &[&str] = &["name", "type", "default", "description", "enum"];

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
        for key in group.keys().filter(|k| !GROUP_KEYS.contains(&k.as_str())) {
            errors.push(unknown_key_message(
                &format!("[[job_group]] #{} ({name})", index + 1),
                key,
                GROUP_KEYS,
            ));
        }
        check_tables(group, "step", STEP_KEYS, &name, &mut errors);
        check_tables(group, "parameter", PARAM_KEYS, &name, &mut errors);
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
    use crate::orb_config::{JobGroup, JobGroupParam, JobGroupStep};
    use indexmap::IndexMap;

    fn keys_of<T: serde::Serialize>(value: &T) -> Vec<String> {
        let table = toml::Table::try_from(value).unwrap();
        let mut keys: Vec<String> = table.keys().cloned().collect();
        keys.sort();
        keys
    }

    fn sorted(known: &[&str]) -> Vec<String> {
        let mut keys: Vec<String> = known.iter().map(|k| k.to_string()).collect();
        keys.sort();
        keys
    }

    #[test]
    fn known_keys_match_struct_fields() {
        let map: IndexMap<String, String> = IndexMap::from([("k".to_string(), "v".to_string())]);
        let step = JobGroupStep {
            builtin: Some(String::new()),
            command: Some(String::new()),
            orb: Some(String::new()),
            run: Some(String::new()),
            script: Some(String::new()),
            with: Some(map.clone()),
            environment: Some(map),
            requires_git_auth: Some(true),
        };
        assert_eq!(keys_of(&step), sorted(STEP_KEYS));

        let param = JobGroupParam {
            name: String::new(),
            param_type: Some(String::new()),
            default: Some(String::new()),
            description: Some(String::new()),
            enum_values: Some(vec![]),
        };
        assert_eq!(keys_of(&param), sorted(PARAM_KEYS));

        let group = JobGroup {
            name: String::new(),
            description: Some(String::new()),
            steps: vec![String::new()],
            params: Some(vec![]),
            executor: Some(String::new()),
            parameter: Some(vec![]),
            step: Some(vec![]),
        };
        assert_eq!(keys_of(&group), sorted(GROUP_KEYS));
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
        assert_eq!(nearest("wiht", STEP_KEYS), Some("with"));
        assert_eq!(nearest("rn", STEP_KEYS), None);
        assert_eq!(nearest("completely_different", STEP_KEYS), None);
    }
}
