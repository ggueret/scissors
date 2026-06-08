//! Editor-based content approval, git-commit style.
//!
//! Opens content in the user's editor, lets them edit or approve it, and
//! returns the bytes above the scissors line. Modelled on git's
//! `commit.cleanup=scissors` convention.
//!
//! Input is UTF-8 text only: non-UTF-8 bytes surface as an I/O error before
//! anything is written.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::{Duration, Instant};

use tempfile::{Builder, TempDir};
use thiserror::Error;

/// The git scissors separator. `build_draft` ends the footer with this exact
/// line; [`strip_scissors`] cuts at its last occurrence.
pub const SCISSORS: &str = "# ------------------------ >8 ------------------------";

/// Editor returning faster than this with an unchanged file is treated as a
/// silent failure (editor never actually opened).
const MIN_ELAPSED_MS: u128 = 500;

/// The fixed editor-buffer filename. Editors that highlight git commit messages
/// key on this exact name (no extension triggers it), so the `#` footer renders
/// as dimmed comments wherever the user's `git commit` is dimmed.
const COMMIT_EDITMSG: &str = "COMMIT_EDITMSG";

/// True for a line that is exactly the scissors separator (trailing whitespace
/// ignored). The one predicate [`strip_scissors`] and the file-mode guard share.
fn is_scissors_line(line: &str) -> bool {
    line.trim_end() == SCISSORS
}

/// Optional knobs for [`approve_in_editor`] and [`approve_file_in_place`].
///
/// Construct via [`Options::new`]/[`Options::default`] and the builder methods.
/// Marked `#[non_exhaustive]` so future options stay additive (non-breaking).
///
/// ```
/// let opts = scissors::Options::new().context("Issue #26 reply");
/// ```
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct Options<'a> {
    /// One-line label shown as a footer comment in the editor buffer so the
    /// user knows which draft they are reviewing.
    pub context: Option<&'a str>,
}

impl<'a> Options<'a> {
    /// An empty set of options.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the context label shown in the editor footer.
    #[must_use]
    pub fn context(mut self, context: &'a str) -> Self {
        self.context = Some(context);
        self
    }
}

/// Result of a stdin-mode approval round-trip.
#[derive(Debug)]
pub enum Outcome {
    /// User saved with non-empty content above the scissors line.
    Approved(String),
    /// User emptied the content above the scissors line. The draft file is
    /// preserved so the user can recover it.
    Aborted { draft_path: PathBuf },
}

/// Errors from [`approve_in_editor`]. The failure cases preserve the draft file
/// and report its path so the user can recover their work.
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
/// left exactly as it was: editing happens in a sidecar that is discarded on
/// failure.
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
    #[error("cannot prepare the edit buffer in {}: {source}", dir.display())]
    Prepare { dir: PathBuf, source: io::Error },
    #[error("editor exited with code {code}")]
    EditorFailed { code: i32 },
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

/// Total, lossless conversion of a stdin-mode editor error onto the file-mode
/// surface. The stdin-only `draft_path` is dropped (file mode keeps no draft;
/// the sidecar path is reported by [`FileError::Persist`] instead).
impl From<ScissorsError> for FileError {
    fn from(e: ScissorsError) -> Self {
        match e {
            ScissorsError::NoEditor => FileError::NoEditor,
            ScissorsError::Io(io) => FileError::Io(io),
            ScissorsError::EditorFailed { code, .. } => FileError::EditorFailed { code },
            ScissorsError::SilentFailure { elapsed_ms, .. } => {
                FileError::SilentFailure { elapsed_ms }
            }
        }
    }
}

/// Return everything above the *last* scissors line, trimmed. If there is no
/// scissors line, return the whole input trimmed. Cutting at the last occurrence
/// (the footer is always appended last) preserves a body that itself contains a
/// scissors line, instead of silently discarding everything below it.
pub fn strip_scissors(raw: &str) -> String {
    let lines: Vec<&str> = raw.lines().collect();
    let cut = lines
        .iter()
        .rposition(|&l| is_scissors_line(l))
        .unwrap_or(lines.len());
    lines[..cut].join("\n").trim_end().to_string()
}

/// Assemble the editor buffer: the content, a blank line, then the scissors
/// footer with instructions (and optional context).
fn build_draft(content: &str, context: Option<&str>) -> String {
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
        out.push_str("#\n# Context: ");
        out.push_str(ctx);
        out.push('\n');
    }
    out
}

/// Resolve the editor command, honouring $VISUAL > $EDITOR > `vi`.
/// Returns the command split into program + args (e.g. `["code", "--wait"]`).
///
/// # Errors
/// [`ScissorsError::Io`] if `$VISUAL`/`$EDITOR` contains unbalanced quotes that
/// `shell-words` cannot split.
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
    // Abort wins over the silent-failure heuristic: emptying the buffer is a
    // deliberate abort, never a "the editor did nothing" misfire.
    if final_content.trim().is_empty() {
        return Ok(Verdict::Aborted);
    }
    if final_content == original.trim_end() && elapsed.as_millis() < MIN_ELAPSED_MS {
        return Ok(Verdict::SilentFailure {
            elapsed_ms: elapsed.as_millis() as u32,
        });
    }
    Ok(Verdict::Approved(final_content))
}

/// Persist the draft directory (leaking it) so the user can recover the file at
/// `path`, and return that path.
fn keep_draft(dir: TempDir, path: PathBuf) -> PathBuf {
    let _ = dir.keep();
    path
}

/// Open `content` in the user's editor and return the approved bytes.
///
/// - `Ok(Outcome::Approved(s))` -- user saved non-empty content.
/// - `Ok(Outcome::Aborted { .. })` -- user emptied the content above scissors;
///   the draft file is preserved at `draft_path`.
///
/// # Errors
/// All error cases preserve the draft file and report its path.
/// - [`ScissorsError::NoEditor`] -- `$VISUAL`/`$EDITOR` unset/empty and `vi` not found.
/// - [`ScissorsError::EditorFailed`] -- the editor exited non-zero.
/// - [`ScissorsError::SilentFailure`] -- the editor returned in under
///   `500 ms` with no change (likely never opened, e.g. a GUI editor without a
///   wait flag, or a sandbox blocking its IPC).
/// - [`ScissorsError::Io`] -- a tempfile or read/write failure, including
///   non-UTF-8 content.
pub fn approve_in_editor(content: &str, options: &Options<'_>) -> Result<Outcome, ScissorsError> {
    let editor = resolve_editor()?;

    // Edit in a COMMIT_EDITMSG file inside a tempdir so the editor dims the footer.
    let dir = Builder::new().prefix("scissors-").tempdir()?;
    let path = dir.path().join(COMMIT_EDITMSG);
    fs::write(&path, build_draft(content, options.context).as_bytes())?;

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
/// `path` is resolved with `fs::canonicalize`, so if it is a **symlink the link
/// target is edited** (the symlink itself is left pointing where it did).
/// Editing happens in a `COMMIT_EDITMSG` sidecar inside a temp directory beside
/// the target (so the editor dims the footer and the final rename is atomic on
/// the same filesystem); the target is never written until that rename. On abort
/// or any error the sidecar is discarded and the target is left exactly as it
/// was. The atomic replace installs a **new inode**: the mode bits are copied,
/// but owner, group, ACLs and extended attributes are reset to the editing
/// user's defaults, and hard links break.
///
/// # Errors
/// On every error the target is left exactly as it was.
/// - [`FileError::Read`] -- `path` cannot be read or canonicalized (missing,
///   permission denied, or non-UTF-8 content).
/// - [`FileError::MarkerInInput`] -- `path` already contains the scissors line;
///   refused before the editor opens to avoid truncating content below it.
/// - [`FileError::Prepare`] -- the sidecar could not be created in the target's
///   directory (e.g. a read-only directory).
/// - [`FileError::NoEditor`] / [`FileError::EditorFailed`] /
///   [`FileError::SilentFailure`] -- editor resolution / launch outcomes.
/// - [`FileError::Persist`] -- the final atomic rename failed; the edited draft
///   is kept at the reported sidecar path.
/// - [`FileError::Io`] -- another I/O failure.
pub fn approve_file_in_place(path: &Path, options: &Options<'_>) -> Result<FileOutcome, FileError> {
    // Resolve the real target ONCE (write through symlinks), then read through
    // the canonical path so the bytes shown and the rename destination are the
    // same inode (no TOCTOU) and the sidecar lands on the same filesystem.
    let target = fs::canonicalize(path).map_err(|source| FileError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let original = fs::read_to_string(&target).map_err(|source| FileError::Read {
        path: path.to_path_buf(),
        source,
    })?;

    // Fail closed: a target that already holds the scissors line would let the
    // editor (or a footer deletion) leave a marker that strips away real content
    // on the permanent on-disk write. Refuse before touching anything, using the
    // same predicate `strip_scissors` cuts on (the first hit is enough to refuse).
    if let Some(i) = original.lines().position(is_scissors_line) {
        return Err(FileError::MarkerInInput {
            path: path.to_path_buf(),
            line: i + 1,
        });
    }

    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let editor = resolve_editor().map_err(FileError::from)?;

    // Edit in a COMMIT_EDITMSG file inside a tempdir beside the target; the target
    // is untouched until the atomic rename on approve.
    let dir = Builder::new()
        .prefix(".scissors-")
        .tempdir_in(parent)
        .map_err(|source| FileError::Prepare {
            dir: parent.to_path_buf(),
            source,
        })?;
    let sidecar = dir.path().join(COMMIT_EDITMSG);
    // Write the draft and copy the target's mode bits onto the sidecar so the
    // replace doesn't change the file's mode. (Owner/group/ACLs/xattrs are NOT
    // preserved: the rename installs a new inode owned by the editing user.)
    fs::write(&sidecar, build_draft(&original, options.context).as_bytes())
        .and_then(|()| fs::set_permissions(&sidecar, fs::metadata(&target)?.permissions()))
        .map_err(|source| FileError::Prepare {
            dir: parent.to_path_buf(),
            source,
        })?;

    eprintln!("scissors: editing {}", sidecar.display());

    let verdict = edit_and_classify(&editor, &sidecar, &original).map_err(FileError::from)?;

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
        Verdict::EditorFailed { code } => Err(FileError::EditorFailed { code }),
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
