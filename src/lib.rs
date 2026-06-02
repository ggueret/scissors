//! Editor-based content approval, git-commit style.
//!
//! Opens content in the user's editor, lets them edit or approve it, and
//! returns the bytes above the scissors line. Modelled on git's
//! `commit.cleanup=scissors` convention.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::{Duration, Instant};

use tempfile::Builder;
use thiserror::Error;

/// The canonical git scissors separator. Everything below it is stripped.
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

/// Return everything above the scissors line, trimmed. If there is no
/// scissors line, return the whole input trimmed.
pub fn strip_scissors(raw: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    for line in raw.lines() {
        if line.trim_end() == SCISSORS {
            break;
        }
        kept.push(line);
    }
    kept.join("\n").trim_end().to_string()
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

/// Open `content` in the user's editor and return the approved bytes.
///
/// - `Ok(Outcome::Approved(s))` -- user saved non-empty content.
/// - `Ok(Outcome::Aborted { .. })` -- user emptied the content above scissors.
/// - `Err(..)` -- no editor, editor failure, silent failure, or I/O error.
///   Aborted and the failure cases preserve the draft file for recovery.
pub fn approve_in_editor(content: &str, context: Option<&str>) -> Result<Outcome, ScissorsError> {
    let editor = resolve_editor()?;
    let draft = build_draft(content, context);

    let mut tmp = Builder::new()
        .prefix("scissors-")
        .suffix(".md")
        .tempfile()?;
    tmp.write_all(draft.as_bytes())?;
    tmp.flush()?;
    let path = tmp.path().to_path_buf();

    let (status, elapsed) = launch_editor(&editor, &path)?;

    if !status.success() {
        let (_f, draft_path) = tmp.keep().map_err(|e| e.error)?;
        return Err(ScissorsError::EditorFailed {
            code: status.code().unwrap_or(-1),
            draft_path,
        });
    }

    let raw_after = fs::read_to_string(&path)?;
    let final_content = strip_scissors(&raw_after);
    let unchanged = final_content == content.trim_end();

    if unchanged && elapsed.as_millis() < MIN_ELAPSED_MS {
        let (_f, draft_path) = tmp.keep().map_err(|e| e.error)?;
        return Err(ScissorsError::SilentFailure {
            elapsed_ms: elapsed.as_millis() as u32,
            draft_path,
        });
    }

    if final_content.trim().is_empty() {
        let (_f, draft_path) = tmp.keep().map_err(|e| e.error)?;
        return Ok(Outcome::Aborted { draft_path });
    }

    // Approved: tmp drops here and the file is removed.
    Ok(Outcome::Approved(final_content))
}

/// Open `path` in the user's editor, edited in place. The caller owns `path`:
/// this never creates or deletes it.
///
/// Editing happens in a sidecar tempfile in the same directory as the target;
/// the target is never written until the final atomic rename on approve. On
/// abort or any error the sidecar is discarded and the target is left exactly
/// as it was.
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

    // Resolve the real target (write through symlinks) so the sidecar lives on
    // the same filesystem and the rename is atomic.
    let target = fs::canonicalize(path).map_err(|source| FileError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let dir = target.parent().unwrap_or_else(|| Path::new("."));

    let editor = resolve_editor().map_err(|e| match e {
        ScissorsError::Io(io) => FileError::Io(io),
        other => FileError::Io(io::Error::other(other.to_string())),
    })?;

    let mut tmp = Builder::new()
        .prefix(".scissors-")
        .suffix(".tmp")
        .tempfile_in(dir)?;

    // Preserve permissions so the rename doesn't change the file's mode.
    let perms = fs::metadata(&target)?.permissions();
    fs::set_permissions(tmp.path(), perms)?;

    tmp.write_all(build_draft(&original, context).as_bytes())?;
    tmp.flush()?;

    eprintln!("scissors: editing {}", tmp.path().display());

    let (status, elapsed) = launch_editor(&editor, tmp.path()).map_err(|e| match e {
        ScissorsError::NoEditor => FileError::NoEditor,
        ScissorsError::Io(io) => FileError::Io(io),
        other => FileError::Io(io::Error::other(other.to_string())),
    })?;

    if !status.success() {
        return Err(FileError::EditorExited {
            code: status.code().unwrap_or(-1),
        });
    }

    let raw = fs::read_to_string(tmp.path()).map_err(|source| FileError::Read {
        path: tmp.path().to_path_buf(),
        source,
    })?;
    let final_content = strip_scissors(&raw);
    let unchanged = final_content == original.trim_end();

    if unchanged && elapsed.as_millis() < MIN_ELAPSED_MS {
        return Err(FileError::SilentFailure {
            elapsed_ms: elapsed.as_millis() as u32,
        });
    }

    if final_content.trim().is_empty() {
        return Ok(FileOutcome::Aborted);
    }

    // Approve: write the stripped content and atomically replace the target.
    // This persist is the only mutation of the target, and it is atomic.
    fs::write(tmp.path(), &final_content)?;
    match tmp.persist(&target) {
        Ok(_) => Ok(FileOutcome::Approved),
        Err(e) => {
            let source = e.error;
            // Keep the edited sidecar so the user can recover their work.
            let sidecar = match e.file.keep() {
                Ok((_file, path)) => path,
                Err(keep_err) => return Err(FileError::Io(keep_err.error)),
            };
            Err(FileError::Persist {
                target: target.clone(),
                sidecar,
                source,
            })
        }
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
