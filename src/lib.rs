//! Editor-based content approval, git-commit style.
//!
//! Opens content in the user's editor, lets them edit or approve it, and
//! returns the bytes above the scissors line. Modelled on git's
//! `commit.cleanup=scissors` convention.

use std::io;
use std::path::PathBuf;
use thiserror::Error;

/// The canonical git scissors separator. Everything below it is stripped.
pub const SCISSORS: &str = "# ------------------------ >8 ------------------------";

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

    #[error("editor exited with code {code}; draft preserved at {draft_path}")]
    EditorFailed { code: i32, draft_path: PathBuf },

    #[error(
        "editor returned in {elapsed_ms}ms without changes; it likely never \
         opened. Causes: $EDITOR missing a wait flag (e.g. `code --wait`), a \
         sandbox blocking the editor's IPC, or the binary not on PATH; \
         draft preserved at {draft_path}"
    )]
    SilentFailure { elapsed_ms: u32, draft_path: PathBuf },

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
