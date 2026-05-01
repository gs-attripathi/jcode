# CLAUDE.md

## Commit policy

When a new feature is added and verified working — either confirmed by the user, or fully covered by passing unit tests — commit those changes to the repo.

## Session files

Sessions are persisted as JSON under `$JCODE_HOME/sessions/` (default `$JCODE_HOME` is `~/.jcode`). Files are named `session_<animal>_<timestamp_ms>_<hash>.json`. Each one contains the full message history; sibling files include `*.bak` (last snapshot) and `*.journal.jsonl` (incremental updates).

When the user runs the binary with `JCODE_HOME=/tmp/jcode-test ...`, sessions land in `/tmp/jcode-test/sessions/` instead — this is the isolated-home pattern used to make sure a dev binary actually owns its sessions and isn't shadowed by an auto-spawned stable server on `~/.jcode/`.
