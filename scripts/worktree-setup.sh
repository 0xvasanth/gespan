#!/usr/bin/env bash
# Prepares a linked git worktree for work: installs the UI dependencies with Bun
# when node_modules is missing or older than bun.lock. Runs from the Claude Code
# SessionStart hook, so it must stay quick and quiet when there is nothing to do.
# The main checkout is left alone.
set -euo pipefail

root="$(git rev-parse --show-toplevel 2>/dev/null)" || exit 0
cd "$root"

git_dir="$(cd "$(git rev-parse --git-dir)" && pwd -P)"
common_dir="$(cd "$(git rev-parse --git-common-dir)" && pwd -P)"
[ "$git_dir" != "$common_dir" ] || exit 0

if [ -d node_modules ] && [ ! bun.lock -nt node_modules ]; then
  exit 0
fi

if ! command -v bun >/dev/null 2>&1; then
  echo "worktree-setup: bun not found; run 'bun install' in $root" >&2
  exit 0
fi

echo "worktree-setup: installing dependencies in $root" >&2
bun install --frozen-lockfile >&2
touch node_modules
