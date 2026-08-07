mod types;

pub use types::{
    CiSection, ExtraJob, JobGroup, JobGroupParam, JobGroupStep, OrbConfig, OrbSection,
    ParamOverride, RecordConfig, SubcommandConfig, DEFAULT_BASE_IMAGE, DEFAULT_BUILDER_IMAGE,
    DEFAULT_CRATE_WAIT_ATTEMPTS, DEFAULT_CRATE_WAIT_SECONDS, DEFAULT_INSTALL_METHOD,
    DEFAULT_ORB_DIR, MAX_CRATE_WAIT_ATTEMPTS, MCP_DEFAULT_BASE_IMAGE,
};

use anyhow::{Context, Result};
use std::path::Path;

/// Treat an empty string as absent.
///
/// A config key present but blank (`binary = ""`) is a value the user has not
/// supplied, not a value they chose. Resolution paths that check only for
/// absence otherwise accept it and carry `""` into whatever they configure —
/// so every such path runs its candidates through this.
pub fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn load_config(path: &Path) -> Result<OrbConfig> {
    if !path.exists() {
        return Ok(OrbConfig::default());
    }
    let content =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let config: OrbConfig = toml::from_str(&content).with_context(|| {
        format!(
            "{} is not valid TOML — fix the syntax, or delete the file to start over",
            path.display()
        )
    })?;
    Ok(config)
}

/// Write the config, keeping whatever the user wrote around it.
///
/// The file is hand-annotated and reviewed — this repo's own records why each
/// image digest is pinned and which Renovate manager keeps it current — so a
/// write that serialises the struct over the top loses that silently, in a diff
/// that otherwise looks like a one-line change (#248). When the file already
/// exists, the new values are merged into the parsed document instead, leaving
/// comments, blank lines and key order intact.
pub fn save_config(path: &Path, config: &OrbConfig) -> Result<()> {
    let rendered = toml::to_string_pretty(config)?;
    let content = match std::fs::read_to_string(path) {
        Ok(existing) => merge_into_document(&existing, &rendered)?,
        // Only "no file yet" means there is nothing to preserve. Any other read
        // failure — a non-UTF-8 byte in a comment, a permissions problem — must
        // abort rather than overwrite the file this exists to protect.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => rendered,
        Err(e) => {
            return Err(e).with_context(|| {
                format!(
                    "cannot read {} to preserve its comments; refusing to overwrite it",
                    path.display()
                )
            });
        }
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_atomically(path, &content)
}

/// Where the new contents are staged before replacing `path`.
///
/// A sibling, deliberately: `rename` is only atomic within one filesystem, so a
/// temporary elsewhere — `/tmp`, say — could be on another and turn the rename
/// back into a copy.
///
/// Named per process so two saves running at once stage separately. Sharing one
/// name would let the second overwrite the first's staged content before the
/// first renames, and the first would then report success having published the
/// second's config.
fn write_temp_path(path: &Path) -> std::path::PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.tmp", std::process::id()));
    path.with_file_name(name)
}

/// Write `content` to `path` by staging it beside the target and renaming over.
///
/// This file holds comments that exist nowhere else — why each image digest is
/// pinned, which Renovate manager maintains it — and every save rewrites the
/// only copy. Writing in place would mean an interruption between truncate and
/// flush leaves a partial file with nothing to recover from. `rename` within a
/// filesystem is atomic, so a reader sees either the old contents or the new,
/// never a half-written mix.
/// Because the rename installs a *new* inode where an in-place write reused the
/// existing one, two properties have to be carried over deliberately: the
/// target's permissions, and the fact that it may be a symlink.
fn write_atomically(path: &Path, content: &str) -> Result<()> {
    use std::io::Write;

    // Write *through* a symlink, not over it. Renaming onto the link's own path
    // would replace it with a regular file and leave the file it pointed at
    // holding the previous config — a config the user is still editing.
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let temp = write_temp_path(&target);

    let staged = || -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(content.as_bytes())?;
        // Get the bytes to stable storage before the rename commits. Without
        // this the rename's metadata can reach disk first, so a machine that
        // dies moments later comes back to a present, empty config — the exact
        // loss this exists to prevent, with the old contents already unlinked.
        file.sync_all()
    }();
    staged.with_context(|| format!("cannot stage the new contents at {}", temp.display()))?;

    // A config may have been chmodded deliberately: it can name CircleCI
    // contexts and pinned images. The staged file is created at the umask
    // default, so without this a save silently widens the permissions.
    if let Ok(existing) = std::fs::metadata(&target) {
        let _ = std::fs::set_permissions(&temp, existing.permissions());
    }

    if let Err(e) = std::fs::rename(&temp, &target) {
        // Leave no litter beside the config when the replace itself fails.
        let _ = std::fs::remove_file(&temp);
        return Err(e).with_context(|| format!("cannot replace {}", target.display()));
    }

    // The rename itself is durable only once the directory entry is. Best
    // effort: not every platform lets a directory be opened this way.
    if let Some(parent) = target.parent() {
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

/// Apply `rendered` onto `existing`, preserving the latter's formatting.
fn merge_into_document(existing: &str, rendered: &str) -> Result<String> {
    // Every caller loads the config before saving it, so this text has already
    // parsed once. Reaching here means the file changed on disk in between —
    // report that rather than repeating the advice `load_config` already gave.
    let mut doc: toml_edit::DocumentMut = existing.parse().with_context(|| {
        "gen-circleci-orb.toml changed on disk and no longer parses; \
         its comments cannot be preserved"
    })?;
    let incoming: toml_edit::DocumentMut = rendered
        .parse()
        .context("failed to re-parse the serialised config")?;

    // Keys this binary does not model are dropped by serde on load, so they are
    // absent from `incoming` and would look deliberately removed. Round-tripping
    // the existing file through the same serde pass shows which keys this binary
    // *does* understand — anything outside that set is left alone rather than
    // swept away by an older generator writing a newer config.
    // Falling back to an empty document here would make every removal a no-op,
    // so a key the caller cleared would silently persist while the command
    // reported success. Fail as loudly as the parse above.
    let known: OrbConfig = toml::from_str(existing)
        .context("gen-circleci-orb.toml changed on disk and no longer reads as a config")?;
    let known: toml_edit::DocumentMut = toml::to_string_pretty(&known)?
        .parse()
        .context("cannot determine which keys this version manages; refusing to write")?;

    merge_table(doc.as_table_mut(), incoming.as_table(), known.as_table());
    Ok(doc.to_string())
}

/// Recursively copy `source` into `target`.
///
/// Keys present in both are updated in place, which is what keeps a comment
/// attached to its key: the decor lives on the target's key entry, not on the
/// value being replaced. Keys only in `source` are appended. A key is removed
/// only when `known` says this binary understands it and `source` no longer has
/// it — an unrecognised key is left as the user wrote it.
fn merge_table(target: &mut toml_edit::Table, source: &toml_edit::Table, known: &toml_edit::Table) {
    // Keys whose decor must be cleared once the borrow on their item is released.
    let mut reset_key_decor: Vec<String> = Vec::new();
    for (key, source_item) in source.iter() {
        merge_entry(target, key, source_item, known, &mut reset_key_decor);
    }
    for key in reset_key_decor {
        move_key_decor_to_section(target, &key);
    }
    remove_stale_keys(target, source, known);
}

/// Merge one key of `source` into `target`.
///
/// Records in `reset_key_decor` any key whose decor must be cleared afterwards,
/// which cannot be done here while the item is mutably borrowed.
fn merge_entry(
    target: &mut toml_edit::Table,
    key: &str,
    source_item: &toml_edit::Item,
    known: &toml_edit::Table,
    reset_key_decor: &mut Vec<String>,
) {
    match (target.get_mut(key), source_item) {
        // Both tables: recurse so nested comments survive too.
        (Some(toml_edit::Item::Table(target_tbl)), toml_edit::Item::Table(source_tbl)) => {
            // No counterpart in `known` means this binary models nothing here,
            // so nothing below is removed.
            let empty = toml_edit::Table::new();
            let known_child = known.get(key).and_then(|i| i.as_table()).unwrap_or(&empty);
            merge_table(target_tbl, source_tbl, known_child);
        }
        // Arrays of tables (`[[job_group]]`): merge element by element so an
        // entry's comment survives a change to a sibling — or to itself.
        (
            Some(toml_edit::Item::ArrayOfTables(target_arr)),
            toml_edit::Item::ArrayOfTables(source_arr),
        ) => {
            let empty = toml_edit::ArrayOfTables::new();
            let known_arr = known
                .get(key)
                .and_then(|i| i.as_array_of_tables())
                .unwrap_or(&empty);
            merge_array_of_tables(target_arr, source_arr, known_arr);
        }
        // Present but not a matching pair: replace only if the value actually
        // changed. An untouched value keeps the formatting the user gave it —
        // otherwise every write reflows their inline arrays, which is the same
        // "don't rewrite what I wrote" complaint in smaller form.
        (Some(target_item), _) if !same_value(target_item, source_item) => {
            // An inline `x = { … }` becoming a `[x]` section carries its old key
            // decor into the header, rendering `[x ]`. Note it and clear that
            // decor once the borrow is released.
            if target_item.is_value() && !source_item.is_value() {
                reset_key_decor.push(key.to_string());
            }
            replace_preserving_decor(target_item, source_item);
        }
        // Unchanged value: leave it exactly as the user wrote it.
        (Some(_), _) => {}
        // New key: append it.
        (None, _) => {
            target.insert(key, source_item.clone());
        }
    }
}

/// Move a key's decor onto the section that replaced it.
///
/// An inline `x = { … }` rendered as `[x]` leaves two artefacts: the space that
/// sat before `=` shows up inside the header, and the comment above the key
/// stays attached to the *previous* section, where it reads as a trailing note
/// on something else. The suffix is dropped and the prefix moves to the section.
fn move_key_decor_to_section(target: &mut toml_edit::Table, key: &str) {
    let prefix = target
        .key(key)
        .and_then(|k| k.leaf_decor().prefix())
        .and_then(|p| p.as_str())
        .map(str::to_string);
    if let Some(mut k) = target.key_mut(key) {
        k.leaf_decor_mut().set_suffix("");
        k.dotted_decor_mut().set_suffix("");
        if prefix.is_some() {
            k.leaf_decor_mut().set_prefix("");
        }
    }
    let Some(prefix) = prefix.filter(|p| !p.trim().is_empty()) else {
        return;
    };
    if let Some(toml_edit::Item::Table(table)) = target.get_mut(key) {
        let existing = table
            .decor()
            .prefix()
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .to_string();
        // Blank line first (from the section's own decor), then the comment,
        // then the header — so the comment sits directly above what it heads.
        table
            .decor_mut()
            .set_prefix(format!("{existing}{}", prefix.trim_start_matches('\n')));
    }
}

/// Drop keys this binary models that `source` no longer has — an unrecognised
/// key is left alone, since serde silently discards what it does not know.
fn remove_stale_keys(
    target: &mut toml_edit::Table,
    source: &toml_edit::Table,
    known: &toml_edit::Table,
) {
    let stale: Vec<String> = target
        .iter()
        .map(|(k, _)| k.to_string())
        .filter(|k| source.get(k).is_none() && known.get(k).is_some())
        .collect();
    for key in stale {
        target.remove(&key);
    }
}

/// Merge `[[section]]` entries, matching them by `name`.
///
/// Every array-of-tables in this config (`[[job_group]]`, `[[extra_job]]`) keys
/// its entries by `name`, so matching on it keeps an entry's comment with that
/// entry when one is removed or the order changes. Matching by position instead
/// would reattach the comments to whatever slid into the slot.
///
/// `known` is the matching array from the round-tripped document. Passing
/// `source` here would make `remove_stale_keys`'s filter a contradiction —
/// "absent from source AND present in source" — so a key cleared inside an
/// entry would never actually be removed.
fn merge_array_of_tables(
    target: &mut toml_edit::ArrayOfTables,
    source: &toml_edit::ArrayOfTables,
    known: &toml_edit::ArrayOfTables,
) {
    let name_of = |t: &toml_edit::Table| t.get("name").and_then(|i| i.as_str()).map(str::to_string);

    // Take the existing entries out so each can be matched to a source entry by
    // name and put back in the source's order, carrying its formatting with it.
    let existing: Vec<toml_edit::Table> = std::mem::take(target).into_iter().collect();
    let mut by_name: Vec<(Option<String>, toml_edit::Table)> =
        existing.into_iter().map(|t| (name_of(&t), t)).collect();

    let empty = toml_edit::Table::new();
    for (i, source_tbl) in source.iter().enumerate() {
        let wanted = name_of(source_tbl);
        // Match by name where both sides have one; fall back to position for
        // entries that do not (nothing in this schema, but the merge should not
        // depend on that).
        let found = wanted
            .as_ref()
            .and_then(|w| by_name.iter().position(|(n, _)| n.as_ref() == Some(w)))
            .or_else(|| (wanted.is_none() && i < by_name.len()).then_some(i));

        match found {
            Some(pos) => {
                let (_, mut existing_tbl) = by_name.remove(pos);
                let known_tbl = known
                    .iter()
                    .find(|k| name_of(k) == wanted)
                    .unwrap_or(&empty);
                merge_table(&mut existing_tbl, source_tbl, known_tbl);
                target.push(existing_tbl);
            }
            None => target.push(source_tbl.clone()),
        }
    }
    // Whatever is left in `by_name` had no counterpart in `source`: the caller
    // removed those entries, and they leave with their own comments.
    drop(by_name);
}

/// Overwrite `target` with `source`, keeping the decor (a trailing comment, the
/// surrounding spacing) that belonged to the value being replaced.
fn replace_preserving_decor(target: &mut toml_edit::Item, source: &toml_edit::Item) {
    let decor = target.as_value().map(|v| v.decor().clone());
    *target = source.clone();
    let Some(decor) = decor else { return };
    match target {
        toml_edit::Item::Value(value) => *value.decor_mut() = decor,
        // An inline `x = { … }` rendered as a `[x]` section: the value's
        // trailing comment belongs after the header, not lost.
        toml_edit::Item::Table(table) => {
            if let Some(suffix) = decor.suffix().and_then(|s| s.as_str()) {
                table.decor_mut().set_suffix(suffix.to_string());
            }
        }
        _ => {}
    }
}

/// True when two items hold the same value, ignoring how it is written.
///
/// `["a", "b"]` and the same array spread over lines are equal here, so an
/// untouched value is left exactly as the user formatted it.
fn same_value(a: &toml_edit::Item, b: &toml_edit::Item) -> bool {
    fn semantic(item: &toml_edit::Item) -> Option<toml::Value> {
        // A standard table has no standalone form, so convert it to the inline
        // one first: otherwise an inline `x = { … }` in the file never compares
        // equal to the section the serialiser emits, and is rewritten on every
        // save even when nothing changed.
        let as_value = item.clone().into_value().ok()?;
        // Wrap in a key so a bare value parses as TOML.
        let wrapped = format!("x = {}", as_value.to_string().trim());
        wrapped.parse::<toml::Table>().ok()?.get("x").cloned()
    }
    match (semantic(a), semantic(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;
    use tempfile::TempDir;

    fn write_toml(dir: &TempDir, content: &str) -> std::path::PathBuf {
        let path = dir.path().join("gen-circleci-orb.toml");
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn load_config_parses_minimal_record_opt_out() {
        // The documented silence opt-out (#155) — a [record] section with only
        // `enabled = false` — must parse: the env-name fields default rather than
        // erroring on "missing field".
        let dir = TempDir::new().unwrap();
        let path = write_toml(
            &dir,
            r#"
[record]
enabled = false
"#,
        );
        let config = load_config(&path).unwrap();
        let record = config.record.expect("[record] must parse");
        assert!(!record.enabled);
        assert!(record.gpg_key_env.is_empty());
    }

    #[test]
    fn load_config_parses_git_push_subcommands() {
        let dir = TempDir::new().unwrap();
        let path = write_toml(
            &dir,
            r#"
[orb]
binary = "mytool"
git_push_subcommands = ["save"]
"#,
        );
        let config = load_config(&path).unwrap();
        let orb = config.orb.as_ref().unwrap();
        assert_eq!(
            orb.git_push_subcommands.as_deref(),
            Some(&["save".to_string()][..])
        );
    }

    #[test]
    fn load_config_parses_ci_section() {
        let dir = TempDir::new().unwrap();
        let path = write_toml(
            &dir,
            r#"
[ci]
build_workflow = "validation"
release_workflow = "orb-release"
requires_job = "toolkit/common_tests"
release_after_job = "publish-orb-jerus-org"
crate_tag_prefix = "mytool-v"
docker_namespace = "myns"
docker_context = "docker"
orb_context = "orb-publishing"
mcp = true
mcp_context = ["release", "bot-check", "pcu-app"]
mcp_earliest_version = "0.1.0"
"#,
        );
        let config = load_config(&path).unwrap();
        let ci = config.ci.as_ref().expect("ci section missing");
        assert_eq!(ci.build_workflow.as_deref(), Some("validation"));
        assert_eq!(ci.docker_context.as_deref(), Some("docker"));
        assert_eq!(ci.mcp, Some(true));
        assert_eq!(ci.mcp_earliest_version.as_deref(), Some("0.1.0"));
        assert_eq!(
            ci.mcp_context.as_deref(),
            Some(
                &[
                    "release".to_string(),
                    "bot-check".to_string(),
                    "pcu-app".to_string()
                ][..]
            )
        );
    }

    #[test]
    fn load_config_returns_default_when_file_not_found() {
        let path = std::path::Path::new("/tmp/does-not-exist/gen-circleci-orb.toml");
        let config = load_config(path).unwrap();
        assert_eq!(config, OrbConfig::default());
    }

    #[test]
    fn load_config_parses_suppression_entry() {
        let dir = TempDir::new().unwrap();
        let path = write_toml(
            &dir,
            r#"
[subcommand.help]
generate_job = false
"#,
        );
        let config = load_config(&path).unwrap();
        let subcommands = config.subcommand.unwrap();
        let help = subcommands.get("help").unwrap();
        assert_eq!(help.generate_job, Some(false));
    }

    #[test]
    fn load_config_parses_param_override() {
        let dir = TempDir::new().unwrap();
        let path = write_toml(
            &dir,
            r#"
[subcommand.generate.param.orb_path]
default = "src/@orb.yml"
"#,
        );
        let config = load_config(&path).unwrap();
        let subcommands = config.subcommand.unwrap();
        let gen = subcommands.get("generate").unwrap();
        let params = gen.param.as_ref().unwrap();
        let override_ = params.get("orb_path").unwrap();
        assert_eq!(override_.default.as_deref(), Some("src/@orb.yml"));
    }

    #[test]
    fn load_config_parses_job_group_with_steps() {
        let dir = TempDir::new().unwrap();
        let path = write_toml(
            &dir,
            r#"
[[job_group]]
name = "sync"
description = "Regenerate and validate"
steps = ["generate", "validate"]
"#,
        );
        let config = load_config(&path).unwrap();
        let groups = config.job_group.unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "sync");
        assert_eq!(groups[0].steps, vec!["generate", "validate"]);
        assert_eq!(
            groups[0].description.as_deref(),
            Some("Regenerate and validate")
        );
        assert!(groups[0].params.is_none());
    }

    #[test]
    fn load_config_parses_job_group_with_explicit_params() {
        let dir = TempDir::new().unwrap();
        let path = write_toml(
            &dir,
            r#"
[[job_group]]
name = "build"
steps = ["generate", "validate"]
params = ["orb_path", "binary"]
"#,
        );
        let config = load_config(&path).unwrap();
        let groups = config.job_group.unwrap();
        let expected: Vec<String> = vec!["orb_path".to_string(), "binary".to_string()];
        assert_eq!(groups[0].params.as_deref(), Some(expected.as_slice()));
    }

    #[test]
    fn load_config_parses_rich_job_group_with_parameters_and_steps() {
        let dir = TempDir::new().unwrap();
        let path = write_toml(
            &dir,
            r#"
[[job_group]]
name = "sync_and_publish"
description = "Prime, generate, publish and commit back."

[[job_group.parameter]]
name = "binary_name"
description = "Consumer binary name."

[[job_group.parameter]]
name = "tag_prefix"
type = "string"
default = "v"

[[job_group.step]]
builtin = "checkout"

[[job_group.step]]
command = "set_https_remote"

[[job_group.step]]
run = "Set up git and environment"
script = "git fetch origin main"
[job_group.step.environment]
TAG_PREFIX = "<< parameters.tag_prefix >>"

[[job_group.step]]
command = "generate"
[job_group.step.with]
format = "binary"
orb_path = "<< parameters.orb_path >>"

[[job_group.step]]
orb = "toolkit/setup"
"#,
        );
        let config = load_config(&path).unwrap();
        let groups = config.job_group.unwrap();
        assert_eq!(groups.len(), 1);
        let g = &groups[0];
        assert_eq!(g.name, "sync_and_publish");

        let params = g.parameter.as_ref().expect("parameters missing");
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "binary_name");
        assert_eq!(params[1].param_type.as_deref(), Some("string"));
        assert_eq!(params[1].default.as_deref(), Some("v"));

        let steps = g.step.as_ref().expect("steps missing");
        assert_eq!(steps.len(), 5);
        assert_eq!(steps[0].builtin.as_deref(), Some("checkout"));
        assert_eq!(steps[1].command.as_deref(), Some("set_https_remote"));
        assert_eq!(steps[2].run.as_deref(), Some("Set up git and environment"));
        assert_eq!(steps[2].script.as_deref(), Some("git fetch origin main"));
        assert_eq!(
            steps[2]
                .environment
                .as_ref()
                .and_then(|e| e.get("TAG_PREFIX"))
                .map(String::as_str),
            Some("<< parameters.tag_prefix >>")
        );
        assert_eq!(steps[3].command.as_deref(), Some("generate"));
        assert_eq!(
            steps[3]
                .with
                .as_ref()
                .and_then(|w| w.get("format"))
                .map(String::as_str),
            Some("binary")
        );
        assert_eq!(steps[4].orb.as_deref(), Some("toolkit/setup"));
    }

    #[test]
    fn load_config_parses_extra_job_verbatim_yaml() {
        let dir = TempDir::new().unwrap();
        let path = write_toml(
            &dir,
            r#"
[[extra_job]]
name = "ensure_registered"
yaml = """
description: Ensure registered
executor: orb-tools/default
steps:
  - run:
      name: Check
      command: echo ok
"""
"#,
        );
        let config = load_config(&path).unwrap();
        let extra = config.extra_job.unwrap();
        assert_eq!(extra[0].name, "ensure_registered");
        assert!(extra[0].yaml.contains("description: Ensure registered"));
        assert!(extra[0].yaml.contains("executor: orb-tools/default"));
    }

    #[test]
    fn load_config_parses_orbs_section() {
        let dir = TempDir::new().unwrap();
        let path = write_toml(
            &dir,
            r#"
[orbs]
"orb-tools" = "circleci/orb-tools@12.3.3"
"#,
        );
        let config = load_config(&path).unwrap();
        let orbs = config.orbs.unwrap();
        assert_eq!(
            orbs.get("orb-tools").map(String::as_str),
            Some("circleci/orb-tools@12.3.3")
        );
    }

    #[test]
    fn save_config_round_trips() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");

        let mut subcommands = IndexMap::new();
        let mut params = IndexMap::new();
        params.insert(
            "orb_path".to_string(),
            ParamOverride {
                default: Some("src/@orb.yml".to_string()),
            },
        );
        subcommands.insert(
            "help".to_string(),
            SubcommandConfig {
                generate_job: Some(false),
                ..SubcommandConfig::default()
            },
        );
        subcommands.insert(
            "generate".to_string(),
            SubcommandConfig {
                param: Some(params),
                ..SubcommandConfig::default()
            },
        );

        // Only the fields this round-trip is about are named; the rest come from
        // Default, so adding a field to OrbSection does not churn this test. The
        // values are arbitrary fixtures — the assertion is that whatever goes in
        // comes back out, not that these particular strings are meaningful.
        let original = OrbConfig {
            orb: Some(OrbSection {
                binary: Some("mytool".to_string()),
                namespaces: Some(vec!["my-org".to_string()]),
                orb_dir: "orb".to_string(),
                ..OrbSection::default()
            }),
            ci: None,
            orbs: None,
            subcommand: Some(subcommands),
            job_group: Some(vec![JobGroup {
                name: "sync".to_string(),
                description: Some("Sync".to_string()),
                steps: vec!["generate".to_string(), "validate".to_string()],
                params: None,
                ..Default::default()
            }]),
            extra_job: None,
            record: None,
        };

        save_config(&path, &original).unwrap();
        let loaded = load_config(&path).unwrap();
        assert_eq!(original, loaded);
    }

    // ── comments survive a write (#248) ────────────────────────────────────

    /// The config is hand-annotated and reviewed — this repo's own records why
    /// each image digest is pinned and which Renovate manager keeps it current.
    /// Serialising the struct over the file loses all of that silently.
    #[test]
    fn writing_preserves_comments_on_unchanged_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");
        std::fs::write(
            &path,
            "[orb]\n\
             binary = \"mytool\"\n\
             # Pinned digest so it survives regeneration; Renovate updates it.\n\
             builder_image = \"rust:1-slim-trixie@sha256:abc123\"\n",
        )
        .unwrap();

        let mut config = load_config(&path).unwrap();
        config.orb.as_mut().unwrap().orb_dir = "orb".to_string();
        save_config(&path, &config).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            written.contains("# Pinned digest so it survives regeneration"),
            "the comment must survive the write:\n{written}"
        );
        assert!(
            written.contains("rust:1-slim-trixie@sha256:abc123"),
            "the value it documents must survive too:\n{written}"
        );
    }

    /// A comment attached to a section header, not a key.
    #[test]
    fn writing_preserves_section_comments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");
        std::fs::write(
            &path,
            "# Drives the generated validation workflow.\n\
             [orb]\n\
             binary = \"mytool\"\n",
        )
        .unwrap();

        let config = load_config(&path).unwrap();
        save_config(&path, &config).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            written.contains("# Drives the generated validation workflow."),
            "a section comment must survive:\n{written}"
        );
    }

    /// Adding a key elsewhere must not disturb an annotated one.
    #[test]
    fn writing_preserves_comments_when_adding_a_section() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");
        std::fs::write(
            &path,
            "[orb]\n\
             # why this is pinned\n\
             base_image = \"debian:13-slim\"\n\
             binary = \"mytool\"\n",
        )
        .unwrap();

        let mut config = load_config(&path).unwrap();
        let mut subs = IndexMap::new();
        subs.insert(
            "somecmd".to_string(),
            SubcommandConfig {
                generate_job: Some(false),
                ..SubcommandConfig::default()
            },
        );
        config.subcommand = Some(subs);
        save_config(&path, &config).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            written.contains("# why this is pinned"),
            "an unrelated addition must not drop a comment:\n{written}"
        );
        assert!(
            written.contains("[subcommand.somecmd]"),
            "the new section must still be written:\n{written}"
        );
    }

    /// An untouched value keeps the formatting the user gave it. Rewriting an
    /// inline array as a multi-line one is the same "don't rewrite what I wrote"
    /// complaint as losing a comment, in smaller form.
    #[test]
    fn writing_leaves_unchanged_values_formatted_as_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");
        std::fs::write(
            &path,
            "[orb]\nbinary = \"mytool\"\nnamespaces = [\"a\", \"b\", \"c\"]\n",
        )
        .unwrap();

        let mut config = load_config(&path).unwrap();
        config.orb.as_mut().unwrap().orb_dir = "orb".to_string();
        save_config(&path, &config).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            written.contains(r#"namespaces = ["a", "b", "c"]"#),
            "an untouched array must keep its inline form:\n{written}"
        );
    }

    /// A value that did change is written out, formatting and all.
    #[test]
    fn writing_updates_a_changed_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");
        std::fs::write(&path, "[orb]\n# keep me\nbinary = \"oldtool\"\n").unwrap();

        let mut config = load_config(&path).unwrap();
        config.orb.as_mut().unwrap().binary = Some("newtool".to_string());
        save_config(&path, &config).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains(r#"binary = "newtool""#), "got:\n{written}");
        assert!(
            !written.contains("oldtool"),
            "stale value left behind:\n{written}"
        );
        assert!(
            written.contains("# keep me"),
            "comment lost on update:\n{written}"
        );
    }

    /// Values must still round-trip — preserving comments is worthless if the
    /// data stops surviving.
    #[test]
    fn writing_over_an_annotated_file_still_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");
        std::fs::write(
            &path,
            "# leading comment\n[orb]\nbinary = \"mytool\"\n# mid comment\nnamespaces = [\"my-org\"]\n",
        )
        .unwrap();

        let mut config = load_config(&path).unwrap();
        config.orb.as_mut().unwrap().orb_dir = "custom".to_string();
        save_config(&path, &config).unwrap();

        let reloaded = load_config(&path).unwrap();
        assert_eq!(
            reloaded, config,
            "values must survive the comment-preserving write"
        );
    }

    /// A read failure that is not "no such file" must not be treated as "nothing
    /// to preserve" — that turns the one case this code exists to protect into a
    /// silent clobber.
    #[test]
    fn writing_refuses_when_the_existing_file_cannot_be_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");
        // A comment carrying a non-UTF-8 byte: readable as bytes, not as a String.
        let mut bytes = b"[orb]\n# maintained by Jos\xe9\nbinary = \"mytool\"\n".to_vec();
        bytes.push(b'\n');
        std::fs::write(&path, &bytes).unwrap();

        let config = OrbConfig {
            orb: Some(OrbSection {
                binary: Some("mytool".to_string()),
                ..OrbSection::default()
            }),
            ..OrbConfig::default()
        };
        assert!(
            save_config(&path, &config).is_err(),
            "an unreadable existing file must abort the write, not overwrite it"
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "the file must be left exactly as it was"
        );
    }

    /// `[[job_group]]` entries are arrays of tables; `config add-job-group`
    /// rewrites them, so their comments have to survive like any other.
    #[test]
    fn writing_preserves_comments_in_arrays_of_tables() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");
        std::fs::write(
            &path,
            "[orb]\nbinary = \"mytool\"\n\n# why the sync group exists\n[[job_group]]\nname = \"sync\"\nsteps = [\"generate\"]\n\n# why the publish group exists\n[[job_group]]\nname = \"publish\"\nsteps = [\"validate\"]\n",
        )
        .unwrap();

        let mut config = load_config(&path).unwrap();
        config.job_group.as_mut().unwrap()[1].steps = vec!["diff".to_string()];
        save_config(&path, &config).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            written.contains("# why the sync group exists"),
            "an untouched array-of-tables entry must keep its comment:\n{written}"
        );
        assert!(
            written.contains("# why the publish group exists"),
            "a changed array-of-tables entry must keep its comment:\n{written}"
        );
        assert!(
            written.contains("diff"),
            "the change must be applied:\n{written}"
        );
    }

    /// serde drops keys it does not know, so a key from a newer generator must
    /// not be swept away by an older binary writing the file.
    #[test]
    fn writing_keeps_keys_the_struct_does_not_model() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");
        std::fs::write(
            &path,
            "[orb]\nbinary = \"mytool\"\n\n# a section a newer version understands\n[future_section]\nsetting = true\n",
        )
        .unwrap();

        let mut config = load_config(&path).unwrap();
        config.orb.as_mut().unwrap().orb_dir = "orb".to_string();
        save_config(&path, &config).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            written.contains("[future_section]") && written.contains("setting = true"),
            "an unrecognised section must survive:\n{written}"
        );
        assert!(
            written.contains("# a section a newer version understands"),
            "and so must its comment:\n{written}"
        );
    }

    /// A comment on the same line as a changed value belongs to that value.
    #[test]
    fn writing_preserves_a_trailing_comment_on_a_changed_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");
        std::fs::write(&path, "[orb]\nbinary = \"oldtool\" # the binary name\n").unwrap();

        let mut config = load_config(&path).unwrap();
        config.orb.as_mut().unwrap().binary = Some("newtool".to_string());
        save_config(&path, &config).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            written.contains("newtool"),
            "the change must apply:\n{written}"
        );
        assert!(
            written.contains("# the binary name"),
            "a same-line comment must survive a value change:\n{written}"
        );
    }

    /// An inline `x = { … }` that the serialiser emits as a `[x]` section must
    /// not carry its old key decor into the header, rendering `[x ]`.
    #[test]
    fn writing_converts_an_inline_table_without_leaking_decor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");
        std::fs::write(
            &path,
            "orbs = { toolkit = \"old@1.0.0\" }\n\n[orb]\nbinary = \"mytool\"\n",
        )
        .unwrap();

        let mut config = load_config(&path).unwrap();
        config
            .orbs
            .as_mut()
            .unwrap()
            .insert("toolkit".to_string(), "new@2.0.0".to_string());
        save_config(&path, &config).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            written.contains("[orbs]"),
            "the section header must render cleanly:\n{written}"
        );
        assert!(
            !written.contains("[orbs "),
            "stale key decor leaked into the header:\n{written}"
        );
        assert!(
            written.contains("new@2.0.0"),
            "the change must apply:\n{written}"
        );
    }

    /// The comment above a key lives in that key's leaf-decor *prefix*. Clearing
    /// the whole decor to tidy a section header therefore deletes it — the very
    /// loss this module exists to prevent.
    #[test]
    fn converting_an_inline_table_keeps_the_comment_above_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");
        std::fs::write(
            &path,
            "# keep pinned; Renovate updates these\norbs = { toolkit = \"jerus-org/circleci-toolkit@2.9.1\" }\n\n[orb]\nbinary = \"mytool\"\n",
        )
        .unwrap();

        let mut config = load_config(&path).unwrap();
        config.orbs.as_mut().unwrap().insert(
            "toolkit".to_string(),
            "jerus-org/circleci-toolkit@3.0.0".to_string(),
        );
        save_config(&path, &config).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            written.contains("# keep pinned; Renovate updates these"),
            "the comment above the key must survive the conversion:\n{written}"
        );
        assert!(
            !written.contains("[orbs "),
            "stale key decor leaked:\n{written}"
        );
    }

    /// A same-line comment on a value that becomes a section must survive too.
    #[test]
    fn converting_an_inline_table_keeps_its_trailing_comment() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");
        std::fs::write(
            &path,
            "orbs = { toolkit = \"old@1.0.0\" } # renovate keeps this current\n\n[orb]\nbinary = \"mytool\"\n",
        )
        .unwrap();

        let mut config = load_config(&path).unwrap();
        config
            .orbs
            .as_mut()
            .unwrap()
            .insert("toolkit".to_string(), "new@2.0.0".to_string());
        save_config(&path, &config).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            written.contains("# renovate keeps this current"),
            "a trailing comment must survive the conversion:\n{written}"
        );
    }

    // ── #267: the write itself ─────────────────────────────────────────────

    fn annotated_config(path: &std::path::Path) -> OrbConfig {
        std::fs::write(
            path,
            concat!(
                "[orb]\n",
                "# renovate keeps this digest current\n",
                "builder_image = \"rust:1-slim-trixie@sha256:abc123\"\n",
                "binary = \"mytool\"\n",
            ),
        )
        .unwrap();
        load_config(path).unwrap()
    }

    #[test]
    fn a_successful_write_leaves_no_temporary_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");
        let mut config = annotated_config(&path);
        config.orb.as_mut().unwrap().binary = Some("renamed".to_string());

        save_config(&path, &config).unwrap();

        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            entries.len(),
            1,
            "stray files in the directory: {entries:?}"
        );
    }

    /// The file carries comments that exist nowhere else, so a write that fails
    /// part-way must leave the previous contents intact rather than a truncated
    /// or empty file. Writing to a sibling temporary and renaming over the
    /// target is what buys that: the target is only ever replaced whole.
    ///
    /// The failure is induced by occupying the staging path with a directory:
    /// deterministic, and needing no permissions games that would behave
    /// differently for root in a CI container.
    #[test]
    fn a_failed_write_leaves_the_original_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");
        let mut config = annotated_config(&path);
        let before = std::fs::read_to_string(&path).unwrap();

        std::fs::create_dir(write_temp_path(&path)).unwrap();
        config.orb.as_mut().unwrap().binary = Some("renamed".to_string());

        assert!(
            save_config(&path, &config).is_err(),
            "a write that cannot complete must report it"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            before,
            "the original must survive a failed write byte-for-byte"
        );
    }

    /// A config may be chmodded deliberately — it names CircleCI contexts and
    /// pinned images. The staged file is created at the umask default, so
    /// without carrying the mode across, a save silently widens it.
    #[test]
    fn a_save_keeps_the_permissions_the_config_had() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");
        let mut config = annotated_config(&path);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

        config.orb.as_mut().unwrap().binary = Some("renamed".to_string());
        save_config(&path, &config).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "got {mode:o}");
    }

    /// A config reached through a symlink — a shared or dotfile-managed one —
    /// must be written *through* the link. Renaming onto the link's own path
    /// replaces it with a regular file and leaves the real file holding the
    /// previous config.
    #[test]
    fn a_save_writes_through_a_symlinked_config() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.toml");
        let link = dir.path().join("gen-circleci-orb.toml");
        let mut config = annotated_config(&real);
        std::os::unix::fs::symlink(&real, &link).unwrap();

        config.orb.as_mut().unwrap().binary = Some("renamed".to_string());
        save_config(&link, &config).unwrap();

        assert!(
            std::fs::symlink_metadata(&link).unwrap().is_symlink(),
            "the symlink must survive the save"
        );
        assert!(
            std::fs::read_to_string(&real).unwrap().contains("renamed"),
            "the file the link points at must be the one updated"
        );
    }

    /// A replace that fails must not leave the staged file beside the config.
    ///
    /// Reached by renaming onto a directory, which is the one way the staging
    /// write can succeed and the replace still fail.
    #[test]
    fn a_failed_replace_removes_the_staged_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("occupied-by-a-directory");
        std::fs::create_dir(&target).unwrap();

        assert!(
            write_atomically(&target, "new contents").is_err(),
            "renaming over a directory must fail"
        );
        assert!(
            !write_temp_path(&target).exists(),
            "the staged file must not survive a failed replace"
        );
    }

    /// The temporary must be a sibling: `rename` is only atomic within one
    /// filesystem, and a temp directory elsewhere could be on another.
    #[test]
    fn the_temporary_is_a_sibling_of_the_target() {
        let path = std::path::Path::new("/some/where/gen-circleci-orb.toml");
        assert_eq!(write_temp_path(path).parent(), path.parent());
    }

    /// Saving an unchanged config must not touch the file's shape at all.
    ///
    /// "Unchanged" means every materialised setting is already stated: a config
    /// that omits one gains it on the next save, which is the point of giving
    /// them real defaults (#245/#251) rather than a rewrite this guards against.
    #[test]
    fn saving_an_unchanged_config_rewrites_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");
        let original = concat!(
            "# keep pinned\n",
            "orbs = { toolkit = \"pinned@2.9.1\" }\n",
            "\n",
            "[orb]\n",
            "binary = \"mytool\"\n",
            "namespaces = [\"my-org\"]\n",
            "orb_dir = \"orb\"\n",
            "base_image = \"debian:13-slim\"\n",
            "builder_image = \"rust:1-slim-trixie\"\n",
            "install_method = \"binstall\"\n",
            "crate_wait_attempts = 40\n",
            "crate_wait_seconds = 15\n",
        );
        std::fs::write(&path, original).unwrap();

        let config = load_config(&path).unwrap();
        save_config(&path, &config).unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            original,
            "a no-op save must leave the file byte-identical"
        );
    }

    /// Clearing a field inside a `[[job_group]]` entry must actually remove the
    /// key, not silently persist and reappear on the next load.
    #[test]
    fn removing_a_key_inside_an_array_of_tables_sticks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");
        std::fs::write(
            &path,
            "[orb]\nbinary = \"mytool\"\n\n[[job_group]]\nname = \"sync\"\nsteps = [\"generate\"]\nexecutor = \"custom-executor\"\n",
        )
        .unwrap();

        let mut config = load_config(&path).unwrap();
        config.job_group.as_mut().unwrap()[0].executor = None;
        save_config(&path, &config).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            !written.contains("custom-executor"),
            "the cleared key must be removed:\n{written}"
        );
        assert_eq!(
            load_config(&path).unwrap().job_group.unwrap()[0].executor,
            None,
            "and must not reappear on reload"
        );
    }

    /// Entries matched by position reattach comments to the wrong entry the
    /// moment one is removed or reordered.
    #[test]
    fn removing_an_array_of_tables_entry_keeps_comments_with_their_own_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");
        std::fs::write(
            &path,
            "[orb]\nbinary = \"mytool\"\n\n# the sync group\n[[job_group]]\nname = \"sync\"\nsteps = [\"generate\"]\n\n# the middle group\n[[job_group]]\nname = \"middle\"\nsteps = [\"validate\"]\n\n# the publish group\n[[job_group]]\nname = \"publish\"\nsteps = [\"diff\"]\n",
        )
        .unwrap();

        let mut config = load_config(&path).unwrap();
        config.job_group.as_mut().unwrap().remove(1);
        save_config(&path, &config).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            !written.contains("# the middle group"),
            "the removed entry's comment must go with it:\n{written}"
        );
        let publish = written
            .find("name = \"publish\"")
            .expect("publish entry kept");
        let comment = written
            .find("# the publish group")
            .expect("publish comment kept");
        assert!(
            comment < publish && written[comment..publish].find("name =").is_none(),
            "each comment must stay with its own entry:\n{written}"
        );
    }

    /// The comment above a converted inline table must stay attached to it, not
    /// drift up to read as a trailing comment on the previous section.
    #[test]
    fn a_converted_inline_tables_comment_stays_adjacent_to_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");
        // concat! rather than a wrapped literal: rustfmt joins `\`-continued
        // strings and bakes the indentation in as real spaces.
        let original = concat!(
            "# keep pinned; Renovate updates these\n",
            "orbs = { toolkit = \"old@1.0.0\" }\n",
            "\n",
            "[orb]\n",
            "binary = \"mytool\"\n",
        );
        std::fs::write(&path, original).unwrap();

        let mut config = load_config(&path).unwrap();
        config
            .orbs
            .as_mut()
            .unwrap()
            .insert("toolkit".to_string(), "new@2.0.0".to_string());
        save_config(&path, &config).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            written.contains("# keep pinned; Renovate updates these\n[orbs]"),
            "the comment must sit on the line directly above its section, not \
             drift up to read as a trailing note on the previous one:\n{written}"
        );
    }

    /// A config that no longer deserialises must not quietly turn the write
    /// add-only; removals would silently stop applying.
    #[test]
    fn writing_fails_when_the_existing_file_no_longer_deserialises() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");
        // Valid TOML, wrong type for a modelled field.
        std::fs::write(
            &path,
            "[orb]\nbinary = \"mytool\"\ncrate_wait_attempts = \"forty\"\n",
        )
        .unwrap();

        let config = OrbConfig {
            orb: Some(OrbSection {
                binary: Some("mytool".to_string()),
                ..OrbSection::default()
            }),
            ..OrbConfig::default()
        };
        assert!(
            save_config(&path, &config).is_err(),
            "a config that cannot be re-read must abort the write, not silently \
             stop removing keys"
        );
    }

    // ── crate-wait defaults live in the struct ─────────────────────────────

    /// The defaults are the struct's, not the type's: `u32::default()` is 0,
    /// which as an attempt count would mean "never wait".
    #[test]
    fn orb_section_default_carries_the_crate_wait_defaults() {
        use crate::orb_config::{DEFAULT_CRATE_WAIT_ATTEMPTS, DEFAULT_CRATE_WAIT_SECONDS};
        let section = OrbSection::default();
        assert_eq!(section.crate_wait_attempts, DEFAULT_CRATE_WAIT_ATTEMPTS);
        assert_eq!(section.crate_wait_seconds, DEFAULT_CRATE_WAIT_SECONDS);
    }

    /// A config written before these settings existed must still load, taking
    /// the defaults rather than zeroes.
    #[test]
    fn config_without_crate_wait_keys_loads_the_defaults() {
        use crate::orb_config::{DEFAULT_CRATE_WAIT_ATTEMPTS, DEFAULT_CRATE_WAIT_SECONDS};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");
        std::fs::write(&path, "[orb]\nbinary = \"mytool\"\n").unwrap();
        let orb = load_config(&path).unwrap().orb.unwrap();
        assert_eq!(orb.crate_wait_attempts, DEFAULT_CRATE_WAIT_ATTEMPTS);
        assert_eq!(orb.crate_wait_seconds, DEFAULT_CRATE_WAIT_SECONDS);
    }

    /// Saving writes the values out, so the config documents the knobs and the
    /// value in force — the scaffolding a consumer edits rather than discovers.
    #[test]
    fn saved_config_spells_out_the_crate_wait_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");
        let config = OrbConfig {
            orb: Some(OrbSection {
                binary: Some("mytool".to_string()),
                ..OrbSection::default()
            }),
            ..OrbConfig::default()
        };
        save_config(&path, &config).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            written.contains("crate_wait_attempts = 40")
                && written.contains("crate_wait_seconds = 15"),
            "the written config must show the defaults in force:\n{written}"
        );
    }

    // ── #251: the rest of [orb]'s defaults are materialised too ────────────

    #[test]
    fn orb_section_default_materialises_the_orb_settings() {
        let orb = OrbSection::default();
        assert_eq!(orb.orb_dir, DEFAULT_ORB_DIR);
        assert_eq!(orb.install_method, DEFAULT_INSTALL_METHOD);
        assert_eq!(orb.base_image, DEFAULT_BASE_IMAGE);
        assert_eq!(orb.builder_image, DEFAULT_BUILDER_IMAGE);
    }

    /// A config written before these keys existed still resolves to the
    /// recommendations, so nothing has to be re-`init`ed to keep working.
    #[test]
    fn a_config_omitting_the_orb_settings_loads_the_recommendations() {
        let config: OrbConfig = toml::from_str("[orb]\nbinary = \"mytool\"\n").unwrap();
        let orb = config.orb.unwrap();
        assert_eq!(orb.orb_dir, DEFAULT_ORB_DIR);
        assert_eq!(orb.install_method, DEFAULT_INSTALL_METHOD);
        assert_eq!(orb.base_image, DEFAULT_BASE_IMAGE);
        assert_eq!(orb.builder_image, DEFAULT_BUILDER_IMAGE);
    }

    /// `circleci_cli_version` is deliberately *not* materialised: a recorded
    /// version is an opt-in to bundling the CLI, so a default value in every
    /// config would install it into every executor that never asked for it.
    #[test]
    fn circleci_cli_version_stays_absent_when_unconfigured() {
        let config: OrbConfig = toml::from_str("[orb]\nbinary = \"mytool\"\n").unwrap();
        assert_eq!(config.orb.unwrap().circleci_cli_version, None);
    }

    #[test]
    fn saved_config_spells_out_the_orb_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");
        let config = OrbConfig {
            orb: Some(OrbSection {
                binary: Some("mytool".to_string()),
                ..OrbSection::default()
            }),
            ..OrbConfig::default()
        };
        save_config(&path, &config).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        for expected in [
            format!("orb_dir = \"{DEFAULT_ORB_DIR}\""),
            format!("install_method = \"{DEFAULT_INSTALL_METHOD}\""),
            format!("base_image = \"{DEFAULT_BASE_IMAGE}\""),
            format!("builder_image = \"{DEFAULT_BUILDER_IMAGE}\""),
        ] {
            assert!(
                written.contains(&expected),
                "expected `{expected}` in the written config:\n{written}"
            );
        }
    }

    /// The whole point of the merge: a recorded value is the project's stated
    /// configuration and a save must not reset it to the recommendation.
    #[test]
    fn a_recorded_orb_setting_survives_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gen-circleci-orb.toml");
        std::fs::write(
            &path,
            concat!(
                "[orb]\n",
                "binary = \"mytool\"\n",
                "# pinned so the digest survives regeneration\n",
                "builder_image = \"rust:1-slim-trixie@sha256:abc123\"\n",
            ),
        )
        .unwrap();
        let mut config = load_config(&path).unwrap();
        assert_eq!(
            config.orb.as_ref().unwrap().builder_image,
            "rust:1-slim-trixie@sha256:abc123"
        );
        config.orb.as_mut().unwrap().binary = Some("mytool".to_string());
        save_config(&path, &config).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            written.contains("rust:1-slim-trixie@sha256:abc123")
                && written.contains("# pinned so the digest survives regeneration"),
            "the pin and its comment must survive:\n{written}"
        );
    }
}
