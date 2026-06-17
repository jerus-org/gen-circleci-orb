use anyhow::Result;
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

        let existing = if abs_path.exists() {
            Some(fs::read_to_string(&abs_path)?)
        } else {
            None
        };

        match &existing {
            Some(current) if current == content => {
                report.unchanged += 1;
            }
            Some(_) => {
                if dry_run {
                    eprintln!("[dry-run] would update: {}", rel_path.display());
                } else {
                    if let Some(parent) = abs_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::write(&abs_path, content)?;
                }
                report.updated += 1;
            }
            None => {
                if dry_run {
                    eprintln!("[dry-run] would create: {}", rel_path.display());
                } else {
                    if let Some(parent) = abs_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::write(&abs_path, content)?;
                }
                report.created += 1;
            }
        }
    }

    prune_orphans(root, files, custom_files, dry_run, &mut report)?;

    Ok(report)
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
    // Keep set: everything generated this run, plus the hand-authored files the
    // config authorises. Anything else in the owned dirs is an orphan.
    let mut keep: HashSet<PathBuf> = files.keys().cloned().collect();
    keep.extend(custom_files.iter().map(PathBuf::from));

    for dir in GENERATOR_OWNED_DIRS {
        let abs_dir = root.join(dir);
        if !abs_dir.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&abs_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let rel = Path::new(dir).join(entry.file_name());
            if keep.contains(&rel) {
                continue;
            }
            if dry_run {
                eprintln!("[dry-run] would remove: {}", rel.display());
            } else {
                fs::remove_file(entry.path())?;
            }
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
}
