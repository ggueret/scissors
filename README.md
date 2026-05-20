# scissors

[![crates.io](https://img.shields.io/crates/v/scissors.svg)](https://crates.io/crates/scissors)
[![PyPI](https://img.shields.io/pypi/v/scissors.svg)](https://pypi.org/project/scissors/)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)

`scissors` is a Unix CLI primitive for **editor-based content approval**,
modelled on git's `commit.cleanup=scissors` convention. It opens content
in your editor and returns the approved bytes on stdout.

## What it will do (v0.1.0)

```bash
# Approve a PR body before submission
gh pr view 42 --json body --jq .body | scissors > approved.md

# Edit and confirm a draft, with context for multi-draft sessions
echo "$DRAFT" | scissors --context "Issue #26 reply" > /tmp/out
```

- Opens stdin in your `$VISUAL` / `$EDITOR` (or `vi` as fallback)
- Strips everything below the scissors line (`# ----- >8 -----`)
- Exit code signals status: `0` approved, `1` aborted, `2` error
- Returns approved content on stdout — composable in pipelines

## Roadmap

- **0.0.1** (this release) — name reservation, working `--help` and `--version`
- **0.1.0** (next) — full implementation: stdin -> editor -> stdout, exit codes for status, `--context` flag
- **Future** — `--json` flag for structured output, additional editor integrations

## Install (once 0.1.0 ships)

```bash
# Via PyPI
pip install scissors

# Via uv
uv tool install scissors

# Via Cargo
cargo install scissors

# Via Homebrew (planned)
brew install ggueret/scissors/scissors
```
