use assert_cmd::Command;

// cargo_bin works locally (cargo test), PATH fallback for VM test
// where pre-built test binaries run without cargo.
fn slopwrap() -> Command {
    Command::cargo_bin("slopwrap")
        .unwrap_or_else(|_| Command::new("slopwrap"))
}

#[test]
#[ignore] // requires bwrap
fn exit_code_zero() {
    let repo = assert_fs::TempDir::new().unwrap();

    slopwrap()
        .args(["--keep", "--", "true"])
        .current_dir(repo.path())
        .assert()
        .success();
}

#[test]
#[ignore] // requires bwrap
fn exit_code_nonzero() {
    let repo = assert_fs::TempDir::new().unwrap();

    slopwrap()
        .args(["--keep", "--", "bash", "-c", "exit 42"])
        .current_dir(repo.path())
        .assert()
        .code(42);
}

#[test]
#[ignore] // requires bwrap
fn exit_code_from_false() {
    let repo = assert_fs::TempDir::new().unwrap();

    slopwrap()
        .args(["--keep", "--", "false"])
        .current_dir(repo.path())
        .assert()
        .code(1);
}
