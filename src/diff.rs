use anyhow::Result;
use std::path::Path;
use std::process::Command;

use crate::overlay::{Change, categorize_changes};

/// Summary of changes found in the overlay.
pub struct DiffSummary {
    pub changes: Vec<Change>,
}

impl DiffSummary {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn added(&self) -> Vec<&Path> {
        self.changes
            .iter()
            .filter_map(|c| match c {
                Change::Added(p) => Some(p.as_path()),
                _ => None,
            })
            .collect()
    }

    pub fn modified(&self) -> Vec<&Path> {
        self.changes
            .iter()
            .filter_map(|c| match c {
                Change::Modified(p) => Some(p.as_path()),
                _ => None,
            })
            .collect()
    }

    pub fn deleted(&self) -> Vec<&Path> {
        self.changes
            .iter()
            .filter_map(|c| match c {
                Change::Deleted(p) => Some(p.as_path()),
                _ => None,
            })
            .collect()
    }
}

/// Generate a diff summary by walking the upperdir.
pub fn summarize(upperdir: &Path, repo: &Path) -> Result<DiffSummary> {
    let changes = categorize_changes(upperdir, repo)?;
    Ok(DiffSummary { changes })
}

/// Print a human-readable summary of changes.
pub fn print_summary(summary: &DiffSummary) {
    if summary.is_empty() {
        println!("No changes.");
        return;
    }

    for change in &summary.changes {
        match change {
            Change::Added(p) => println!("  A  {}", p.display()),
            Change::Modified(p) => println!("  M  {}", p.display()),
            Change::Deleted(p) => println!("  D  {}", p.display()),
        }
    }

    let (a, m, d) = (
        summary.added().len(),
        summary.modified().len(),
        summary.deleted().len(),
    );
    println!(
        "\n{} added, {} modified, {} deleted",
        a, m, d
    );
}

/// Show unified diff for modified files by running `diff -u`.
pub fn show_diff(upperdir: &Path, repo: &Path, summary: &DiffSummary) -> Result<()> {
    for change in &summary.changes {
        match change {
            Change::Modified(rel) => {
                let repo_file = repo.join(rel);
                let upper_file = upperdir.join(rel);
                let output = Command::new("diff")
                    .args(["-u", "--color=auto"])
                    .arg(&repo_file)
                    .arg(&upper_file)
                    .output()?;
                // diff exits 1 when files differ — that's expected
                print!("{}", String::from_utf8_lossy(&output.stdout));
            }
            Change::Added(rel) => {
                let upper_file = upperdir.join(rel);
                let output = Command::new("diff")
                    .args(["-u", "--color=auto", "/dev/null"])
                    .arg(&upper_file)
                    .output()?;
                print!("{}", String::from_utf8_lossy(&output.stdout));
            }
            Change::Deleted(rel) => {
                let repo_file = repo.join(rel);
                if repo_file.is_file() {
                    let output = Command::new("diff")
                        .args(["-u", "--color=auto"])
                        .arg(&repo_file)
                        .arg("/dev/null")
                        .output()?;
                    print!("{}", String::from_utf8_lossy(&output.stdout));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn empty_upperdir_empty_summary() {
        let upper = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let summary = summarize(upper.path(), repo.path()).unwrap();
        assert!(summary.is_empty());
        assert!(summary.added().is_empty());
        assert!(summary.modified().is_empty());
        assert!(summary.deleted().is_empty());
    }

    #[test]
    fn added_file_categorized() {
        let upper = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        fs::write(upper.path().join("new.txt"), "data").unwrap();
        let summary = summarize(upper.path(), repo.path()).unwrap();
        assert_eq!(summary.added(), vec![Path::new("new.txt")]);
        assert!(summary.modified().is_empty());
        assert!(summary.deleted().is_empty());
    }

    #[test]
    fn modified_file_categorized() {
        let upper = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        fs::write(repo.path().join("f.txt"), "old").unwrap();
        fs::write(upper.path().join("f.txt"), "new").unwrap();
        let summary = summarize(upper.path(), repo.path()).unwrap();
        assert!(summary.added().is_empty());
        assert_eq!(summary.modified(), vec![Path::new("f.txt")]);
        assert!(summary.deleted().is_empty());
    }

    #[test]
    fn mixed_changes_all_categorized() {
        let upper = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();

        // Added
        fs::write(upper.path().join("added.txt"), "new").unwrap();
        // Modified
        fs::write(repo.path().join("mod.txt"), "old").unwrap();
        fs::write(upper.path().join("mod.txt"), "new").unwrap();
        // We can't easily create whiteouts without privileges,
        // so just test added + modified here.

        let summary = summarize(upper.path(), repo.path()).unwrap();
        assert_eq!(summary.added(), vec![Path::new("added.txt")]);
        assert_eq!(summary.modified(), vec![Path::new("mod.txt")]);
    }

    #[test]
    fn counts_are_correct() {
        let upper = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        fs::write(upper.path().join("a.txt"), "a").unwrap();
        fs::write(upper.path().join("b.txt"), "b").unwrap();
        fs::write(repo.path().join("c.txt"), "old").unwrap();
        fs::write(upper.path().join("c.txt"), "new").unwrap();

        let summary = summarize(upper.path(), repo.path()).unwrap();
        assert_eq!(summary.added().len(), 2);
        assert_eq!(summary.modified().len(), 1);
        assert_eq!(summary.deleted().len(), 0);
        assert_eq!(summary.changes.len(), 3);
    }

    // --- Property-based tests ---

    fn arb_filename(u: &mut arbtest::arbitrary::Unstructured) -> arbtest::arbitrary::Result<String> {
        let len: usize = u.int_in_range(1..=8)?;
        let mut name = String::with_capacity(len);
        for _ in 0..len {
            let c = u.int_in_range(b'a'..=b'z')?;
            name.push(c as char);
        }
        name.push_str(".txt");
        Ok(name)
    }

    fn arb_file_tree(
        u: &mut arbtest::arbitrary::Unstructured,
    ) -> arbtest::arbitrary::Result<Vec<(String, String)>> {
        let count: usize = u.int_in_range(0..=6)?;
        let mut files = Vec::new();
        for _ in 0..count {
            let name = arb_filename(u)?;
            let content_len: usize = u.int_in_range(0..=16)?;
            let mut content = String::with_capacity(content_len);
            for _ in 0..content_len {
                let c = u.int_in_range(b'a'..=b'z')?;
                content.push(c as char);
            }
            files.push((name, content));
        }
        Ok(files)
    }

    fn write_tree(base: &Path, files: &[(String, String)]) {
        for (path, content) in files {
            fs::write(base.join(path), content).unwrap();
        }
    }

    #[test]
    fn prop_every_upper_file_categorized_exactly_once() {
        arbtest::arbtest(|u| {
            let repo_files = arb_file_tree(u)?;
            let upper_files = arb_file_tree(u)?;

            let repo_dir = tempfile::tempdir().unwrap();
            let upper_dir = tempfile::tempdir().unwrap();

            write_tree(repo_dir.path(), &repo_files);
            write_tree(upper_dir.path(), &upper_files);

            let summary = summarize(upper_dir.path(), repo_dir.path()).unwrap();

            // Every file in upper should appear exactly once
            let upper_names: std::collections::HashSet<&str> =
                upper_files.iter().map(|(n, _): &(String, String)| n.as_str()).collect();

            for name in &upper_names {
                let p = Path::new(name);
                let count = summary
                    .changes
                    .iter()
                    .filter(|c| match c {
                        Change::Added(pp) | Change::Modified(pp) | Change::Deleted(pp) => pp == p,
                    })
                    .count();
                assert_eq!(count, 1, "file {name} should appear exactly once, got {count}");
            }

            Ok(())
        });
    }

    #[test]
    fn prop_repo_only_files_never_reported() {
        arbtest::arbtest(|u| {
            let repo_files = arb_file_tree(u)?;
            let upper_files = arb_file_tree(u)?;

            let repo_dir = tempfile::tempdir().unwrap();
            let upper_dir = tempfile::tempdir().unwrap();

            write_tree(repo_dir.path(), &repo_files);
            write_tree(upper_dir.path(), &upper_files);

            let summary = summarize(upper_dir.path(), repo_dir.path()).unwrap();

            let upper_names: std::collections::HashSet<&str> =
                upper_files.iter().map(|(n, _): &(String, String)| n.as_str()).collect();

            for change in &summary.changes {
                let p = match change {
                    Change::Added(p) | Change::Modified(p) | Change::Deleted(p) => p,
                };
                assert!(
                    upper_names.contains(p.to_str().unwrap()),
                    "repo-only file {:?} should not appear in changes",
                    p
                );
            }

            Ok(())
        });
    }

    #[test]
    fn prop_counts_sum_to_total() {
        arbtest::arbtest(|u| {
            let repo_files = arb_file_tree(u)?;
            let upper_files = arb_file_tree(u)?;

            let repo_dir = tempfile::tempdir().unwrap();
            let upper_dir = tempfile::tempdir().unwrap();

            write_tree(repo_dir.path(), &repo_files);
            write_tree(upper_dir.path(), &upper_files);

            let summary = summarize(upper_dir.path(), repo_dir.path()).unwrap();

            let total = summary.added().len() + summary.modified().len() + summary.deleted().len();
            assert_eq!(total, summary.changes.len());

            Ok(())
        });
    }
}
