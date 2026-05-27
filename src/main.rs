use std::io::{self, Read, Write};
use std::process::ExitCode;

use clap::Parser;
use scissors::{approve_in_editor, Outcome, ScissorsError};

/// Editor-based content approval, git-commit style.
///
/// Reads draft content from stdin, opens it in your editor, and prints the
/// approved content (everything above the scissors line) to stdout.
#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
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

    let mut content = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut content) {
        eprintln!("scissors: failed to read stdin: {e}");
        return ExitCode::from(2);
    }

    if cli.yes {
        // Non-interactive approval: emit the content as-is, no editor.
        println!("{}", content.trim_end());
        return ExitCode::SUCCESS;
    }

    match approve_in_editor(&content, cli.context.as_deref()) {
        Ok(Outcome::Approved(approved)) => {
            // Trailing newline for POSIX text composability.
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
