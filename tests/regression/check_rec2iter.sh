#!/usr/bin/env bash
# rec2iter gate: the recursion-to-iteration pass must still make fib(90) run.
#
# `tests/regression/fixtures/rec2iter/fib_rec2iter.c` is deliberately NOT a
# corpus test: the suite compiles every `tests/regression/*.c` with GCC too,
# and GCC has no rec2iter pass, so its `fib(90)` arm does not finish (measured:
# timeout after 30 s, i.e. ~10^12 years of work).  What the pass buys is
# therefore checked here: with LCCC the program must finish well inside the
# timeout and print "ALL PASS".
set -euo pipefail

CCC=$(realpath "${CCC:-./target/fastbuild/lccc}")
REPO=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
SRC="$REPO/tests/regression/fixtures/rec2iter/fib_rec2iter.c"
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

GCC_INC=$("${GCC_BIN:-gcc}" -print-file-name=include)
"$CCC" -O2 ${GCC_INC:+-I"$GCC_INC"} "$SRC" -o "$td/fib"
out=$(timeout 20 "$td/fib")
[ "$out" = "ALL PASS" ] || { echo "FAIL: rec2iter fixture printed '$out'" >&2; exit 1; }
echo "rec2iter: PASS (fib(90) computed and all values correct)"
