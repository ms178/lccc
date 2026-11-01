#!/bin/bash
# Run-diff every program of a C corpus between a reference compiler and LCCC.
#
#   scripts/diffrun.sh REF LCCC "FLAGS"      e.g. diffrun.sh gcc-16 target/fastbuild/lccc "-m32 -O1"
#   P=<dir> scripts/diffrun.sh ...            corpus directory (default tests/benchmark/programs)
#
# Both binaries get byte-identical FLAGS.  A program counts as `same` only if
# the reference run exits normally (no signal, no timeout) and LCCC reproduces
# its exit status and stdout byte for byte.  Every other outcome is printed:
#   SKIP-REF-COMPILE   the reference compiler rejects the program for FLAGS
#   CRASH-REF          the reference binary died from a signal (UB or a
#                      resource limit in the program: there is no oracle)
#   TIMEOUT            the reference binary ran longer than 60 s
#   FAIL-LCCC-COMPILE  LCCC rejects what the reference accepts   (counted diff)
#   DIFF               exit status or stdout differ               (counted diff)
# The reference compiler uses its own headers; only LCCC is pointed at the
# host GCC's freestanding include directory (stddef.h, x86 intrinsics).
# The stack limit is pinned to the Linux default (8 MiB): a deeply recursive
# program (tce_sum at -O1, where neither GCC nor LCCC turns the tail call
# into a loop) passes or crashes on the limit alone, so an inherited limit
# would make the verdict depend on the caller's shell.
set -u
ulimit -s 8192
REF=$1; LCCC=$2; FL=$3
P=${P:-$(cd "$(dirname "$0")/.." && pwd)/tests/benchmark/programs}
INC=$(gcc -print-file-name=include)
W=$(mktemp -d -p "${TMPDIR:-$HOME/.cache/tmp}")
trap 'rm -rf "$W"' EXIT
same=0; diff=0; skip=0
for f in "$P"/*.c; do
  b=$(basename "$f" .c)
  # shellcheck disable=SC2086
  $REF $FL "$f" -o "$W/a" -lm 2>/dev/null || { skip=$((skip+1)); echo "SKIP-REF-COMPILE $b"; continue; }
  # shellcheck disable=SC2086
  $LCCC $FL -I"$INC" "$f" -o "$W/b" -lm 2>/dev/null || { diff=$((diff+1)); echo "FAIL-LCCC-COMPILE $b"; continue; }
  timeout 60 "$W/a" >"$W/oa" 2>/dev/null; ra=$?
  if [ "$ra" = 124 ]; then skip=$((skip+1)); echo "TIMEOUT $b"; continue; fi
  if [ "$ra" -gt 128 ]; then skip=$((skip+1)); echo "CRASH-REF $b rc=$ra"; continue; fi
  timeout 60 "$W/b" >"$W/ob" 2>/dev/null; rb=$?
  if [ "$ra" = "$rb" ] && cmp -s "$W/oa" "$W/ob"; then same=$((same+1))
  else diff=$((diff+1)); echo "DIFF $b rc=$ra/$rb"; fi
done
echo "flags=$FL same=$same diff=$diff skip=$skip"
[ "$diff" = 0 ]
