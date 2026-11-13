# shellcheck shell=bash
# Helpers shared by worktree-setup.sh and worktree-prune-builds.sh. Sourced,
# not run.

# Cargo output folders, relative to a checkout. gespan-gpui is its own
# workspace, so it has its own target folder.
# shellcheck disable=SC2034 # used by the scripts that source this file
TARGET_DIRS=("target" "crates/gespan-gpui/target")

# Prints each checkout of this repository, one per line, the main one first.
worktree_paths() {
  git worktree list --porcelain | sed -n 's/^worktree //p'
}

# A folder's path with symlinks resolved (/tmp and /private/tmp are one place).
real_dir() {
  (cd "$1" 2>/dev/null && pwd -P) || printf '%s' "$1"
}

# Builds older than this many days only seed a worktree when nothing newer
# exists, since their dependencies have likely moved on.
RECENT_DAYS=3

# The newest modification time, in seconds, among the files that find prints
# for the arguments given.
newest_found() {
  if stat -c %Y / >/dev/null 2>&1; then
    find "$@" -exec stat -c %Y {} + 2>/dev/null | sort -n | tail -1
  else
    find "$@" -exec stat -f %m {} + 2>/dev/null | sort -n | tail -1
  fi
}

# Seconds since the epoch at which anything was last written to a target
# folder. Compiling, checking and linting all add files to
# target/<profile>/deps, .fingerprint and friends, which updates those
# folders, so two levels down is enough.
newest_mtime() {
  newest_found "$1" -maxdepth 2
}

# "<count> <newest>" for the .rlib files of a profile folder such as
# target/debug. cargo check and clippy write .rmeta files, and .rlib files only
# for build-script dependencies, so the count tells a complete build (about 840
# here) from one that was only checked (about 165).
rlib_stats() {
  local deps="$1/deps" count
  count="$(find "$deps" -maxdepth 1 -name '*.rlib' 2>/dev/null | wc -l | tr -d ' ')"
  if [ "$count" -gt 0 ]; then
    echo "$count $(newest_found "$deps" -maxdepth 1 -name '*.rlib')"
  else
    echo "0 0"
  fi
}

# The names of the profile folders (those holding deps/, such as debug or
# native-dev) under target folder $1 in any checkout.
profiles() {
  local relative="$1" tree deps
  worktree_paths | while IFS= read -r tree; do
    for deps in "$tree/$relative"/*/deps; do
      [ -d "$deps" ] || continue
      basename "$(dirname "$deps")"
    done
  done | sort -u
}

# The best copy of profile folder $2 under target folder $1, in any checkout
# but $3: the most complete one built in the last RECENT_DAYS days, or the most
# complete one at all when none is that recent. Prints nothing when there is
# none.
best_profile() {
  local relative="$1" profile="$2" skip="${3:-}" cutoff tree dir count newest recent
  cutoff=$(($(date +%s) - RECENT_DAYS * 86400))
  worktree_paths | while IFS= read -r tree; do
    tree="$(real_dir "$tree")"
    [ "$tree" != "$skip" ] || continue
    dir="$tree/$relative/$profile"
    [ -d "$dir/deps" ] || continue
    read -r count newest <<<"$(rlib_stats "$dir")"
    [ "$count" -gt 0 ] || continue
    recent=0
    [ "$newest" -lt "$cutoff" ] || recent=1
    printf '%s\t%s\t%s\t%s\n' "$recent" "$count" "$newest" "$dir"
  done | sort -t $'\t' -k1,1nr -k2,2nr -k3,3nr | sed -n 1p | cut -f4-
}

# Copies folder $1 to $2 only if the copy can share disk blocks with the
# original. A full copy of a build folder is tens of gigabytes, so anything
# that can't clone is refused rather than copied. $2 must not exist.
clone_tree() {
  local from="$1" to="$2"
  case "$(uname -s)" in
    Darwin)
      # cp -c falls back to a full copy when clonefile can't work, so check
      # first: both sides on one APFS volume.
      local mount_point
      mount_point="$(df -P "$(dirname "$to")" | awk 'NR == 2 { print $6 }')"
      mount | grep -F " on $mount_point (" | grep -q "(apfs" || return 1
      [ "$(stat -f %d "$from")" = "$(stat -f %d "$(dirname "$to")")" ] || return 1
      cp -c -pR "$from" "$to"
      ;;
    Linux)
      # --reflink=always fails instead of copying when the filesystem can't.
      cp -pR --reflink=always "$from" "$to"
      ;;
    *)
      return 1
      ;;
  esac
}
