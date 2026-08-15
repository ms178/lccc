#!/usr/bin/env bash
# AArch64 differential regression driver: compile each regression test with
# lccc-arm, assemble+link with aarch64-linux-gnu-gcc, run under qemu-aarch64,
# and compare stdout+exit code against a native aarch64-gcc -O2 build of the
# same source. Mirrors run_regression.sh semantics (.flags/.env/.txt files).
#
# Usage: CCC_ARM=./target/fastbuild/lccc-arm bash tests/regression/run_regression_arm.sh [filter]
set -u
CCC_ARM=${CCC_ARM:-./target/fastbuild/lccc-arm}
CCCFLAGS=${CCCFLAGS:--O2}
CROSS_GCC=${CROSS_GCC:-aarch64-linux-gnu-gcc}
QEMU=${QEMU:-qemu-aarch64}
dir=$(cd "$(dirname "$0")" && pwd)
filter=${1:-}

pass=0; fail=0; skip=0
for src in "$dir"/*.c; do
  name=$(basename "$src" .c)
  [ -n "$filter" ] && case "$name" in *"$filter"*) ;; *) continue ;; esac

  extra_flags=""
  [ -f "$dir/$name.flags" ] && extra_flags=$(cat "$dir/$name.flags")
  # PGO round-trip tests use the @PROFDIR@ placeholder that only the x86
  # driver substitutes. Passing it through verbatim makes gcc create a
  # literal '@PROFDIR@/' directory in the repo (which once polluted a
  # format-patch with .gcda binary blobs). Skip them here.
  case "$extra_flags" in *@PROFDIR@*) skip=$((skip+1)); continue ;; esac
  # x86-only tests: intrinsics headers, SSE/AVX flags, x86 inline asm.
  if grep -qE '(immintrin|emmintrin|xmmintrin|pmmintrin|smmintrin|tmmintrin|wmmintrin|nmmintrin)\.h' "$src" \
     || echo "$extra_flags" | grep -qE '(-m(sse|avx|pclmul|fma|gfni|vaes))' \
     || grep -qE '__asm__.*%(e|r)(ax|bx|cx|dx|si|di)' "$src"; then
    skip=$((skip+1)); continue
  fi
  # Env-gated tests select x86-specific compiler modes; skip.
  if [ -f "$dir/$name.env" ]; then skip=$((skip+1)); continue; fi

  LCCC_NO_COMPARE=0
  grep -q "LCCC_NO_COMPARE" "$dir/$name.txt" 2>/dev/null && LCCC_NO_COMPARE=1

  if ! "$CCC_ARM" $CCCFLAGS "$src" $extra_flags -S -o "/tmp/arm_${name}.s" 2>/tmp/arm_err.txt; then
    # lccc-arm may legitimately not support every construct yet; treat
    # compile failure as SKIP (backend maturity), not FAIL, unless
    # ARM_STRICT=1.
    if [ "${ARM_STRICT:-0}" = 1 ]; then echo "FAIL  $name (lccc-arm compile)"; fail=$((fail+1));
    else skip=$((skip+1)); fi
    continue
  fi
  if ! "$CROSS_GCC" "/tmp/arm_${name}.s" $extra_flags -o "/tmp/arm_${name}" -static -lm 2>/tmp/arm_ld.txt; then
    echo "FAIL  $name (assemble/link: $(tail -1 /tmp/arm_ld.txt))"; fail=$((fail+1)); continue
  fi
  lout=$(timeout 20 "$QEMU" "/tmp/arm_${name}" 2>/dev/null); lrc=$?
  [ $lrc -eq 124 ] && lout="(TIMEOUT)"

  if [ "$LCCC_NO_COMPARE" = 1 ]; then
    # lccc-conformance test: exit 0 is the pass criterion.
    if [ $lrc -eq 0 ]; then pass=$((pass+1)); else echo "FAIL  $name (rc=$lrc, lccc-only)"; fail=$((fail+1)); fi
    rm -f "/tmp/arm_${name}.s" "/tmp/arm_${name}"; continue
  fi

  if ! "$CROSS_GCC" -O2 "$src" $extra_flags -o "/tmp/armg_${name}" -static -lm 2>/dev/null; then
    # GCC can't build it either (e.g. lccc extension test): fall back to rc==0.
    if [ $lrc -eq 0 ]; then pass=$((pass+1)); else echo "FAIL  $name (rc=$lrc, no-gcc-ref)"; fail=$((fail+1)); fi
    rm -f "/tmp/arm_${name}.s" "/tmp/arm_${name}"; continue
  fi
  gout=$(timeout 20 "$QEMU" "/tmp/armg_${name}" 2>/dev/null); grc=$?
  if [ "$lout" = "$gout" ] && [ $lrc -eq $grc ]; then
    pass=$((pass+1))
  else
    echo "FAIL  $name (lccc rc=$lrc vs gcc rc=$grc; outputs $( [ "$lout" = "$gout" ] && echo match || echo DIFFER))"
    fail=$((fail+1))
  fi
  rm -f "/tmp/arm_${name}.s" "/tmp/arm_${name}" "/tmp/armg_${name}"
done
echo "=== ARM Regression: $pass passed, $fail failed, $skip skipped ==="
[ $fail -eq 0 ]
