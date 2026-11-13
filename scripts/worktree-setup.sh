#!/usr/bin/env bash
# Prepares a linked git worktree for work:
#   - installs the UI dependencies with Bun when node_modules is missing or
#     older than bun.lock;
#   - seeds missing Cargo target folders with copy-on-write clones of other
#     checkouts' build output, so the first build only recompiles this
#     repository's own crates;
#   - prunes idle build output from other worktrees in the background.
# Runs from the Claude Code SessionStart hook, so it must stay quick and quiet
# when there is nothing to do. The main checkout is left alone.
#
# GESPAN_NO_TARGET_SEED=1 skips seeding; GESPAN_NO_TARGET_PRUNE=1 skips pruning.
set -euo pipefail

root="$(git rev-parse --show-toplevel 2>/dev/null)" || exit 0
cd "$root"

git_dir="$(cd "$(git rev-parse --git-dir)" && pwd -P)"
common_dir="$(cd "$(git rev-parse --git-common-dir)" && pwd -P)"
[ "$git_dir" != "$common_dir" ] || exit 0

scripts="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/worktree-lib.sh
. "$scripts/worktree-lib.sh"

install_dependencies() {
  if [ -d node_modules ] && [ ! bun.lock -nt node_modules ]; then
    return
  fi
  if ! command -v bun >/dev/null 2>&1; then
    echo "worktree-setup: bun not found; run 'bun install' in $root" >&2
    return
  fi
  echo "worktree-setup: installing dependencies in $root" >&2
  bun install --frozen-lockfile >&2
  touch node_modules
}

# Builds missing target folder $1 out of clones of other checkouts' build
# output: each profile (debug, native-dev, release) from the checkout with the
# most complete recent build of it, so a folder that was only checked or
# linted doesn't win over a real build.
seed_target() {
  local relative="$1" here staging profile best primary="" entry cloned=0
  here="$(real_dir "$root")"
  # Fill a folder beside the destination and rename it, so a failed or
  # interrupted clone never leaves a half-filled target folder for Cargo.
  staging="$relative.seeding.$$"
  rm -rf "$staging"
  mkdir "$staging"
  while IFS= read -r profile; do
    best="$(best_profile "$relative" "$profile" "$here")"
    [ -n "$best" ] || continue
    [ -n "$primary" ] || primary="$(dirname "$best")"
    if ! clone_tree "$best" "$staging/$profile" 2>/dev/null; then
      rm -rf "$staging"
      echo "worktree-setup: could not clone $relative without a full copy; leaving it to build" >&2
      return
    fi
    cloned=1
    echo "worktree-setup: cloned $relative/$profile from $best" >&2
  done < <(profiles "$relative")
  if [ "$cloned" = 0 ]; then
    rm -rf "$staging"
    return
  fi
  # Downloads kept beside the profiles, such as sherpa-onnx-prebuilt. Editor
  # check folders (flycheck*) and scratch space are left behind.
  for entry in "$primary"/*/; do
    entry="${entry%/}"
    [ ! -d "$entry/deps" ] || continue
    case "$(basename "$entry")" in flycheck* | tmp | doc) continue ;; esac
    clone_tree "$entry" "$staging/$(basename "$entry")" 2>/dev/null || true
  done
  mv "$staging" "$relative"
}

seed_targets() {
  local relative
  for relative in "${TARGET_DIRS[@]}"; do
    [ ! -e "$relative" ] || continue
    [ -d "$(dirname "$relative")" ] || continue
    seed_target "$relative"
  done
}

install_dependencies
[ "${GESPAN_NO_TARGET_SEED:-}" = 1 ] || seed_targets
if [ "${GESPAN_NO_TARGET_PRUNE:-}" != 1 ]; then
  nohup bash "$scripts/worktree-prune-builds.sh" >/dev/null 2>&1 &
fi
exit 0
