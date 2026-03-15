use assert_cmd::Command;
use assert_fs::prelude::*;
use predicates::prelude::*;

// cargo_bin works locally (cargo test), PATH fallback for VM test
// where pre-built test binaries run without cargo.
fn slopwrap() -> Command {
    Command::cargo_bin("slopwrap")
        .unwrap_or_else(|_| Command::new("slopwrap"))
}

#[test]
#[ignore] // requires bwrap
fn ls_repo_contents() {
    let repo = assert_fs::TempDir::new().unwrap();
    repo.child("hello.txt").write_str("world").unwrap();

    slopwrap()
        .args(["--keep", "--"])
        .arg("ls")
        .arg(repo.path())
        .current_dir(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("hello.txt"));
}

#[test]
#[ignore] // requires bwrap
fn touch_does_not_modify_real_repo() {
    let repo = assert_fs::TempDir::new().unwrap();
    let overlay = assert_fs::TempDir::new().unwrap();

    slopwrap()
        .args([
            "--keep",
            "--overlay-dir",
            overlay.path().to_str().unwrap(),
            "--",
            "touch",
        ])
        .arg(repo.path().join("newfile"))
        .current_dir(repo.path())
        .assert()
        .success();

    assert!(!repo.path().join("newfile").exists());
}

#[test]
#[ignore] // requires bwrap
fn rm_does_not_affect_real_repo() {
    let repo = assert_fs::TempDir::new().unwrap();
    repo.child("keep_me.txt").write_str("important").unwrap();
    let overlay = assert_fs::TempDir::new().unwrap();

    slopwrap()
        .args([
            "--keep",
            "--overlay-dir",
            overlay.path().to_str().unwrap(),
            "--",
            "rm",
        ])
        .arg(repo.path().join("keep_me.txt"))
        .current_dir(repo.path())
        .assert()
        .success();

    assert!(repo.path().join("keep_me.txt").exists());
}

#[test]
#[ignore] // requires bwrap
fn resolv_conf_accessible() {
    let repo = assert_fs::TempDir::new().unwrap();

    slopwrap()
        .args(["--keep", "--", "cat", "/etc/resolv.conf"])
        .current_dir(repo.path())
        .assert()
        .success();
}

#[test]
#[ignore] // requires bwrap
fn hostname_is_slopwrap() {
    let repo = assert_fs::TempDir::new().unwrap();

    slopwrap()
        .args(["--keep", "--", "hostname"])
        .current_dir(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("slopwrap"));
}

#[test]
#[ignore] // requires bwrap
fn no_net_blocks_network() {
    let repo = assert_fs::TempDir::new().unwrap();

    slopwrap()
        .args([
            "--keep",
            "--no-net",
            "--",
            "curl",
            "-s",
            "--max-time",
            "3",
            "https://example.com",
        ])
        .current_dir(repo.path())
        .assert()
        .failure();
}
