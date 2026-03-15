use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Overlay directory layout.
pub struct OverlayDirs {
    pub upperdir: PathBuf,
    pub workdir: PathBuf,
}

/// Create overlay directories (upperdir and workdir) under `base`.
pub fn setup(base: &Path) -> Result<OverlayDirs> {
    let upperdir = base.join("upper");
    let workdir = base.join("work");
    fs::create_dir_all(&upperdir).context("creating upperdir")?;
    fs::create_dir_all(&workdir).context("creating workdir")?;
    Ok(OverlayDirs { upperdir, workdir })
}

/// Check if a file is an overlayfs whiteout (char device 0,0).
pub fn is_whiteout(path: &Path) -> bool {
    use nix::sys::stat::{SFlag, stat};
    use std::os::unix::fs::{FileTypeExt, MetadataExt};

    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };

    // Check if it's a character device
    let ft = meta.file_type();
    if !ft.is_char_device() {
        // nix::sys::stat can also detect via S_IFCHR
        let Ok(st) = stat(path) else {
            return false;
        };
        if st.st_mode & SFlag::S_IFMT.bits() != SFlag::S_IFCHR.bits() {
            return false;
        }
    }

    // Check major:minor == 0:0
    let rdev = meta.rdev();
    nix::sys::stat::major(rdev) == 0 && nix::sys::stat::minor(rdev) == 0
}

/// Check if a directory is an opaque directory (has trusted.overlay.opaque xattr).
pub fn is_opaque_dir(path: &Path) -> bool {
    #[cfg(target_os = "linux")]
    {
        use std::ffi::CString;
        let Ok(cpath) = CString::new(path.as_os_str().as_encoded_bytes()) else {
            return false;
        };
        let attr_name = c"trusted.overlay.opaque";
        let mut buf = [0u8; 2];
        let ret = unsafe {
            libc::lgetxattr(
                cpath.as_ptr(),
                attr_name.as_ptr(),
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
            )
        };
        ret == 1 && buf[0] == b'y'
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        false
    }
}

/// Represents a change found in the overlay upperdir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Added(PathBuf),
    Modified(PathBuf),
    Deleted(PathBuf),
}

/// Categorize all changes in the upperdir relative to the repo.
pub fn categorize_changes(upperdir: &Path, repo: &Path) -> Result<Vec<Change>> {
    let mut changes = Vec::new();

    for entry in WalkDir::new(upperdir) {
        let entry = entry?;
        if entry.path() == upperdir {
            continue;
        }

        let rel = entry
            .path()
            .strip_prefix(upperdir)
            .context("stripping upperdir prefix")?;

        let repo_path = repo.join(rel);

        if is_whiteout(entry.path()) {
            changes.push(Change::Deleted(rel.to_path_buf()));
        } else if entry.file_type().is_dir() {
            // Skip directories themselves — we care about files.
            // But handle opaque dirs as deletions of the original.
            if is_opaque_dir(entry.path()) && repo_path.exists() {
                changes.push(Change::Deleted(rel.to_path_buf()));
                changes.push(Change::Added(rel.to_path_buf()));
            }
            continue;
        } else if repo_path.exists() {
            changes.push(Change::Modified(rel.to_path_buf()));
        } else {
            changes.push(Change::Added(rel.to_path_buf()));
        }
    }

    changes.sort_by(|a, b| {
        let path_a = match a {
            Change::Added(p) | Change::Modified(p) | Change::Deleted(p) => p,
        };
        let path_b = match b {
            Change::Added(p) | Change::Modified(p) | Change::Deleted(p) => p,
        };
        path_a.cmp(path_b)
    });

    Ok(changes)
}

/// Apply overlay changes to the real repo.
pub fn apply(upperdir: &Path, repo: &Path) -> Result<()> {
    for entry in WalkDir::new(upperdir) {
        let entry = entry?;
        if entry.path() == upperdir {
            continue;
        }

        let rel = entry
            .path()
            .strip_prefix(upperdir)
            .context("stripping upperdir prefix")?;
        let target = repo.join(rel);

        if is_whiteout(entry.path()) {
            // Delete the corresponding file/dir in the repo
            if target.is_dir() {
                fs::remove_dir_all(&target)
                    .with_context(|| format!("removing dir {}", target.display()))?;
            } else if target.exists() {
                fs::remove_file(&target)
                    .with_context(|| format!("removing file {}", target.display()))?;
            }
        } else if entry.file_type().is_dir() {
            if is_opaque_dir(entry.path()) {
                // Replace entire directory
                if target.exists() {
                    fs::remove_dir_all(&target)?;
                }
                fs::create_dir_all(&target)?;
            } else {
                fs::create_dir_all(&target)?;
            }
        } else {
            // Regular file — copy
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), &target)
                .with_context(|| format!("copying {} -> {}", entry.path().display(), target.display()))?;
        }
    }
    Ok(())
}

/// Discard overlay by removing the base directory.
pub fn discard(base: &Path) -> Result<()> {
    fs::remove_dir_all(base).context("removing overlay directory")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn setup_creates_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = setup(tmp.path()).unwrap();
        assert!(dirs.upperdir.is_dir());
        assert!(dirs.workdir.is_dir());
    }

    #[test]
    fn categorize_empty_upperdir_no_changes() {
        let upper = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let changes = categorize_changes(upper.path(), repo.path()).unwrap();
        assert!(changes.is_empty());
    }

    #[test]
    fn categorize_added_file() {
        let upper = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        fs::write(upper.path().join("newfile.txt"), "hello").unwrap();
        let changes = categorize_changes(upper.path(), repo.path()).unwrap();
        assert_eq!(changes, vec![Change::Added(PathBuf::from("newfile.txt"))]);
    }

    #[test]
    fn categorize_modified_file() {
        let upper = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        fs::write(repo.path().join("existing.txt"), "old").unwrap();
        fs::write(upper.path().join("existing.txt"), "new").unwrap();
        let changes = categorize_changes(upper.path(), repo.path()).unwrap();
        assert_eq!(
            changes,
            vec![Change::Modified(PathBuf::from("existing.txt"))]
        );
    }

    #[test]
    fn apply_new_file() {
        let upper = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        fs::write(upper.path().join("new.txt"), "content").unwrap();
        apply(upper.path(), repo.path()).unwrap();
        assert_eq!(fs::read_to_string(repo.path().join("new.txt")).unwrap(), "content");
    }

    #[test]
    fn apply_modified_file() {
        let upper = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        fs::write(repo.path().join("f.txt"), "old").unwrap();
        fs::write(upper.path().join("f.txt"), "new").unwrap();
        apply(upper.path(), repo.path()).unwrap();
        assert_eq!(fs::read_to_string(repo.path().join("f.txt")).unwrap(), "new");
    }

    #[test]
    fn apply_nested_file() {
        let upper = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        fs::create_dir_all(upper.path().join("a/b")).unwrap();
        fs::write(upper.path().join("a/b/c.txt"), "deep").unwrap();
        apply(upper.path(), repo.path()).unwrap();
        assert_eq!(
            fs::read_to_string(repo.path().join("a/b/c.txt")).unwrap(),
            "deep"
        );
    }

    #[test]
    fn discard_removes_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("overlay");
        fs::create_dir_all(&base).unwrap();
        fs::write(base.join("file"), "data").unwrap();
        discard(&base).unwrap();
        assert!(!base.exists());
    }

    #[test]
    fn apply_preserves_untouched_files() {
        let upper = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        fs::write(repo.path().join("untouched.txt"), "original").unwrap();
        fs::write(upper.path().join("other.txt"), "new").unwrap();
        apply(upper.path(), repo.path()).unwrap();
        assert_eq!(
            fs::read_to_string(repo.path().join("untouched.txt")).unwrap(),
            "original"
        );
    }

    // --- Property-based tests ---

    /// Generate a safe filename component from arbitrary bytes.
    fn arb_filename(u: &mut arbtest::arbitrary::Unstructured) -> arbtest::arbitrary::Result<String> {
        let len: usize = u.int_in_range(1..=12)?;
        let mut name = String::with_capacity(len);
        for _ in 0..len {
            let c = u.int_in_range(b'a'..=b'z')?;
            name.push(c as char);
        }
        name.push_str(".txt");
        Ok(name)
    }

    /// Generate an arbitrary file tree: Vec<(relative_path, contents)>.
    /// Generate arbitrary file tree. Uses separate alphabets for dirs vs files
    /// to avoid a name being both a directory and a leaf file.
    fn arb_file_tree(
        u: &mut arbtest::arbitrary::Unstructured,
    ) -> arbtest::arbitrary::Result<Vec<(String, String)>> {
        let count: usize = u.int_in_range(0..=8)?;
        let mut files = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..count {
            let depth: usize = u.int_in_range(0..=2)?;
            let mut path_parts = Vec::new();
            for _ in 0..depth {
                // Directory names: use "d_" prefix to avoid collisions with file names
                let len: usize = u.int_in_range(1..=4)?;
                let mut dir = String::from("d_");
                for _ in 0..len {
                    let c = u.int_in_range(b'a'..=b'z')?;
                    dir.push(c as char);
                }
                path_parts.push(dir);
            }
            path_parts.push(arb_filename(u)?);
            let path = path_parts.join("/");

            // Skip duplicate paths
            if !seen.insert(path.clone()) {
                continue;
            }

            let content_len: usize = u.int_in_range(0..=32)?;
            let mut content = String::with_capacity(content_len);
            for _ in 0..content_len {
                let c = u.int_in_range(b'a'..=b'z')?;
                content.push(c as char);
            }
            files.push((path, content));
        }
        Ok(files)
    }

    fn write_tree(base: &Path, files: &[(String, String)]) {
        for (path, content) in files {
            let full = base.join(path);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&full, content).unwrap();
        }
    }

    fn collect_tree(base: &Path) -> std::collections::BTreeMap<PathBuf, String> {
        let mut map = std::collections::BTreeMap::new();
        for entry in WalkDir::new(base) {
            let entry = entry.unwrap();
            if entry.file_type().is_file() {
                let rel = entry.path().strip_prefix(base).unwrap().to_path_buf();
                let content = fs::read_to_string(entry.path()).unwrap();
                map.insert(rel, content);
            }
        }
        map
    }

    #[test]
    fn prop_apply_produces_merged_state() {
        arbtest::arbtest(|u| {
            let repo_files = arb_file_tree(u)?;
            let upper_files = arb_file_tree(u)?;

            let repo_dir = tempfile::tempdir().unwrap();
            let upper_dir = tempfile::tempdir().unwrap();

            write_tree(repo_dir.path(), &repo_files);
            write_tree(upper_dir.path(), &upper_files);

            // Compute expected state: repo files overwritten by upper files
            let mut expected: std::collections::BTreeMap<PathBuf, String> =
                std::collections::BTreeMap::new();
            for (p, c) in &repo_files {
                expected.insert(PathBuf::from(p), String::clone(c));
            }
            for (p, c) in &upper_files {
                expected.insert(PathBuf::from(p), String::clone(c));
            }

            apply(upper_dir.path(), repo_dir.path()).unwrap();

            let actual = collect_tree(repo_dir.path());
            assert_eq!(actual, expected);

            Ok(())
        });
    }

    #[test]
    fn prop_apply_then_no_changes() {
        arbtest::arbtest(|u| {
            let repo_files = arb_file_tree(u)?;
            let upper_files = arb_file_tree(u)?;

            let repo_dir = tempfile::tempdir().unwrap();
            let upper_dir = tempfile::tempdir().unwrap();

            write_tree(repo_dir.path(), &repo_files);
            write_tree(upper_dir.path(), &upper_files);

            apply(upper_dir.path(), repo_dir.path()).unwrap();

            // After applying, the upper should have no "new" changes vs repo
            // (all upper files are now in repo with same content)
            let changes = categorize_changes(upper_dir.path(), repo_dir.path()).unwrap();
            let _non_added: Vec<_> = changes
                .iter()
                .filter(|c| !matches!(c, Change::Added(_)))
                .collect();
            // All remaining changes should be Modified (same content) — but
            // categorize_changes checks existence not content, so modified is expected.
            // The key invariant: no Added files remain.
            for c in &changes {
                match c {
                    Change::Added(p) => {
                        panic!("file {p:?} should exist in repo after apply");
                    }
                    Change::Deleted(_) => {
                        panic!("no whiteouts were created");
                    }
                    Change::Modified(_) => {} // expected — upper still has the files
                }
            }

            Ok(())
        });
    }

    #[test]
    fn prop_untouched_files_preserved() {
        arbtest::arbtest(|u| {
            let repo_files = arb_file_tree(u)?;
            let upper_files = arb_file_tree(u)?;

            let repo_dir = tempfile::tempdir().unwrap();
            let upper_dir = tempfile::tempdir().unwrap();

            write_tree(repo_dir.path(), &repo_files);
            let before = collect_tree(repo_dir.path());

            write_tree(upper_dir.path(), &upper_files);
            let upper_paths: std::collections::HashSet<PathBuf> = upper_files
                .iter()
                .map(|(p, _)| PathBuf::from(p))
                .collect();

            apply(upper_dir.path(), repo_dir.path()).unwrap();
            let after = collect_tree(repo_dir.path());

            // Files not in upper should be byte-identical
            for (path, content) in &before {
                if !upper_paths.contains(path) {
                    assert_eq!(
                        after.get(path),
                        Some(content),
                        "untouched file {path:?} was modified"
                    );
                }
            }

            Ok(())
        });
    }
}
