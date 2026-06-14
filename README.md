<div align="center">
  <h1>
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="assets/brand/logo-dark.svg">
      <img alt="scissors" src="assets/brand/logo-light.svg" height="96">
    </picture>
  </h1>
  <p>
    <a href="https://crates.io/crates/scissors"><img alt="crates.io" src="https://img.shields.io/crates/v/scissors.svg"></a>
    <a href="https://pypi.org/project/scissors/"><img alt="PyPI" src="https://img.shields.io/pypi/v/scissors.svg"></a>
    <a href="https://github.com/ggueret/scissors/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ggueret/scissors/actions/workflows/ci.yml/badge.svg"></a>
    <a href="LICENSE-MIT"><img alt="License: MIT OR Apache-2.0" src="https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg"></a>
  </p>
</div>

<div align="center">

`scissors` is a Unix CLI primitive for **editor-based content approval**,
modelled on git's `commit.cleanup=scissors` convention. It opens your content
in your editor and keeps the approved bytes (everything above the scissors
line). Use it as a stdin->stdout filter, or point it at a file to edit in
place.

</div>

---

## Install

```bash
pip install scissors        # PyPI (standalone binary, no Python needed at runtime)
uv tool install scissors    # uv
cargo install scissors      # crates.io
```

## Usage

```bash
# Approve a PR body before submission
gh pr view 42 --json body --jq .body | scissors > approved.md && \
  gh pr edit 42 --body-file approved.md

# Edit and confirm a draft, with context for multi-draft sessions
echo "$DRAFT" | scissors --context "Issue #26 reply" > /tmp/out

# Non-interactive: approve as-is, no editor (CI, scripts, agents)
echo "$DRAFT" | scissors --yes > /tmp/out
```

The editor opens with your content followed by a scissors line:

```
your content here

# ------------------------ >8 ------------------------
# Do not modify or remove the line above.
# Everything below is context and will be stripped from the final content.
# ...
```

Edit the content above the line, save, and close. Everything below the
scissors line is discarded.

## The editor buffer

`scissors` opens the draft as a file named `COMMIT_EDITMSG`, the same name
`git commit` uses. Editors that dim git commit messages (VS Code, Vim, Emacs
with Magit, ...) dim this buffer the same way, so the `#` footer reads as
greyed-out comments while you edit and the body stays plain. The cut is located
by the `>8` marker, so the footer is detected and stripped even if its `#`
prefix gets altered.

## Choosing the editor

`scissors` resolves the editor in order: `$SCISSORS_EDITOR`, then `$VISUAL`,
then `$EDITOR`, then `vi`. `SCISSORS_EDITOR` is a dedicated override, like git's
`GIT_EDITOR`: point `scissors` at a specific editor or flags without touching
`$VISUAL`/`$EDITOR`, which the rest of your shell relies on.

A GUI editor needs a blocking flag; a dedicated window also makes the review
easy to find when many windows are open:

```bash
export SCISSORS_EDITOR="code --wait --new-window"   # VS Code, dedicated window
```

Without a wait flag the editor returns immediately and `scissors` reports a
silent failure (exit 2).

## File mode

Pass a file to edit it in place, like `$EDITOR <file>`:

```bash
scissors notes.md          # opens notes.md, edits it in place
```

On approve, the approved content atomically replaces the file. On abort (you
emptied the buffer above the scissors line) or on any error, the file is left
exactly as it was: editing happens in a `COMMIT_EDITMSG` sidecar in a temp
directory beside the target, and the file is never touched until the final
atomic rename, so it is never left half-written even if `scissors` is
interrupted mid-edit. The atomic replace
gives the file a new inode, so hard links break and a symlink is written through
to its target. The mode bits are copied onto the new inode, but owner, group,
ACLs and extended attributes reset to the editing user's defaults, so avoid
`sudo scissors <file>` on a file whose ownership or security context must be
preserved. `scissors` prints the sidecar path to stderr at launch, so you
can reopen it if you lose the editor window. The outcome is signalled by the
exit code (0/1/2); nothing is written to stdout in this mode.

To review a file's content without editing it in place, pipe it through the stdin filter instead: `scissors < notes.md > approved.md`.

In-place mode suits agents and scripts: write the draft to a path, run a bare
`scissors <path>` (easy to put on an allowlist), then read the file back. No
pipe and no command substitution are required.

Pass `-` as the file (`scissors -`) to force the stdin/stdout path explicitly;
omitting the argument does the same.

## Exit codes

| Code | Meaning |
|------|---------|
| `0`  | approved -- file written in place (file mode), or content on stdout (stdin) |
| `1`  | aborted -- you emptied the content above the scissors line |
| `2`  | error -- no editor available, editor failed, non-UTF-8 input, or I/O error |

In stdin mode, on abort or error the draft tempfile is preserved and its path is printed to stderr. In file mode, on abort or error the file is left unchanged (the edit happens in a sidecar that is discarded).

## Editor resolution

`scissors` uses `$VISUAL`, then `$EDITOR`, then falls back to `vi`. For GUI
editors, set a blocking flag so the process waits for you to close the file:

```bash
export VISUAL="code --wait"
export VISUAL="subl -w"
```

## Non-interactive / headless use

`scissors` is **fail-closed**: without an editor it errors (exit 2) rather than
silently approving. For environments with no human at a terminal (CI, scripts,
AI agents), pass `--yes` to approve the input as-is without opening an editor:

```bash
echo "$DRAFT" | scissors --yes   # approve verbatim, exit 0
```

This mirrors `git commit` (interactive editor vs `-m`/`--no-edit`): the same
pipeline works interactively (edit) and unattended (`--yes`).

One asymmetry to note for scripts: with `--yes`, empty stdin is approved as-is
(exit 0), but an empty or whitespace-only *file* is rejected (exit 1), since
there is nothing to approve in place.

## Running under a sandbox

Some environments (AI agents, restricted CI, Claude Code) run commands in a
sandbox that blocks the IPC between a GUI editor's CLI (`code --wait`,
`cursor --wait`, `subl -w`) and the running editor process. The CLI then
returns immediately without opening anything, and `scissors` reports a
silent-failure error (exit 2). Three ways to make it work inside a sandbox:

1. **Use a TUI editor** -- `vi`, `nano`, `emacs -nw` read the TTY directly and
   need no cross-process IPC: `export VISUAL=nano`
2. **Exclude the GUI editor from the sandbox** -- if your sandbox has an
   allowlist (e.g. Claude Code's `sandbox.excludedCommands`), add the binary:
   `{ "sandbox": { "excludedCommands": ["code *"] } }`
3. **Point `$EDITOR` at a helper** the host can route to an editor running
   outside the sandbox.

`scissors` stays sandbox-agnostic: it honours `$EDITOR` like any Unix tool and
surfaces a clear error when the editor never opens.

## Library usage (Rust)

Both entry points take an `Options` builder so new knobs stay additive. Input
is UTF-8 text only; non-UTF-8 bytes surface as an error before anything is
written.

```rust
use scissors::{approve_in_editor, Options, Outcome};

// stdin-style: returns the approved text
match approve_in_editor("draft content", &Options::new().context("Issue #26 reply"))? {
    Outcome::Approved(text) => println!("approved: {text}"),
    Outcome::Aborted { draft_path } => eprintln!("aborted: {}", draft_path.display()),
}
```

```rust
use std::path::Path;
use scissors::{approve_file_in_place, FileOutcome, Options};

// in-place: the approved content is written back to the file
match approve_file_in_place(Path::new("notes.md"), &Options::new())? {
    FileOutcome::Approved => println!("notes.md updated"),
    FileOutcome::Aborted => eprintln!("aborted; notes.md unchanged"),
}
```

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option.
