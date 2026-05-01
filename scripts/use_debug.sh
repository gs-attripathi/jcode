#!/usr/bin/env bash
# Install the debug build as the daily-driver `jcode`. Copies the binary
# to ~/.jcode/builds/dev/jcode so it lives independently of the source
# tree, then points current + stable channel symlinks at it. Re-run this
# script after any `cargo build --bin jcode` to refresh the installed
# copy with the latest debug build.
#
# Use this when you want your everyday `jcode` to be the dev build —
# notably, when iterating on jcode itself and you want auto-spawned
# server processes to use your latest code instead of the old release.
#
# Reverse it with `scripts/install_release.sh` (or `scripts/install.sh`)
# which re-points the launcher at a real release.
#
# Paths after this script:
# - ~/.jcode/builds/dev/jcode     copy of target/debug/jcode
# - ~/.jcode/builds/current/jcode -> ~/.jcode/builds/dev/jcode
# - ~/.jcode/builds/stable/jcode  -> ~/.jcode/builds/dev/jcode (so
#   anything that auto-launches "stable" picks up the dev build)
# - ~/.local/bin/jcode            -> ~/.jcode/builds/current/jcode
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
src_bin="$repo_root/target/debug/jcode"

skip_build=false
if [[ "${1:-}" == "--no-build" ]]; then
  skip_build=true
  shift
fi
if [[ "$#" -gt 0 ]]; then
  echo "Usage: $0 [--no-build]" >&2
  exit 1
fi

if ! $skip_build; then
  echo "Building debug binary..."
  cargo build --bin jcode --manifest-path "$repo_root/Cargo.toml"
fi

if [[ ! -x "$src_bin" ]]; then
  echo "Debug binary not found: $src_bin" >&2
  echo "Run \`cargo build --bin jcode\` first or omit --no-build." >&2
  exit 1
fi

# Don't clobber a currently-running daily-driver: kill anything bound to
# the dev install path so the file copy below succeeds on macOS.
pkill -9 -f "$HOME/.jcode/builds/(dev|current|stable)/jcode" 2>/dev/null || true

builds_dir="$HOME/.jcode/builds"
dev_dir="$builds_dir/dev"
mkdir -p "$dev_dir" "$builds_dir/current" "$builds_dir/stable"
install -m 755 "$src_bin" "$dev_dir/jcode"

ln -sfn "$dev_dir/jcode" "$builds_dir/current/jcode"
ln -sfn "$dev_dir/jcode" "$builds_dir/stable/jcode"
printf 'dev\n' > "$builds_dir/current-version"
printf 'dev\n' > "$builds_dir/stable-version"

install_dir="${JCODE_INSTALL_DIR:-$HOME/.local/bin}"
mkdir -p "$install_dir"
ln -sfn "$builds_dir/current/jcode" "$install_dir/jcode"

echo "Installed debug build as daily driver:"
echo "  $dev_dir/jcode  ($(stat -f '%z' "$dev_dir/jcode" 2>/dev/null || stat -c '%s' "$dev_dir/jcode") bytes)"
echo "  $builds_dir/current/jcode -> $dev_dir/jcode"
echo "  $builds_dir/stable/jcode  -> $dev_dir/jcode"
echo "  $install_dir/jcode        -> $builds_dir/current/jcode"
echo ""
echo "After future rebuilds, re-run \`scripts/use_debug.sh\` to refresh"
echo "the installed copy. (cargo build alone won't update what \`jcode\` runs.)"

if ! echo "$PATH" | tr ':' '\n' | grep -qx "$install_dir"; then
  echo ""
  echo "Tip: add $install_dir to PATH if needed."
fi
