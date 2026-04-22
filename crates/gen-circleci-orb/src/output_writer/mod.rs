use anyhow::Result;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Summary of what the writer did.
#[derive(Debug, Default, PartialEq)]
pub struct WriteReport {
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
}

/// Write `files` (relative path → content) under `root`.
///
/// Diff-aware: files whose content is identical to what's on disk are skipped.
/// When `dry_run` is true nothing is written; instead a summary is printed to stderr.
pub fn write_tree(
    root: &Path,
    files: &HashMap<PathBuf, String>,
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

    Ok(report)
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
        let report = write_tree(dir.path(), &files, false).unwrap();
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
        let report = write_tree(dir.path(), &files, false).unwrap();
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
        let report = write_tree(dir.path(), &files, false).unwrap();
        assert_eq!(report.updated, 1);
        assert_eq!(
            fs::read_to_string(dir.path().join("src/foo.yml")).unwrap(),
            "new content"
        );
    }

    #[test]
    fn dry_run_writes_nothing() {
        let dir = TempDir::new().unwrap();
        let files = single_file("src/foo.yml", "hello");
        let report = write_tree(dir.path(), &files, true).unwrap();
        assert_eq!(report.created, 1, "dry_run should still count created");
        assert!(
            !dir.path().join("src/foo.yml").exists(),
            "dry_run must not write files"
        );
    }
}
