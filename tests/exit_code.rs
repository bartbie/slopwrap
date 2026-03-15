use assert_cmd::Command;

fn slopwrap() -> Command {
    Command::cargo_bin("slopwrap").unwrap()
}

#[test]
#[ignore] // requires bwrap
fn exit_code_zero() {
    let repo = assert_fs::TempDir::new().unwrap();

    slopwrap()
        .args(["--keep", "--", "true"])
        .env("PWD", repo.path())
        .assert()
        .success();
}

#[test]
#[ignore] // requires bwrap
fn exit_code_nonzero() {
    let repo = assert_fs::TempDir::new().unwrap();

    slopwrap()
        .args(["--keep", "--", "bash", "-c", "exit 42"])
        .env("PWD", repo.path())
        .assert()
        .code(42);
}

#[test]
#[ignore] // requires bwrap
fn exit_code_from_false() {
    let repo = assert_fs::TempDir::new().unwrap();

    slopwrap()
        .args(["--keep", "--", "false"])
        .env("PWD", repo.path())
        .assert()
        .code(1);
}
