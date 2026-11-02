#!/usr/bin/env bash
# Reproduce the before/after table of §20.  Usage:
#   ./ab.sh <baseline-bindir> <candidate-bindir>
# Each bindir is a directory holding lccc and the lccc-{i686,arm,riscv} drivers
# (for the baseline: build origin/main into its own CARGO_TARGET_DIR).
set -euo pipefail
base=${1:?baseline bindir}; cand=${2:?candidate bindir}
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
for t in "" -i686 -arm -riscv; do
    name=${t#-}; name=${name:-x86_64}
    "$base/lccc$t" -O2 -S "$here/probe.c" -o "$tmp/b$t.s"
    "$cand/lccc$t" -O2 -S "$here/probe.c" -o "$tmp/c$t.s"
    echo "== $name (before)"
    python3 "$here/count.py" "$tmp/b$t.s"
    echo "== $name (after)"
    python3 "$here/count.py" "$tmp/c$t.s"
done
