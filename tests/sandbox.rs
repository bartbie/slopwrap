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

#[test]
#[ignore] // requires bwrap
fn claude_flag_exposes_claude_json() {
    let repo = assert_fs::TempDir::new().unwrap();
    let home = std::env::var("HOME").unwrap();
    let claude_json = std::path::PathBuf::from(&home).join(".claude.json");

    // Skip if user doesn't have .claude.json
    if !claude_json.exists() {
        return;
    }

    slopwrap()
        .args(["--keep", "--claude", "--", "cat"])
        .arg(&claude_json)
        .current_dir(repo.path())
        .assert()
        .success();
}

#[test]
#[ignore] // requires bwrap
fn claude_flag_keeps_rest_of_config_ro() {
    let repo = assert_fs::TempDir::new().unwrap();
    let home = std::env::var("HOME").unwrap();

    // Try to write to a non-claude path under .config — should fail
    slopwrap()
        .args(["--keep", "--claude", "--", "touch"])
        .arg(format!("{home}/.config/slopwrap-test-sentinel"))
        .current_dir(repo.path())
        .assert()
        .failure();
}

#[test]
#[ignore] // requires bwrap
fn env_is_minimal() {
    let repo = assert_fs::TempDir::new().unwrap();

    slopwrap()
        .args(["--keep", "--", "bash", "-c", "env"])
        .current_dir(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("DBUS_SESSION_BUS_ADDRESS").not())
        .stdout(predicate::str::contains("WAYLAND_DISPLAY").not())
        .stdout(predicate::str::contains("XDG_RUNTIME_DIR").not());
}

#[test]
#[ignore] // requires bwrap
fn etc_passwd_readable() {
    if !std::path::Path::new("/etc/passwd").exists() {
        return;
    }
    let repo = assert_fs::TempDir::new().unwrap();

    slopwrap()
        .args(["--keep", "--", "cat", "/etc/passwd"])
        .current_dir(repo.path())
        .assert()
        .success();
}

#[test]
#[ignore] // requires bwrap
fn etc_group_readable() {
    if !std::path::Path::new("/etc/group").exists() {
        return;
    }
    let repo = assert_fs::TempDir::new().unwrap();

    slopwrap()
        .args(["--keep", "--", "cat", "/etc/group"])
        .current_dir(repo.path())
        .assert()
        .success();
}

#[test]
#[ignore] // requires bwrap
fn claude_flag_dot_claude_always_mounted() {
    let repo = assert_fs::TempDir::new().unwrap();
    let home = std::env::var("HOME").unwrap();

    slopwrap()
        .args(["--keep", "--claude", "--", "ls"])
        .arg(format!("{home}/.claude/"))
        .current_dir(repo.path())
        .assert()
        .success();
}

#[test]
#[ignore] // requires bwrap
fn claude_flag_credentials_writable() {
    let repo = assert_fs::TempDir::new().unwrap();
    let home = std::env::var("HOME").unwrap();

    slopwrap()
        .args(["--keep", "--claude", "--", "touch"])
        .arg(format!("{home}/.claude/test-write-sentinel"))
        .current_dir(repo.path())
        .assert()
        .success();
}
