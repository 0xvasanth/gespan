#!/usr/bin/env bash
# Deletes Cargo build output from linked worktrees that haven't compiled
# anything for a while. Build output is a cache: a pruned worktree is seeded
# again by worktree-setup.sh on its next session, or rebuilds.
#
#   scripts/worktree-prune-builds.sh [--days N] [--dry-run]
#
# Never touches the main checkout or the worktree it runs in, and always keeps
# the copies worktree-setup.sh would clone from, so new worktrees have
# something to clone.
set -euo pipefail

days=14
dry_run=0
while [ $# -gt 0 ]; do
  case "$1" in
    --days) days="$2"; shift 2 ;;
    --dry-run) dry_run=1; shift ;;
    *) echo "usage: $0 [--days N] [--dry-run]" >&2; exit 2 ;;
  esac
done
case "$days" in
  '' | *[!0-9]*) echo "worktree-prune-builds: --days needs a whole number" >&2; exit 2 ;;
esac

root="$(git rev-parse --show-toplevel 2>/dev/null)" || exit 0
scripts="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/worktree-lib.sh
. "$scripts/worktree-lib.sh"

cutoff=$(( $(date +%s) - days * 86400 ))
root="$(real_dir "$root")"
main="$(real_dir "$(worktree_paths | sed -n 1p)")"

for relative in "${TARGET_DIRS[@]}"; do
  # The checkouts holding the best copy of some profile: worktree-setup.sh
  # clones from these, so they stay however idle they are.
  keep="$(
    profiles "$relative" | while IFS= read -r profile; do
      best="$(best_profile "$relative" "$profile")"
      [ -z "$best" ] || dirname "$best"
    done
  )"
  worktree_paths | while IFS= read -r tree; do
    tree="$(real_dir "$tree")"
    [ -d "$tree/$relative" ] || continue
    [ "$tree" != "$main" ] && [ "$tree" != "$root" ] || continue
    ! printf '%s\n' "$keep" | grep -Fxq "$tree/$relative" || continue
    time="$(newest_mtime "$tree/$relative")"
    [ -n "$time" ] && [ "$time" -lt "$cutoff" ] || continue
    if [ "$dry_run" = 1 ]; then
      echo "would remove $tree/$relative"
    else
      echo "removing $tree/$relative"
      rm -rf "${tree:?}/$relative"
    fi
  done
done
