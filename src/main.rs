use clap::Parser;

/// Editor-based content approval, git-commit style.
///
/// PREVIEW RELEASE: this 0.0.1 is a name-reservation placeholder.
/// The functional implementation ships in v0.1.0.
/// See https://github.com/ggueret/scissors for the roadmap.
#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    /// Optional context shown as a footer comment (functional in v0.1.0)
    #[arg(long, value_name = "TEXT")]
    context: Option<String>,
}

fn main() {
    let cli = Cli::parse();
    eprintln!("scissors {} - preview release", env!("CARGO_PKG_VERSION"));
    eprintln!();
    eprintln!("This 0.0.1 is a name-reservation placeholder. The functional");
    eprintln!("implementation (stdin -> editor -> approved stdout) ships in v0.1.0.");
    if let Some(ctx) = cli.context.as_deref() {
        eprintln!();
        eprintln!("note: --context {ctx:?} was provided but is not yet functional");
    }
    eprintln!();
    eprintln!("Track progress: https://github.com/ggueret/scissors");
}
