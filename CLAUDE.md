# CLAUDE.md

## Commit policy

When a new feature is added and verified working — either confirmed by the user, or fully covered by passing unit tests — commit those changes to the repo.

## Session files

Sessions are persisted as JSON under `$JCODE_HOME/sessions/` (default `$JCODE_HOME` is `~/.jcode`). Files are named `session_<animal>_<timestamp_ms>_<hash>.json`. Each one contains the full message history; sibling files include `*.bak` (last snapshot) and `*.journal.jsonl` (incremental updates).

When the user runs the binary with `JCODE_HOME=/tmp/jcode-test ...`, sessions land in `/tmp/jcode-test/sessions/` instead — this is the isolated-home pattern used to make sure a dev binary actually owns its sessions and isn't shadowed by an auto-spawned stable server on `~/.jcode/`.

## Dev → release workflow

Two scripts in `scripts/` are the only commands you should suggest for switching between debug and release builds:

- **`scripts/use_debug.sh`** — builds `target/debug/jcode`, copies it to `~/.jcode/builds/dev/jcode`, and points the launcher (`~/.local/bin/jcode`) plus the `current` and `stable` channels at it. Use this when the user wants to test new code as their daily driver. Re-run after any rebuild to refresh the installed copy.
- **`scripts/use_release.sh`** — builds release-LTO (or `--fast` for plain release), installs a versioned binary, repoints all symlinks to it. Use this when the user has tested a dev build and wants to promote it to their default release.

Both scripts kill any running jcode processes first to avoid fighting an in-flight server during the symlink swap. Configs (`~/.jcode/config.toml`, `auth.json`, `mcp.json`, `sessions/`, `memory/`, `todos/`) live in `JCODE_HOME` and are independent of which binary is installed — switching dev↔release never loses state.

**Don't manually edit symlinks or copy binaries around** unless one of these scripts is broken; if you need a one-off, use `JCODE_HOME=/tmp/something <abs path to binary>` instead so the user's real `~/.jcode/` stays untouched.

**Architecture note:** jcode is a client-server CLI. The `--provider auto serve` process is the long-running server holding session state, MCP connections, and the agent loop; the TUI you launch is just a client that connects to its Unix socket (`$JCODE_HOME/jcode.sock`). When the user reports "my changes aren't running", check `ps aux | grep jcode` — if there's a server process started before the latest install, the new TUI client is just attaching to that old server. `pkill -9 -f 'target/(debug|release)/jcode'` plus `pkill -9 -f 'builds/.*/jcode'` clears it; the next launch spawns a fresh server with the current binary. Both `use_debug.sh` and `use_release.sh` already do this kill step, so this only bites with manual launches.
