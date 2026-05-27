#!/bin/sh
# Deterministic fake editor for scissors integration tests.
# The file to edit is the last argument. Behaviour is set via env vars:
#   MOCK_EDITOR_ACTION = approve | edit | abort | noop   (default: noop)
#   MOCK_EDITOR_EXIT   = exit code                       (default: 0)
file="$1"
case "${MOCK_EDITOR_ACTION:-noop}" in
  approve)
    # Replace the whole buffer with fresh approved content.
    printf 'approved content\n' > "$file"
    ;;
  edit)
    # Prepend a line, keeping the rest of the buffer (incl. scissors footer).
    tmp="$(cat "$file")"
    printf 'edited line\n%s\n' "$tmp" > "$file"
    ;;
  abort)
    # Empty the buffer entirely (nothing above the scissors line).
    : > "$file"
    ;;
  noop)
    # Touch nothing - simulates an editor that never opened (silent failure).
    ;;
esac
exit "${MOCK_EDITOR_EXIT:-0}"
