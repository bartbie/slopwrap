use assert_cmd::Command;
use assert_fs::prelude::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;

// cargo_bin works locally (cargo test), PATH fallback for VM test
// where pre-built test binaries run without cargo.
fn slopwrap() -> Command {
    Command::cargo_bin("slopwrap")
        .unwrap_or_else(|_| Command::new("slopwrap"))
}

#[test]
#[ignore] // requires bwrap
fn changes_land_in_overlay_not_repo() {
    let repo = assert_fs::TempDir::new().unwrap();
    repo.child("original.txt").write_str("original").unwrap();
    let overlay = assert_fs::TempDir::new().unwrap();

    slopwrap()
        .args([
            "--keep",
            "--overlay-dir",
            overlay.path().to_str().unwrap(),
            "--",
            "bash",
            "-c",
        ])
        .arg(format!(
            "echo new > {}/created.txt && echo modified > {}/original.txt",
            repo.path().display(),
            repo.path().display()
        ))
        .current_dir(repo.path())
        .assert()
        .success();

    // Real repo is untouched
    assert!(!repo.path().join("created.txt").exists());
    assert_eq!(
        fs::read_to_string(repo.path().join("original.txt")).unwrap(),
        "original"
    );

    // Overlay has the changes
    let upper = overlay.path().join("upper");
    assert!(upper.join("created.txt").exists());
    assert!(upper.join("original.txt").exists());
}

#[test]
#[ignore] // requires bwrap
fn custom_overlay_dir() {
    let repo = assert_fs::TempDir::new().unwrap();
    let overlay = assert_fs::TempDir::new().unwrap();

    slopwrap()
        .args([
            "--keep",
            "--overlay-dir",
            overlay.path().to_str().unwrap(),
            "--",
            "bash",
            "-c",
        ])
        .arg(format!(
            "echo test > {}/overlay_test.txt",
            repo.path().display()
        ))
        .current_dir(repo.path())
        .assert()
        .success();

    assert!(overlay.path().join("upper").join("overlay_test.txt").exists());
}

#[test]
#[ignore] // requires bwrap
fn overlay_workdir_is_cleanable() {
    // Overlayfs sets d--------- on its internal workdir. After slopwrap exits,
    // fix_overlay_permissions should restore permissions so cleanup works.
    let repo = assert_fs::TempDir::new().unwrap();
    let overlay = assert_fs::TempDir::new().unwrap();

    slopwrap()
        .args([
            "--keep",
            "--overlay-dir",
            overlay.path().to_str().unwrap(),
            "--",
            "true",
        ])
        .current_dir(repo.path())
        .assert()
        .success();

    // The work/ dir should be removable (permissions fixed by slopwrap)
    let work = overlay.path().join("work");
    assert!(work.exists());
    // Should not have d--------- anymore
    let mode = fs::metadata(&work).unwrap().permissions().mode();
    assert!(mode & 0o700 != 0, "work dir should be readable after cleanup, got mode {mode:#o}");
    // Actually removable
    fs::remove_dir_all(overlay.path()).expect("overlay dir should be removable after slopwrap fixes permissions");
}
