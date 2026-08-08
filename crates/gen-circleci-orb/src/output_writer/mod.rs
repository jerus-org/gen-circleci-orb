use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Directories (relative to the orb root) treated as fully owned by the
/// generator: any file under them that the current generation did not produce
/// is an orphan and is pruned. Scoped so hand-authored / auxiliary files
/// elsewhere in the tree (e.g. `src/@orb.yml`, `src/examples/`) are never
/// touched.
const GENERATOR_OWNED_DIRS: &[&str] = &["src/commands", "src/jobs", "src/scripts"];

/// Summary of what the writer did.
#[derive(Debug, Default, PartialEq)]
pub struct WriteReport {
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub removed: usize,
}

/// Write `files` (relative path → content) under `root`.
///
/// Diff-aware: files whose content is identical to what's on disk are skipped.
/// When `dry_run` is true nothing is written; instead a summary is printed to stderr.
///
/// After writing, prunes orphans in the generator-owned dirs. `custom_files` lists
/// hand-authored paths (relative to `root`) that are kept even though they are not
/// in `files` — i.e. the config "authorises" them. Anything in the owned dirs that
/// is neither generated nor authorised is removed.
pub fn write_tree(
    root: &Path,
    files: &HashMap<PathBuf, String>,
    custom_files: &[String],
    dry_run: bool,
) -> Result<WriteReport> {
    let mut report = WriteReport::default();

    for (rel_path, content) in files {
        let abs_path = root.join(rel_path);
        // Decide, act, count. `classify` is not given `dry_run`, so the report
        // cannot come to depend on whether anything was written — which is what
        // `--check` gates orb publishing on.
        let action = classify(&abs_path, content)?;
        apply(action, &abs_path, rel_path, content, dry_run)?;
        match action {
            FileAction::Create => report.created += 1,
            FileAction::Update => report.updated += 1,
            FileAction::Unchanged => report.unchanged += 1,
        }
    }

    prune_orphans(root, files, custom_files, dry_run, &mut report)?;

    Ok(report)
}

/// What `write_tree` will do with one generated file.
#[derive(Debug, Clone, Copy, PartialEq)]
enum FileAction {
    Create,
    Update,
    Unchanged,
}

/// Decide the action for one file. Reads, never writes — so the same answer is
/// produced whether or not the caller intends to act on it.
fn classify(abs_path: &Path, content: &str) -> Result<FileAction> {
    if !abs_path.exists() {
        return Ok(FileAction::Create);
    }
    // Named, because a `--check` run reads every generated path: an unreadable
    // or non-UTF-8 file would otherwise abort the whole run with a bare
    // "stream did not contain valid UTF-8" and no way to tell which one.
    let current =
        fs::read_to_string(abs_path).with_context(|| format!("reading {}", abs_path.display()))?;
    Ok(if current == content {
        FileAction::Unchanged
    } else {
        FileAction::Update
    })
}

/// Carry out `action` — or, under `dry_run`, describe it and change nothing.
fn apply(
    action: FileAction,
    abs_path: &Path,
    rel_path: &Path,
    content: &str,
    dry_run: bool,
) -> Result<()> {
    let verb = match action {
        FileAction::Unchanged => return Ok(()),
        FileAction::Create => "create",
        FileAction::Update => "update",
    };
    if dry_run {
        eprintln!("[dry-run] would {verb}: {}", rel_path.display());
        return Ok(());
    }
    if let Some(parent) = abs_path.parent() {
        fs::create_dir_all(parent)?;
    }
    // A plain write, unlike the files `fs_atomic` guards: this tree is generated
    // in full from the binary's `--help` and never hand-edited, so a torn write
    // costs a re-run rather than something nobody can reconstruct.
    fs::write(abs_path, content)?;
    Ok(())
}

/// Delete `abs_path` — or, under `dry_run`, describe it and change nothing.
/// The removal counterpart of [`apply`], so both halves of the writer describe
/// and act through the same shape.
fn discard(abs_path: &Path, rel_path: &Path, dry_run: bool) -> Result<()> {
    if dry_run {
        eprintln!("[dry-run] would remove: {}", rel_path.display());
        return Ok(());
    }
    fs::remove_file(abs_path)?;
    Ok(())
}

/// Delete files under the generator-owned directories that are not in the
/// freshly generated set (`files`). Treats `commands/`, `jobs/`, `scripts/` as
/// owned by the generator so suppressing/renaming/removing a subcommand does not
/// leave orphan files behind. Respects `dry_run` (reports, writes nothing).
fn prune_orphans(
    root: &Path,
    files: &HashMap<PathBuf, String>,
    custom_files: &[String],
    dry_run: bool,
    report: &mut WriteReport,
) -> Result<()> {
    let keep = Keep::new(files.keys().cloned().collect(), custom_files);

    for dir in GENERATOR_OWNED_DIRS {
        let abs_dir = root.join(dir);
        if abs_dir.is_dir() {
            prune_dir(&abs_dir, Path::new(dir), &keep, dry_run, report)?;
        }
    }
    Ok(())
}

/// What survives a prune: this run's output, and what the config authorises.
struct Keep {
    generated: HashSet<PathBuf>,
    authorised: Vec<PathBuf>,
}

impl Keep {
    fn new(generated: HashSet<PathBuf>, custom_files: &[String]) -> Self {
        let authorised = custom_files
            .iter()
            .map(|entry| entry.trim())
            // An empty entry is a prefix of everything, so one blank array
            // element would exempt the whole tree — and `verify_no_drift`,
            // which reads the removal count, would pass on a drifted orb.
            .filter(|entry| !entry.is_empty())
            .map(PathBuf::from)
            .inspect(|entry| {
                if GENERATOR_OWNED_DIRS
                    .iter()
                    .any(|d| Path::new(d).starts_with(entry))
                {
                    tracing::warn!(
                        "[orb] custom_files entry {} covers a whole generator-owned \
                         directory, so nothing in it will be pruned",
                        entry.display()
                    );
                }
            })
            .collect();
        Self {
            generated,
            authorised,
        }
    }

    /// An authorised entry covers the path it names *and everything beneath
    /// it*, so a consumer can protect `src/scripts/lib` without listing each
    /// file in it. Before the prune recursed, nested files were immune by
    /// accident; naming the directory is how that intent is now expressed.
    fn covers(&self, rel: &Path) -> bool {
        self.generated.contains(rel) || self.authorised.iter().any(|entry| rel.starts_with(entry))
    }
}

/// Prune one directory and everything below it.
///
/// Recursive because `apply` creates whatever parents a generated path needs,
/// so an orphan can sit at any depth, and a nested one left behind is invisible
/// to `verify_no_drift` (#275). Empty directories are left where they fall —
/// deleting them would widen the reach for something that is not drift.
///
/// A symlink is never followed — `file_type` does not resolve it, so it is not
/// a directory here — but an unauthorised one is still removed. `orb pack`
/// reads through links, so leaving one would publish content that `--check`
/// reported no drift for. Unlinking removes the link, never its target.
fn prune_dir(
    abs_dir: &Path,
    rel_dir: &Path,
    keep: &Keep,
    dry_run: bool,
    report: &mut WriteReport,
) -> Result<()> {
    for entry in fs::read_dir(abs_dir)? {
        let entry = entry?;
        let rel = rel_dir.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            prune_dir(&entry.path(), &rel, keep, dry_run, report)?;
        } else if !keep.covers(&rel) {
            discard(&entry.path(), &rel, dry_run)?;
            report.removed += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn single_file(path: &str, content: &str) -> HashMap<PathBuf, String> {
        let mut m = HashMap::new();
        m.insert(PathBuf::from(path), content.to_string());
        m
    }

    /// Also covers parent-directory creation: the root is empty, so `src/` has
    /// to be made on the way.
    #[test]
    fn new_file_is_created() {
        let dir = TempDir::new().unwrap();
        let files = single_file("src/foo.yml", "hello");
        let report = write_tree(dir.path(), &files, &[], false).unwrap();
        assert_eq!(report.created, 1);
        assert_eq!(report.updated, 0);
        assert_eq!(report.unchanged, 0);
        assert_eq!(
            fs::read_to_string(dir.path().join("src/foo.yml")).unwrap(),
            "hello"
        );
    }

    #[test]
    fn identical_file_is_skipped() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/foo.yml"), "hello").unwrap();

        let files = single_file("src/foo.yml", "hello");
        let report = write_tree(dir.path(), &files, &[], false).unwrap();
        assert_eq!(report.unchanged, 1);
        assert_eq!(report.created, 0);
        assert_eq!(report.updated, 0);
    }

    #[test]
    fn changed_file_is_updated() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/foo.yml"), "old content").unwrap();

        let files = single_file("src/foo.yml", "new content");
        let report = write_tree(dir.path(), &files, &[], false).unwrap();
        assert_eq!(report.updated, 1);
        assert_eq!(
            fs::read_to_string(dir.path().join("src/foo.yml")).unwrap(),
            "new content"
        );
    }

    /// `write_tree` will create a nested path, so an orphan can be nested too —
    /// and one that survives is invisible to `verify_no_drift`.
    #[test]
    fn a_nested_orphan_is_pruned() {
        let dir = TempDir::new().unwrap();
        let deep = dir.path().join("src/scripts/deep");
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("stale.sh"), "stale").unwrap();

        let files = single_file("src/scripts/flat.sh", "x");
        let report = write_tree(dir.path(), &files, &[], false).unwrap();

        assert_eq!(report.removed, 1);
        assert!(!deep.join("stale.sh").exists());
    }

    /// Recursion must not reach authorised files: `custom_files` paths are
    /// relative to the root and may name a nested one.
    #[test]
    fn a_nested_custom_file_is_preserved() {
        let dir = TempDir::new().unwrap();
        let deep = dir.path().join("src/scripts/deep");
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("keep.sh"), "custom").unwrap();
        fs::write(deep.join("stale.sh"), "stale").unwrap();

        let files = single_file("src/scripts/flat.sh", "x");
        let custom = ["src/scripts/deep/keep.sh".to_string()];
        let report = write_tree(dir.path(), &files, &custom, false).unwrap();

        assert_eq!(report.removed, 1, "only the unauthorised one");
        assert!(deep.join("keep.sh").exists());
        assert!(!deep.join("stale.sh").exists());
    }

    /// Authorising a directory covers what is under it, so a consumer need not
    /// list every nested helper to keep it.
    #[test]
    fn authorising_a_directory_covers_what_is_under_it() {
        let dir = TempDir::new().unwrap();
        let lib = dir.path().join("src/scripts/lib");
        fs::create_dir_all(&lib).unwrap();
        fs::write(lib.join("common.sh"), "helper").unwrap();
        fs::write(lib.join("more.sh"), "helper").unwrap();
        fs::write(dir.path().join("src/scripts/stale.sh"), "stale").unwrap();

        let files = single_file("src/scripts/flat.sh", "x");
        let custom = ["src/scripts/lib".to_string()];
        let report = write_tree(dir.path(), &files, &custom, false).unwrap();

        assert_eq!(report.removed, 1, "only the unauthorised one");
        assert!(lib.join("common.sh").exists());
        assert!(lib.join("more.sh").exists());
    }

    #[test]
    fn nested_files_outside_owned_dirs_are_not_pruned() {
        let dir = TempDir::new().unwrap();
        let deep = dir.path().join("src/examples/nested");
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("example.yml").as_path(), "ex").unwrap();

        let files = single_file("src/commands/keep.yml", "new");
        let report = write_tree(dir.path(), &files, &[], false).unwrap();

        assert_eq!(report.removed, 0);
        assert!(deep.join("example.yml").exists());
    }

    /// An orphaned link is removed like any other orphan — `orb pack` reads
    /// through links, so leaving one publishes content `--check` called clean.
    /// Only the link goes: what it points at is not ours.
    #[cfg(unix)]
    #[test]
    fn an_orphaned_symlink_is_unlinked_but_its_target_is_not() {
        let outside = TempDir::new().unwrap();
        let elsewhere = outside.path().join("precious.txt");
        fs::write(&elsewhere, "not ours").unwrap();

        let dir = TempDir::new().unwrap();
        let scripts = dir.path().join("src/scripts");
        fs::create_dir_all(&scripts).unwrap();
        std::os::unix::fs::symlink(outside.path(), scripts.join("to-dir")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, scripts.join("to-file")).unwrap();

        let files = single_file("src/scripts/flat.sh", "x");
        let report = write_tree(dir.path(), &files, &[], false).unwrap();

        assert_eq!(report.removed, 2, "both links are orphans");
        assert!(elsewhere.exists(), "the target must be untouched");
        assert!(outside.path().is_dir(), "the linked directory must survive");
    }

    /// A blank entry is a prefix of every path, so one would exempt the whole
    /// tree — and leave `verify_no_drift` green on a drifted orb.
    #[test]
    fn a_blank_custom_files_entry_does_not_exempt_everything() {
        let dir = TempDir::new().unwrap();
        let cmds = dir.path().join("src/commands");
        fs::create_dir_all(&cmds).unwrap();
        fs::write(cmds.join("orphan.yml"), "stale").unwrap();

        let files = single_file("src/commands/keep.yml", "new");
        let report = write_tree(dir.path(), &files, &["  ".to_string()], false).unwrap();

        assert_eq!(report.removed, 1);
        assert!(!cmds.join("orphan.yml").exists());
    }

    #[test]
    fn orphan_in_owned_dir_is_pruned() {
        let dir = TempDir::new().unwrap();
        let cmds = dir.path().join("src/commands");
        fs::create_dir_all(&cmds).unwrap();
        fs::write(cmds.join("keep.yml"), "old").unwrap();
        fs::write(cmds.join("orphan.yml"), "stale").unwrap();

        // Generation only produces keep.yml.
        let files = single_file("src/commands/keep.yml", "new");
        let report = write_tree(dir.path(), &files, &[], false).unwrap();

        assert_eq!(report.removed, 1, "orphan must be pruned");
        assert!(cmds.join("keep.yml").exists(), "generated file kept");
        assert!(
            !cmds.join("orphan.yml").exists(),
            "orphan must be deleted from disk"
        );
    }

    #[test]
    fn authorised_custom_file_is_preserved() {
        let dir = TempDir::new().unwrap();
        let cmds = dir.path().join("src/commands");
        fs::create_dir_all(&cmds).unwrap();
        // A hand-authored command the generator does not produce, plus a true orphan.
        fs::write(cmds.join("build_container.yml"), "custom").unwrap();
        fs::write(cmds.join("orphan.yml"), "stale").unwrap();

        let files = single_file("src/commands/generate.yml", "gen");
        let custom = ["src/commands/build_container.yml".to_string()];
        let report = write_tree(dir.path(), &files, &custom, false).unwrap();

        assert_eq!(report.removed, 1, "only the unauthorised orphan is pruned");
        assert!(
            cmds.join("build_container.yml").exists(),
            "config-authorised custom file must be preserved"
        );
        assert!(
            !cmds.join("orphan.yml").exists(),
            "unauthorised orphan must be pruned"
        );
    }

    #[test]
    fn dry_run_reports_but_does_not_prune() {
        let dir = TempDir::new().unwrap();
        let jobs = dir.path().join("src/jobs");
        fs::create_dir_all(&jobs).unwrap();
        fs::write(jobs.join("orphan.yml"), "stale").unwrap();

        let files = single_file("src/jobs/keep.yml", "new");
        let report = write_tree(dir.path(), &files, &[], true).unwrap();

        assert_eq!(report.removed, 1, "dry_run still counts would-be removals");
        assert!(
            jobs.join("orphan.yml").exists(),
            "dry_run must not delete files"
        );
    }

    #[test]
    fn files_outside_owned_dirs_are_not_pruned() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("src/examples")).unwrap();
        // @orb.yml at src root and an examples file: neither is in the generated
        // set, but both live outside the generator-owned dirs.
        fs::write(dir.path().join("src/@orb.yml"), "version: 2.1").unwrap();
        fs::write(dir.path().join("src/examples/example.yml"), "ex").unwrap();

        let files = single_file("src/commands/keep.yml", "new");
        let report = write_tree(dir.path(), &files, &[], false).unwrap();

        assert_eq!(report.removed, 0, "non-owned files must be left alone");
        assert!(dir.path().join("src/@orb.yml").exists());
        assert!(dir.path().join("src/examples/example.yml").exists());
    }

    #[test]
    fn dry_run_writes_nothing() {
        let dir = TempDir::new().unwrap();
        let files = single_file("src/foo.yml", "hello");
        let report = write_tree(dir.path(), &files, &[], true).unwrap();
        assert_eq!(report.created, 1, "dry_run should still count created");
        assert!(
            !dir.path().join("src/foo.yml").exists(),
            "dry_run must not write files"
        );
    }

    // ── multi-outcome runs, and the report's independence from writing ────

    /// Lay out a tree exercising all four outcomes at once: one file to create,
    /// one to update, one already correct, and one orphan to prune.
    fn mixed_tree(dir: &TempDir) -> HashMap<PathBuf, String> {
        let cmds = dir.path().join("src/commands");
        fs::create_dir_all(&cmds).unwrap();
        fs::write(cmds.join("stale.yml"), "old").unwrap();
        fs::write(cmds.join("current.yml"), "same").unwrap();
        fs::write(cmds.join("orphan.yml"), "stale").unwrap();

        let mut files = HashMap::new();
        files.insert(PathBuf::from("src/commands/stale.yml"), "new".to_string());
        files.insert(
            PathBuf::from("src/commands/current.yml"),
            "same".to_string(),
        );
        files.insert(
            PathBuf::from("src/commands/fresh.yml"),
            "brand new".to_string(),
        );
        files
    }

    /// The four counters are what `verify_no_drift` gates orb publishing on, so
    /// they have to be right when several outcomes occur in the same run — the
    /// case a single-outcome test cannot catch.
    #[test]
    fn every_outcome_is_counted_in_one_run() {
        let dir = TempDir::new().unwrap();
        let files = mixed_tree(&dir);
        let report = write_tree(dir.path(), &files, &[], false).unwrap();
        assert_eq!(
            report,
            WriteReport {
                created: 1,
                updated: 1,
                unchanged: 1,
                removed: 1,
            }
        );
    }

    /// The report must not depend on whether anything was written. `--check`
    /// and `--dry-run` both reach `write_tree` with `dry_run = true`, and
    /// `--check` turns the result straight into the pass/fail that gates
    /// publishing — so a report that differed between the modes would gate on a
    /// number no real write ever produced.
    #[test]
    fn a_dry_run_reports_exactly_what_a_real_write_does() {
        let wet = TempDir::new().unwrap();
        let dry = TempDir::new().unwrap();
        let wet_files = mixed_tree(&wet);
        let dry_files = mixed_tree(&dry);

        let wet_report = write_tree(wet.path(), &wet_files, &[], false).unwrap();
        let dry_report = write_tree(dry.path(), &dry_files, &[], true).unwrap();

        assert_eq!(
            dry_report, wet_report,
            "a dry run must produce the same report as the write it previews"
        );
    }

    /// …and it must leave the tree exactly as it found it.
    #[test]
    fn a_dry_run_touches_nothing_at_all() {
        let dir = TempDir::new().unwrap();
        let files = mixed_tree(&dir);
        let cmds = dir.path().join("src/commands");

        write_tree(dir.path(), &files, &[], true).unwrap();

        assert_eq!(fs::read_to_string(cmds.join("stale.yml")).unwrap(), "old");
        assert_eq!(
            fs::read_to_string(cmds.join("current.yml")).unwrap(),
            "same"
        );
        assert!(cmds.join("orphan.yml").exists(), "no pruning in a dry run");
        assert!(!cmds.join("fresh.yml").exists(), "no creation in a dry run");
    }

    /// The counts have to describe the filesystem, not merely each other.
    #[test]
    fn the_counts_match_what_actually_happened_on_disk() {
        let dir = TempDir::new().unwrap();
        let files = mixed_tree(&dir);
        let cmds = dir.path().join("src/commands");

        let report = write_tree(dir.path(), &files, &[], false).unwrap();

        assert_eq!(
            fs::read_to_string(cmds.join("fresh.yml")).unwrap(),
            "brand new"
        );
        assert_eq!(fs::read_to_string(cmds.join("stale.yml")).unwrap(), "new");
        assert_eq!(
            fs::read_to_string(cmds.join("current.yml")).unwrap(),
            "same"
        );
        assert!(!cmds.join("orphan.yml").exists());

        let on_disk = fs::read_dir(&cmds).unwrap().count();
        assert_eq!(
            on_disk,
            report.created + report.updated + report.unchanged,
            "every surviving file is accounted for by a counter"
        );
    }

    /// Re-running over the tree the previous run produced must report nothing
    /// to do — the property `--check` relies on to stay quiet on a clean repo.
    #[test]
    fn writing_twice_leaves_the_second_run_with_nothing_to_do() {
        let dir = TempDir::new().unwrap();
        let files = mixed_tree(&dir);
        write_tree(dir.path(), &files, &[], false).unwrap();
        let second = write_tree(dir.path(), &files, &[], false).unwrap();
        assert_eq!(
            second,
            WriteReport {
                created: 0,
                updated: 0,
                unchanged: 3,
                removed: 0,
            }
        );
    }
}
