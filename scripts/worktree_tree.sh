#!/usr/bin/env bash
# worktree_tree.sh — print the git tree hash of the worktree exactly as
# `git add -A` would stage it, without touching the real index.
#
# The content address shared by ci_local.sh (which records the tree it
# tested in target/ci_local.pass) and lccc-snapshot.sh (which refuses to
# publish a tree without a matching pass): one definition, so the two can
# never disagree about what "the same tree" means.
set -euo pipefail
cd "${1:-.}"
idx=$(mktemp "${TMPDIR:-/tmp}/worktree-tree-index.XXXXXX")
trap 'rm -f "$idx"' EXIT
cp -f "$(git rev-parse --git-path index)" "$idx" 2>/dev/null || : >"$idx"
GIT_INDEX_FILE=$idx git add -A >/dev/null
GIT_INDEX_FILE=$idx git write-tree
