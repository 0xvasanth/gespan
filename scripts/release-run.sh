#!/usr/bin/env bash
# usage: release-run.sh <workflow> <run title> [-f input=value ...]
# Runs a release workflow on $TAG and waits for it. A run with that title that
# already passed on the tag is not repeated, so a rerun of Release is safe.
set -euo pipefail
workflow="$1"
title="$2"
shift 2

find_run() {
  gh run list --workflow "$workflow" --branch "$TAG" --event workflow_dispatch --limit 20 \
    --json databaseId,displayTitle,status,conclusion \
    --jq "[.[] | select(.displayTitle == \"$title\")] | $1"
}

if [ -n "$(find_run 'map(select(.conclusion == "success")) | .[0].databaseId // empty')" ]; then
  echo "$title already passed."
  exit 0
fi
before="$(find_run '.[0].databaseId // 0')"
gh workflow run "$workflow" --ref "$TAG" "$@"
run=""
for _ in $(seq 1 30); do
  sleep 10
  run="$(find_run '.[0].databaseId // 0')"
  [ "$run" != "$before" ] && [ "$run" != 0 ] && break
  run=""
done
if [ -z "$run" ]; then
  echo "$title did not start." >&2
  exit 1
fi
gh run watch "$run" --exit-status --interval 60 >/dev/null
