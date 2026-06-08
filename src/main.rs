use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use scissors::{
    approve_file_in_place, approve_in_editor, FileOutcome, Options, Outcome, ScissorsError,
};

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

    let opts = match cli.context.as_deref() {
        Some(ctx) => Options::new().context(ctx),
        None => Options::new(),
    };

    // In-place file mode: a real FILE (not the `-` stdin sentinel).
    if let Some(path) = cli.file.as_deref() {
        if path != Path::new("-") {
            if cli.yes {
                // Approve as-is without an editor: the file must exist and be non-empty.
                return match fs::read_to_string(path) {
                    Ok(c) if !c.trim().is_empty() => ExitCode::SUCCESS,
                    Ok(_) => {
                        eprintln!("scissors: {} is empty; nothing to approve", path.display());
                        ExitCode::from(1)
                    }
                    Err(e) => {
                        eprintln!("scissors: cannot read {}: {e}", path.display());
                        ExitCode::from(2)
                    }
                };
            }
            return match approve_file_in_place(path, &opts) {
                Ok(FileOutcome::Approved) => ExitCode::SUCCESS,
                Ok(FileOutcome::Aborted) => {
                    eprintln!("scissors: aborted; {} left unchanged", path.display());
                    ExitCode::from(1)
                }
                Err(e) => {
                    eprintln!("scissors: {e}");
                    ExitCode::from(2)
                }
            };
        }
    }

    // stdin mode (no FILE, or the `-` sentinel): read stdin, approve via a managed
    // tempfile, print the approved content to stdout.
    let mut content = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut content) {
        eprintln!("scissors: failed to read stdin: {e}");
        return ExitCode::from(2);
    }

    if cli.yes {
        println!("{}", content.trim_end());
        return ExitCode::SUCCESS;
    }

    match approve_in_editor(&content, &opts) {
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
