//! Strict key checking for the rich-mode job-group tables (#502).
//!
//! serde drops a key it does not model, so a misspelt or unsupported key in a
//! `[[job_group.step]]` or `[[job_group.parameter]]` table is silently ignored
//! and the config looks as if it does something it does not. Those tables are
//! checked here, on the raw TOML, and every offending key is reported at once.

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
    for (index, group) in groups.iter().filter_map(|g| g.as_table()).enumerate() {
        let name = group
            .get("name")
            .and_then(|n| n.as_str())
            .map(|n| format!("job_group \"{n}\""))
            .unwrap_or_else(|| format!("job_group #{}", index + 1));
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
    for (index, entry) in entries.iter().filter_map(|e| e.as_table()).enumerate() {
        for key in entry.keys().filter(|k| !known.contains(&k.as_str())) {
            let mut message = format!(
                "[[job_group.{table}]] #{} in {group_name}: unknown key `{key}`",
                index + 1
            );
            if let Some(near) = nearest(key, known) {
                message.push_str(&format!(" (did you mean `{near}`?)"));
            }
            message.push_str(&format!(" — known keys: {}", known.join(", ")));
            errors.push(message);
        }
    }
}

/// The known key closest to `key`, if one is close enough to be a likely typo.
fn nearest<'a>(key: &str, known: &[&'a str]) -> Option<&'a str> {
    let limit = (key.chars().count() / 3).max(1);
    known
        .iter()
        .map(|k| (levenshtein(key, k), *k))
        .filter(|(distance, _)| *distance <= limit)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, k)| k)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (diagonal + usize::from(ca != *cb))
                .min(row[j] + 1)
                .min(above + 1);
            diagonal = above;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orb_config::{JobGroupParam, JobGroupStep};
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
}
