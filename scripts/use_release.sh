#!/usr/bin/env bash
# Promote the current source tree to your daily-driver release: builds a
# release binary, installs it into ~/.jcode/builds/versions/<hash>/, and
# points the current+stable channel symlinks plus the launcher at it.
#
# Counterpart to scripts/use_debug.sh:
# - use_debug.sh   = "switch to dev build for testing"
# - use_release.sh = "I'm happy with the dev build, roll it out as my
#                     default release"
#
# Configs (~/.jcode/config.toml, auth.json, sessions/, mcp.json, memory/,
# etc.) are not touched by this script — they live in JCODE_HOME and
# carry over to whichever binary is on PATH automatically.
#
# Pass --fast to skip LTO for a quicker (but slower-running) release
# build; defaults to release-lto.
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"

# Don't fight a running daily-driver while we swap symlinks.
pkill -9 -f "$HOME/.jcode/builds/(dev|current|stable|versions)/jcode" 2>/dev/null || true
pkill -9 -f "$repo_root/target/(debug|release|release-lto)/jcode" 2>/dev/null || true

# Delegate the actual build + install to the existing release installer
# so this script stays a thin orchestration layer that one-shots the
# rollout from a clean state.
"$repo_root/scripts/install_release.sh" "$@"

echo ""
echo "Verifying:"
"$HOME/.local/bin/jcode" version 2>/dev/null | head -2 \
  || echo "  (warning: ~/.local/bin/jcode did not run; check PATH)"
