#!/usr/bin/env bash
# Publishes the badges in OUT_DIR to the `badges` branch, and adds the run's
# line to history.csv there, so coverage over time stays in git.
# Run by CI with GITHUB_TOKEN and GITHUB_REPOSITORY set.
#   tools/badges/publish.sh OUT_DIR
set -euo pipefail

out="${1:?usage: publish.sh OUT_DIR}"
remote="https://x-access-token:${GITHUB_TOKEN:?}@github.com/${GITHUB_REPOSITORY:?}.git"
branch=badges
work="$(mktemp -d)"

if git ls-remote --exit-code --heads "$remote" "$branch" >/dev/null; then
  git clone -q --depth 1 --branch "$branch" "$remote" "$work"
else
  git -C "$work" init -q -b "$branch"
  git -C "$work" remote add origin "$remote"
fi

cp "$out"/*.svg "$out"/summary.json "$work"/
if [ ! -f "$work/history.csv" ]; then
  echo "measured_at,commit,coverage,tests,crates" > "$work/history.csv"
fi
cat "$out/history-row.csv" >> "$work/history.csv"

git -C "$work" config user.name "github-actions[bot]"
git -C "$work" config user.email "41898282+github-actions[bot]@users.noreply.github.com"
git -C "$work" add -A
git -C "$work" commit -q -m "badges: ${GITHUB_SHA:0:7}"
git -C "$work" push -q origin "$branch"
