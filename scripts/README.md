# scripts/

Build and install helpers for the `jcode` binary.

## Two-step workflow for adding a feature

1. **Build a debug binary and use it as your daily driver.** Edit code, run:
   ```
   scripts/use_debug.sh
   ```
   Builds `target/debug/jcode`, copies it to `~/.jcode/builds/dev/jcode`,
   and points `~/.local/bin/jcode` (and the `current` + `stable`
   channels) at it. Plain `jcode` from PATH now runs the dev build.
   Re-run after each rebuild to refresh the installed copy.

2. **Promote dev to your default release once you're happy.**
   ```
   scripts/use_release.sh
   ```
   Builds an LTO release and installs it into a versioned path; the
   launcher and channels switch back to the release. Pass `--fast` to
   skip LTO if you want a faster build at the cost of a slower-running
   binary.

Configs (`~/.jcode/config.toml`, auth, sessions, MCP, memory, todos)
live in `JCODE_HOME` (default `~/.jcode/`) and are independent of which
binary is installed — both scripts preserve them automatically.

## Other scripts

- `install_release.sh` — what `use_release.sh` calls under the hood.
  Does the actual cargo build and symlink dance.
- `install.sh` — installs the published "stable" channel from a release
  archive (used by end users, not for self-dev).
- `remote_build.sh` — offload heavy cargo work to another machine. Use
  when local builds run out of memory; see `AGENTS.md` for details.

## Sandbox testing without touching your real `~/.jcode/`

If you want to test risky changes without putting them in front of your
real session history / configs, prefix any `jcode` invocation with a
disposable home:

```
JCODE_HOME=/tmp/jcode-test /Users/attripathi/projects/jcode/target/debug/jcode
```

That spawns a fresh server in `/tmp/jcode-test/` with its own sessions,
config, auth, etc. — your real `~/.jcode/` is untouched. Wipe with
`rm -rf /tmp/jcode-test` when done.
