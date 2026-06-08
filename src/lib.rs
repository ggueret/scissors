//! Editor-based content approval, git-commit style.
//!
//! Opens content in the user's editor, lets them edit or approve it, and
//! returns the bytes above the scissors line. Modelled on git's
//! `commit.cleanup=scissors` convention.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::{Duration, Instant};

use tempfile::{Builder, TempDir};
use thiserror::Error;

/// The git scissors separator. `build_draft` ends the footer with this exact
/// line; `strip_scissors` cuts at its last occurrence.
pub const SCISSORS: &str = "# ------------------------ >8 ------------------------";

/// Editor returning faster than this with an unchanged file is treated as a
/// silent failure (editor never actually opened).
const MIN_ELAPSED_MS: u128 = 500;

/// Result of an approval round-trip.
#[derive(Debug)]
pub enum Outcome {
    /// User saved with non-empty content above the scissors line.
    Approved(String),
    /// User emptied the content above the scissors line. The draft file is
    /// preserved so the user can recover it.
    Aborted { draft_path: PathBuf },
}

/// Errors from [`approve_in_editor`].
#[derive(Debug, Error)]
pub enum ScissorsError {
    #[error("no editor available ($VISUAL, $EDITOR unset and editor not found)")]
    NoEditor,

    #[error("editor exited with code {code}; draft at {draft_path}")]
    EditorFailed { code: i32, draft_path: PathBuf },

    #[error(
        "editor returned in {elapsed_ms}ms without changes; it likely never \
         opened. Causes: $EDITOR missing a wait flag (e.g. `code --wait`), a \
         sandbox blocking the editor's IPC, or the binary not on PATH; \
         draft at {draft_path}"
    )]
    SilentFailure {
        elapsed_ms: u32,
        draft_path: PathBuf,
    },

    #[error("io error: {0}")]
    Io(#[from] io::Error),
}

/// Outcome of an in-place file approval. The approved content is in the file
/// itself; on abort the file is left untouched.
#[derive(Debug)]
pub enum FileOutcome {
    Approved,
    Aborted,
}

/// Errors from [`approve_file_in_place`]. On every error the target file is
/// left untouched: editing happens in a sidecar that is discarded on failure.
#[derive(Debug, Error)]
pub enum FileError {
    #[error("no editor available ($VISUAL, $EDITOR unset and editor not found)")]
    NoEditor,
    #[error("cannot read {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error(
        "{} already contains the scissors line (line {line}); refusing to edit \
         it in place to avoid discarding content below it",
        path.display()
    )]
    MarkerInInput { path: PathBuf, line: usize },
    #[error("editor exited with code {code}")]
    EditorExited { code: i32 },
    #[error(
        "editor returned in {elapsed_ms}ms without changes; it likely never \
             opened. Causes: $EDITOR missing a wait flag (e.g. `code --wait`), a \
             sandbox blocking the editor's IPC, or the binary not on PATH"
    )]
    SilentFailure { elapsed_ms: u32 },
    #[error(
        "failed to replace {}: {source}; your edited draft is kept at {}",
        target.display(),
        sidecar.display()
    )]
    Persist {
        target: PathBuf,
        sidecar: PathBuf,
        source: io::Error,
    },
    #[error("io error: {0}")]
    Io(#[from] io::Error),
}

/// Return everything above the *last* scissors line, trimmed. If there is no
/// scissors line, return the whole input trimmed. Cutting at the last occurrence
/// (the footer is always appended last) preserves a body that itself contains a
/// scissors line, instead of silently discarding everything below it.
pub fn strip_scissors(raw: &str) -> String {
    let lines: Vec<&str> = raw.lines().collect();
    let cut = lines
        .iter()
        .rposition(|l| l.trim_end() == SCISSORS)
        .unwrap_or(lines.len());
    lines[..cut].join("\n").trim_end().to_string()
}

/// Assemble the editor buffer: the content, a blank line, then the scissors
/// footer with instructions (and optional context).
pub fn build_draft(content: &str, context: Option<&str>) -> String {
    let mut out = String::new();
    out.push_str(content.trim_end_matches('\n'));
    out.push_str("\n\n");
    out.push_str(SCISSORS);
    out.push('\n');
    out.push_str("# Do not modify or remove the line above.\n");
    out.push_str("# Everything below is context and will be stripped from the final content.\n");
    out.push_str("# Save and close the editor when done.\n");
    out.push_str("# Empty all content above this line to abort without submitting.\n");
    if let Some(ctx) = context {
        out.push_str("#\n");
        out.push_str(&format!("# Context: {ctx}\n"));
    }
    out
}

/// Resolve the editor command, honouring $VISUAL > $EDITOR > `vi`.
/// Returns the command split into program + args (e.g. `["code", "--wait"]`).
pub fn resolve_editor() -> Result<Vec<String>, ScissorsError> {
    for var in ["VISUAL", "EDITOR"] {
        if let Ok(val) = std::env::var(var) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                let parts = shell_words::split(trimmed)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
                if !parts.is_empty() {
                    return Ok(parts);
                }
            }
        }
    }
    Ok(vec!["vi".to_string()])
}

/// Launch the editor on `path`, blocking until it exits. Returns the exit
/// status and how long it took. A missing editor binary maps to `NoEditor`.
fn launch_editor(cmd: &[String], path: &Path) -> Result<(ExitStatus, Duration), ScissorsError> {
    let (program, args) = cmd.split_first().ok_or(ScissorsError::NoEditor)?;
    let start = Instant::now();
    let status = Command::new(program)
        .args(args)
        .arg(path)
        .status()
        .map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => ScissorsError::NoEditor,
            _ => ScissorsError::Io(e),
        })?;
    Ok((status, start.elapsed()))
}

/// The fixed editor-buffer filename. Editors that highlight git commit messages
/// key on this exact name (no extension triggers it), so the `#` footer renders
/// as dimmed comments wherever the user's `git commit` is dimmed.
const COMMIT_EDITMSG: &str = "COMMIT_EDITMSG";

/// What an editor round-trip produced, before the draft is persisted or cleaned.
enum Verdict {
    Approved(String),
    Aborted,
    SilentFailure { elapsed_ms: u32 },
    EditorFailed { code: i32 },
}

/// Launch the editor on `path`, then classify the result against `original`.
fn edit_and_classify(
    editor: &[String],
    path: &Path,
    original: &str,
) -> Result<Verdict, ScissorsError> {
    let (status, elapsed) = launch_editor(editor, path)?;
    if !status.success() {
        return Ok(Verdict::EditorFailed {
            code: status.code().unwrap_or(-1),
        });
    }
    let raw = fs::read_to_string(path)?;
    let final_content = strip_scissors(&raw);
    if final_content == original.trim_end() && elapsed.as_millis() < MIN_ELAPSED_MS {
        return Ok(Verdict::SilentFailure {
            elapsed_ms: elapsed.as_millis() as u32,
        });
    }
    if final_content.trim().is_empty() {
        return Ok(Verdict::Aborted);
    }
    Ok(Verdict::Approved(final_content))
}

/// Persist the draft directory so the user can recover the file at `path`.
fn keep_draft(dir: TempDir, path: PathBuf) -> PathBuf {
    let _ = dir.keep();
    path
}

/// Open `content` in the user's editor and return the approved bytes.
///
/// - `Ok(Outcome::Approved(s))` -- user saved non-empty content.
/// - `Ok(Outcome::Aborted { .. })` -- user emptied the content above scissors.
/// - `Err(..)` -- no editor, editor failure, silent failure, or I/O error.
///   Aborted and the failure cases preserve the draft file for recovery.
pub fn approve_in_editor(content: &str, context: Option<&str>) -> Result<Outcome, ScissorsError> {
    let editor = resolve_editor()?;

    // Edit in a COMMIT_EDITMSG file inside a tempdir so the editor dims the footer.
    let dir = Builder::new().prefix("scissors-").tempdir()?;
    let path = dir.path().join(COMMIT_EDITMSG);
    fs::write(&path, build_draft(content, context).as_bytes())?;

    match edit_and_classify(&editor, &path, content)? {
        Verdict::Approved(approved) => Ok(Outcome::Approved(approved)),
        Verdict::Aborted => Ok(Outcome::Aborted {
            draft_path: keep_draft(dir, path),
        }),
        Verdict::SilentFailure { elapsed_ms } => Err(ScissorsError::SilentFailure {
            elapsed_ms,
            draft_path: keep_draft(dir, path),
        }),
        Verdict::EditorFailed { code } => Err(ScissorsError::EditorFailed {
            code,
            draft_path: keep_draft(dir, path),
        }),
    }
}

/// Open `path` in the user's editor, edited in place. The caller owns `path`:
/// this never creates or deletes it.
///
/// Editing happens in a `COMMIT_EDITMSG` sidecar inside a temp directory beside
/// the target (so the editor dims the footer); the target is never written until
/// the final atomic rename on approve. On abort or any error the sidecar is
/// discarded and the target is left exactly as it was.
///
/// - `Ok(FileOutcome::Approved)` -- the stripped content was atomically swapped
///   onto `path`.
/// - `Ok(FileOutcome::Aborted)` -- user emptied the content; `path` untouched.
/// - `Err(..)` -- no editor, editor failure, silent failure, or I/O error;
///   `path` untouched.
pub fn approve_file_in_place(path: &Path, context: Option<&str>) -> Result<FileOutcome, FileError> {
    let original = fs::read_to_string(path).map_err(|source| FileError::Read {
        path: path.to_path_buf(),
        source,
    })?;

    // Fail closed: a target that already holds the scissors line would let the
    // editor (or a footer deletion) leave a marker that strips away real content
    // on the permanent on-disk write. Refuse before touching anything.
    if let Some(i) = original.lines().position(|l| l.trim_end() == SCISSORS) {
        return Err(FileError::MarkerInInput {
            path: path.to_path_buf(),
            line: i + 1,
        });
    }

    // Resolve the real target (write through symlinks) so the sidecar lives on
    // the same filesystem and the rename is atomic.
    let target = fs::canonicalize(path).map_err(|source| FileError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let parent = target.parent().unwrap_or_else(|| Path::new("."));

    let editor = resolve_editor().map_err(|e| match e {
        ScissorsError::Io(io) => FileError::Io(io),
        other => FileError::Io(io::Error::other(other.to_string())),
    })?;

    // Edit in a COMMIT_EDITMSG file inside a tempdir beside the target; the target
    // is untouched until the atomic rename on approve.
    let dir = Builder::new().prefix(".scissors-").tempdir_in(parent)?;
    let sidecar = dir.path().join(COMMIT_EDITMSG);
    fs::write(&sidecar, build_draft(&original, context).as_bytes())?;
    // Preserve permissions so the replace doesn't change the file's mode.
    fs::set_permissions(&sidecar, fs::metadata(&target)?.permissions())?;

    eprintln!("scissors: editing {}", sidecar.display());

    let verdict = edit_and_classify(&editor, &sidecar, &original).map_err(|e| match e {
        ScissorsError::NoEditor => FileError::NoEditor,
        ScissorsError::Io(io) => FileError::Io(io),
        other => FileError::Io(io::Error::other(other.to_string())),
    })?;

    match verdict {
        Verdict::Approved(content) => {
            // Write the stripped content and atomically replace the target.
            fs::write(&sidecar, &content)?;
            match fs::rename(&sidecar, &target) {
                Ok(()) => Ok(FileOutcome::Approved),
                Err(source) => {
                    // Keep the edited sidecar so the user can recover their work.
                    let _ = dir.keep();
                    Err(FileError::Persist {
                        target,
                        sidecar,
                        source,
                    })
                }
            }
        }
        // Abort/error: the tempdir (and the sidecar in it) is discarded on drop;
        // the target is left exactly as it was.
        Verdict::Aborted => Ok(FileOutcome::Aborted),
        Verdict::SilentFailure { elapsed_ms } => Err(FileError::SilentFailure { elapsed_ms }),
        Verdict::EditorFailed { code } => Err(FileError::EditorExited { code }),
    }
}

#[cfg(test)]
mod strip_tests {
    use super::*;

    #[test]
    fn no_scissors_returns_trimmed_whole() {
        assert_eq!(strip_scissors("hello world\n\n"), "hello world");
    }

    #[test]
    fn scissors_at_start_returns_empty() {
        let input = format!("{SCISSORS}\nfooter stuff");
        assert_eq!(strip_scissors(&input), "");
    }

    #[test]
    fn scissors_in_middle_returns_content_above() {
        let input = format!("my draft\nsecond line\n{SCISSORS}\n# context\n");
        assert_eq!(strip_scissors(&input), "my draft\nsecond line");
    }

    #[test]
    fn scissors_like_but_not_exact_is_ignored() {
        let input = "draft\n# --- >8 --- not the real one\nmore";
        assert_eq!(strip_scissors(input), input.trim_end());
    }

    #[test]
    fn round_trips_a_body_that_contains_a_marker() {
        // build_draft appends the footer last, so its marker is the LAST one;
        // cutting there preserves a body that itself contains a marker line.
        let body = format!("keep this\n{SCISSORS}\nand this too");
        assert_eq!(strip_scissors(&build_draft(&body, None)), body);
    }

    #[test]
    fn cuts_at_the_last_marker_not_the_first() {
        let input = format!("a\n{SCISSORS}\nb\n{SCISSORS}\nc");
        assert_eq!(strip_scissors(&input), format!("a\n{SCISSORS}\nb"));
    }
}

#[cfg(test)]
mod build_tests {
    use super::*;

    #[test]
    fn includes_content_and_scissors() {
        let draft = build_draft("my content", None);
        assert!(draft.starts_with("my content\n"));
        assert!(draft.contains(SCISSORS));
        assert!(draft.contains("# Empty all content above this line to abort"));
    }

    #[test]
    fn context_appears_when_provided() {
        let draft = build_draft("x", Some("Issue #26 reply"));
        assert!(draft.contains("# Context: Issue #26 reply"));
    }

    #[test]
    fn no_context_line_when_absent() {
        let draft = build_draft("x", None);
        assert!(!draft.contains("# Context:"));
    }

    #[test]
    fn round_trips_through_strip() {
        let draft = build_draft("hello\nworld", None);
        assert_eq!(strip_scissors(&draft), "hello\nworld");
    }
}

#[cfg(test)]
mod editor_tests {
    use super::*;
    use serial_test::serial;

    #[test]
    #[serial]
    fn visual_takes_priority() {
        std::env::set_var("VISUAL", "myvisual --wait");
        std::env::set_var("EDITOR", "myeditor");
        assert_eq!(resolve_editor().unwrap(), vec!["myvisual", "--wait"]);
    }

    #[test]
    #[serial]
    fn editor_used_when_visual_unset() {
        std::env::remove_var("VISUAL");
        std::env::set_var("EDITOR", "nano");
        assert_eq!(resolve_editor().unwrap(), vec!["nano"]);
    }

    #[test]
    #[serial]
    fn falls_back_to_vi_when_both_unset() {
        std::env::remove_var("VISUAL");
        std::env::remove_var("EDITOR");
        assert_eq!(resolve_editor().unwrap(), vec!["vi"]);
    }
}
