use std::path::PathBuf;

use assert_cmd::Command;
use predicates::str::contains;

fn mock_editor() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock-editor.sh")
}

fn scissors() -> Command {
    Command::cargo_bin("scissors").unwrap()
}

#[test]
fn version_flag_prints_version() {
    scissors()
        .arg("--version")
        .assert()
        .success()
        .stdout(contains("scissors 0.1.0"));
}

#[test]
fn help_flag_prints_usage() {
    scissors()
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("Usage: scissors"))
        .stdout(contains("--context"))
        .stdout(contains("--yes"));
}

#[test]
fn approve_prints_content_on_stdout_exit_0() {
    scissors()
        .env("EDITOR", mock_editor())
        .env("MOCK_EDITOR_ACTION", "approve")
        .write_stdin("original draft")
        .assert()
        .success()
        .stdout(contains("approved content"));
}

#[test]
fn edit_keeps_content_above_scissors() {
    scissors()
        .env("EDITOR", mock_editor())
        .env("MOCK_EDITOR_ACTION", "edit")
        .write_stdin("body text")
        .assert()
        .success()
        .stdout(contains("edited line"))
        .stdout(contains("body text"));
}

#[test]
fn abort_exits_1_and_preserves_draft() {
    scissors()
        .env("EDITOR", mock_editor())
        .env("MOCK_EDITOR_ACTION", "abort")
        .write_stdin("will be emptied")
        .assert()
        .code(1)
        .stderr(contains("aborted"))
        .stderr(contains("draft preserved at"));
}

#[test]
fn silent_failure_exits_2() {
    // noop editor returns immediately without changing the file.
    scissors()
        .env("EDITOR", mock_editor())
        .env("MOCK_EDITOR_ACTION", "noop")
        .write_stdin("unchanged draft")
        .assert()
        .code(2)
        .stderr(contains("without changes"));
}

#[test]
fn editor_failure_exits_2() {
    scissors()
        .env("EDITOR", mock_editor())
        .env("MOCK_EDITOR_ACTION", "approve")
        .env("MOCK_EDITOR_EXIT", "3")
        .write_stdin("draft")
        .assert()
        .code(2)
        .stderr(contains("editor exited with code 3"));
}

#[test]
fn missing_editor_exits_2_with_yes_hint() {
    scissors()
        .env("EDITOR", "/nonexistent/editor-binary-xyz")
        .env_remove("VISUAL")
        .write_stdin("draft")
        .assert()
        .code(2)
        .stderr(contains("no editor available"))
        .stderr(contains("--yes"));
}

#[test]
fn yes_flag_passes_through_without_editor() {
    // --yes must work even with no usable editor: it never launches one.
    scissors()
        .arg("--yes")
        .env("EDITOR", "/nonexistent/editor-binary-xyz")
        .env_remove("VISUAL")
        .write_stdin("approved as-is")
        .assert()
        .success()
        .stdout(contains("approved as-is"));
}
