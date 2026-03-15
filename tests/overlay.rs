use assert_cmd::Command;
use assert_fs::prelude::*;
use std::fs;

fn slopwrap() -> Command {
    Command::cargo_bin("slopwrap").unwrap()
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
        .env("PWD", repo.path())
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
        .env("PWD", repo.path())
        .assert()
        .success();

    assert!(overlay.path().join("upper").join("overlay_test.txt").exists());
}
