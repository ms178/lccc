#!/usr/bin/env bash
# Compile+run the CCC regression suite. Each test returns 0 on success.
# Usage: run_regression.sh [--compare-gcc]
set -u
CCC=${CCC:-./target/release/lccc}
CCCFLAGS=${CCCFLAGS:--O2}
COMPARE_GCC=0
[ "${1:-}" = "--compare-gcc" ] && COMPARE_GCC=1
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
pass=0; fail=0
for src in "$dir"/*.c; do
  name=$(basename "$src" .c)
  # Per-test flags: a sibling <name>.flags file adds compiler flags
  # (e.g. "-mavx2" for SIMD tests that need explicit ISA enablement).
  extra_flags=""
  if [ -f "$dir/$name.flags" ]; then
    extra_flags=$(cat "$dir/$name.flags")
  fi
  # Flags AFTER the source: GCC's driver drops library flags (-lm) that
  # appear before the object under --as-needed, so extra_flags must follow
  # the input file (LCCC accepts both orders; GCC does not).
  if $CCC $CCCFLAGS "$src" $extra_flags -o "/tmp/ccc_${name}" 2>/tmp/ccc_err.txt; then
    /tmp/ccc_${name} >/tmp/ccc_out.txt 2>&1; rc=$?
    if [ $rc -eq 0 ]; then
      if [ "$COMPARE_GCC" = 1 ]; then
        if command -v gcc >/dev/null && gcc -O2 "$src" $extra_flags -o /tmp/gcc_${name} 2>/dev/null \
           && /tmp/gcc_${name} >/tmp/gcc_out.txt 2>&1 \
           && diff -q /tmp/ccc_out.txt /tmp/gcc_out.txt >/dev/null; then
          echo "PASS  $name"; pass=$((pass+1))
        else echo "MISMATCH $name"; fail=$((fail+1)); fi
      else echo "PASS  $name"; pass=$((pass+1)); fi
    else echo "FAIL  $name (run rc=$rc) out=[$(cat /tmp/ccc_out.txt)]"; fail=$((fail+1)); fi
  else echo "FAIL  $name compile: $(tail -1 /tmp/ccc_err.txt)"; fail=$((fail+1)); fi
done
echo "=== Regression: $pass passed, $fail failed ==="
[ $fail -eq 0 ]
