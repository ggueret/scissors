use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use scissors::{approve_file_in_place, approve_in_editor, Outcome, ScissorsError};

/// Editor-based content approval, git-commit style.
///
/// With a FILE argument, edits it in place. Without it (or with `-`), reads the
/// draft from stdin and prints the approved content (everything above the
/// scissors line) to stdout.
#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    /// File to edit in place. The caller owns it: scissors never creates or
    /// deletes it. Use `-` (or omit) to read the draft from stdin instead.
    #[arg(value_name = "FILE")]
    file: Option<PathBuf>,

    /// Edit a managed temp copy instead of FILE in place: the original is left
    /// untouched and the approved content is written to stdout.
    #[arg(long, requires = "file")]
    copy: bool,

    /// Optional context shown as a footer comment in the editor buffer
    #[arg(long, value_name = "TEXT")]
    context: Option<String>,

    /// Approve the input as-is without opening an editor (for non-interactive
    /// use, e.g. CI). Fail-closed: without this flag, a missing editor is an error.
    #[arg(long)]
    yes: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // In-place file mode: a real FILE (not the `-` stdin sentinel) and not
    // --copy. The result stays in the file; nothing is written to stdout, which
    // keeps a bare `scissors <file>` trivially allowlistable.
    if let Some(path) = cli.file.as_deref() {
        if path != Path::new("-") && !cli.copy {
            if cli.yes {
                // Approve as-is: the file already holds the draft, leave it.
                return ExitCode::SUCCESS;
            }
            return match approve_file_in_place(path, cli.context.as_deref()) {
                Ok(Outcome::Approved(_)) => ExitCode::SUCCESS,
                Ok(Outcome::Aborted { draft_path }) => {
                    eprintln!(
                        "scissors: aborted; draft restored at {}",
                        draft_path.display()
                    );
                    ExitCode::from(1)
                }
                Err(err) => {
                    eprintln!("scissors: {err}");
                    ExitCode::from(2)
                }
            };
        }
    }

    // stdin mode (no FILE, or the `-` sentinel) or --copy mode (a real FILE
    // copied into a managed tempfile): read the content, approve through a
    // scissors-owned tempfile, print to stdout. A real FILE is never touched.
    let content = match cli.file.as_deref() {
        Some(path) if path != Path::new("-") => match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("scissors: cannot read {}: {e}", path.display());
                return ExitCode::from(2);
            }
        },
        _ => {
            let mut buf = String::new();
            if let Err(e) = io::stdin().read_to_string(&mut buf) {
                eprintln!("scissors: failed to read stdin: {e}");
                return ExitCode::from(2);
            }
            buf
        }
    };

    if cli.yes {
        println!("{}", content.trim_end());
        return ExitCode::SUCCESS;
    }

    match approve_in_editor(&content, cli.context.as_deref()) {
        Ok(Outcome::Approved(approved)) => {
            println!("{approved}");
            io::stdout().flush().ok();
            ExitCode::SUCCESS
        }
        Ok(Outcome::Aborted { draft_path }) => {
            eprintln!(
                "scissors: aborted; draft preserved at {}",
                draft_path.display()
            );
            ExitCode::from(1)
        }
        Err(err) => {
            eprintln!("scissors: {err}");
            if matches!(err, ScissorsError::NoEditor) {
                eprintln!(
                    "scissors: hint: in a non-interactive environment, pass --yes \
                     to approve without editing"
                );
            }
            ExitCode::from(2)
        }
    }
}
