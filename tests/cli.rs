use assert_cmd::Command;
use predicates::str::contains;

#[test]
fn version_flag_prints_version() {
    Command::cargo_bin("scissors")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(contains("scissors 0.0.1"));
}

#[test]
fn help_flag_prints_usage() {
    Command::cargo_bin("scissors")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("Usage: scissors"))
        .stdout(contains("--context"));
}

#[test]
fn no_args_prints_preview_notice_to_stderr() {
    Command::cargo_bin("scissors")
        .unwrap()
        .assert()
        .success()
        .stderr(contains("preview release"));
}

#[test]
fn unknown_flag_exits_with_error() {
    Command::cargo_bin("scissors")
        .unwrap()
        .arg("--nonexistent-flag")
        .assert()
        .failure();
}
