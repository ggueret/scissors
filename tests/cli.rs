use std::fs;
use std::path::PathBuf;

use assert_cmd::Command;
use predicates::str::contains;
use predicates::str::is_empty;

fn mock_editor() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock-editor.sh")
}

fn scissors() -> Command {
    let mut cmd = Command::cargo_bin("scissors").unwrap();
    // Editor-flow tests drive the editor via $EDITOR + the mock; the runner's
    // own $VISUAL (which scissors prefers) must not leak in and override it.
    cmd.env_remove("VISUAL");
    cmd
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

fn draft_file(content: &str) -> tempfile::NamedTempFile {
    let f = tempfile::Builder::new().suffix(".md").tempfile().unwrap();
    fs::write(f.path(), content).unwrap();
    f
}

#[test]
fn in_place_approve_writes_file_exit_0_no_stdout() {
    let f = draft_file("original draft");
    scissors()
        .arg(f.path())
        .env("EDITOR", mock_editor())
        .env("MOCK_EDITOR_ACTION", "approve")
        .assert()
        .success()
        .stdout(is_empty());
    assert_eq!(
        fs::read_to_string(f.path()).unwrap().trim_end(),
        "approved content"
    );
}

#[test]
fn in_place_edit_keeps_content_above_scissors() {
    let f = draft_file("body text");
    scissors()
        .arg(f.path())
        .env("EDITOR", mock_editor())
        .env("MOCK_EDITOR_ACTION", "edit")
        .assert()
        .success()
        .stdout(is_empty());
    let out = fs::read_to_string(f.path()).unwrap();
    assert!(out.contains("edited line"), "got: {out:?}");
    assert!(out.contains("body text"), "got: {out:?}");
    assert!(!out.contains(">8"), "footer should be stripped: {out:?}");
}

#[test]
fn in_place_abort_leaves_file_unchanged_exit_1() {
    let f = draft_file("keep me");
    scissors()
        .arg(f.path())
        .env("EDITOR", mock_editor())
        .env("MOCK_EDITOR_ACTION", "abort")
        .assert()
        .code(1)
        .stderr(contains("left unchanged"))
        .stderr(contains(f.path().to_str().unwrap()));
    assert_eq!(fs::read_to_string(f.path()).unwrap(), "keep me");
}

#[test]
fn in_place_silent_failure_leaves_file_unchanged_exit_2() {
    let f = draft_file("untouched");
    scissors()
        .arg(f.path())
        .env("EDITOR", mock_editor())
        .env("MOCK_EDITOR_ACTION", "noop")
        .assert()
        .code(2)
        .stderr(contains("without changes"));
    assert_eq!(fs::read_to_string(f.path()).unwrap(), "untouched");
}

#[test]
fn in_place_editor_failure_leaves_file_unchanged_exit_2() {
    let f = draft_file("safe");
    scissors()
        .arg(f.path())
        .env("EDITOR", mock_editor())
        .env("MOCK_EDITOR_ACTION", "approve")
        .env("MOCK_EDITOR_EXIT", "3")
        .assert()
        .code(2)
        .stderr(contains("editor exited with code 3"));
    assert_eq!(fs::read_to_string(f.path()).unwrap(), "safe");
}

#[test]
fn dash_file_reads_stdin_like_no_arg() {
    // `-` is the conventional stdin sentinel: route to stdin/stdout, not
    // in-place on a file literally named "-".
    scissors()
        .arg("-")
        .env("EDITOR", mock_editor())
        .env("MOCK_EDITOR_ACTION", "approve")
        .write_stdin("from stdin via dash")
        .assert()
        .success()
        .stdout(contains("approved content"));
}

#[test]
fn in_place_missing_editor_restores_original_exit_2() {
    let f = draft_file("keep me");
    scissors()
        .arg(f.path())
        .env("EDITOR", "/nonexistent/editor-binary-xyz")
        .env_remove("VISUAL")
        .assert()
        .code(2)
        .stderr(contains("no editor available"));
    assert_eq!(fs::read_to_string(f.path()).unwrap(), "keep me");
}

#[test]
fn yes_in_place_is_noop_exit_0() {
    let f = draft_file("as is");
    scissors()
        .arg("--yes")
        .arg(f.path())
        .env("EDITOR", "/nonexistent/editor-binary-xyz")
        .env_remove("VISUAL")
        .assert()
        .success()
        .stdout(is_empty());
    assert_eq!(fs::read_to_string(f.path()).unwrap(), "as is");
}

#[test]
fn yes_in_place_missing_file_exits_2() {
    // No draft_file: use a path that does not exist.
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope.md");
    scissors()
        .arg("--yes")
        .arg(&missing)
        .env_remove("VISUAL")
        .assert()
        .code(2)
        .stderr(contains("cannot read"));
}

#[test]
fn yes_in_place_empty_file_exits_1() {
    let f = draft_file("");
    scissors()
        .arg("--yes")
        .arg(f.path())
        .env_remove("VISUAL")
        .assert()
        .code(1)
        .stderr(contains("empty"));
}

#[test]
fn yes_in_place_whitespace_only_file_exits_1() {
    let f = draft_file("   \n");
    scissors()
        .arg("--yes")
        .arg(f.path())
        .env_remove("VISUAL")
        .assert()
        .code(1)
        .stderr(contains("empty"));
}

#[test]
fn in_place_nonexistent_file_exits_2_with_path() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope.md");
    scissors()
        .arg(&missing)
        .env("EDITOR", mock_editor())
        .env("MOCK_EDITOR_ACTION", "approve")
        .assert()
        .code(2)
        .stderr(contains("cannot read"));
}

#[test]
fn in_place_prints_sidecar_path_on_stderr() {
    let f = draft_file("hello");
    scissors()
        .arg(f.path())
        .env("EDITOR", mock_editor())
        .env("MOCK_EDITOR_ACTION", "approve")
        .assert()
        .success()
        .stderr(contains("editing"))
        .stderr(contains(".scissors-"));
}

#[test]
fn in_place_edits_commit_editmsg_with_hash_footer() {
    // Even a .md target is edited in a COMMIT_EDITMSG buffer (uniform footer).
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("note.md");
    fs::write(&target, "body text").unwrap();
    let dump = dir.path().join("dump.txt");
    scissors()
        .arg(&target)
        .env("EDITOR", mock_editor())
        .env("MOCK_EDITOR_ACTION", "approve")
        .env("MOCK_EDITOR_DUMP", &dump)
        .assert()
        .success()
        .stderr(contains("COMMIT_EDITMSG"));
    let buf = fs::read_to_string(&dump).unwrap();
    assert!(buf.contains("\n# "), "footer must be hash comments: {buf}");
    assert!(buf.contains(">8"), "marker present: {buf}");
    assert_eq!(
        fs::read_to_string(&target).unwrap().trim_end(),
        "approved content"
    );
}

#[test]
fn stdin_edits_with_hash_footer() {
    let dir = tempfile::tempdir().unwrap();
    let dump = dir.path().join("dump.txt");
    scissors()
        .env("EDITOR", mock_editor())
        .env("MOCK_EDITOR_ACTION", "approve")
        .env("MOCK_EDITOR_DUMP", &dump)
        .write_stdin("hello")
        .assert()
        .success()
        .stdout(contains("approved content"));
    let buf = fs::read_to_string(&dump).unwrap();
    assert!(buf.contains("\n# "), "footer must be hash comments: {buf}");
}
