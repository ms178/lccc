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
  #
  # @PROFDIR@ expands to a directory unique to this checkout and this test.
  # The PGO tests previously hard-coded paths like /tmp/pgd_switch, so two
  # working trees on one machine (or a rerun racing a stale directory) shared
  # profile state and the first run after a rebuild reported spurious
  # mismatches. Each test now gets its own directory, wiped before use.
  extra_flags=""
  if [ -f "$dir/$name.flags" ]; then
    extra_flags=$(cat "$dir/$name.flags")
    case "$extra_flags" in
      *@PROFDIR@*)
        profdir="${TMPDIR:-/tmp}/lccc-prof.$(printf '%s' "$dir" | cksum | cut -d" " -f1)/$name"
        rm -rf "$profdir"; mkdir -p "$profdir"
        extra_flags=${extra_flags//@PROFDIR@/$profdir}
        ;;
    esac
  fi
  # Per-test environment: a sibling <name>.env file is sourced so tests can
  # select non-default compiler modes (e.g. LCCC_FORCE_SSE2=1 for the legacy
  # SSE2 vectorization path). GCC ignores these variables.
  # LCCC_NO_COMPARE=1 marks an lccc-conformance test whose GCC reference is a
  # DEFECTIVE oracle (GCC's own binary SIGILLs, returns wrong TLS/absolute-
  # symbol values, or cannot even compile the construct) — the test still runs
  # under lccc and must pass, but the invalid GCC comparison is skipped.
  LCCC_NO_COMPARE=0
  # Per-test env vars must not LEAK into later tests (set -a exports persist
  # across loop iterations: vectorize_f32_sum_sse2.env's LCCC_FORCE_SSE2=1
  # silently switched every alphabetically-later test to the SSE2 path).
  # Track and unset them after the test runs.
  _envfile_vars=""
  if [ -f "$dir/$name.env" ]; then
    _envfile_vars=$(sed -n 's/^\([A-Za-z_][A-Za-z_0-9]*\)=.*/\1/p' "$dir/$name.env")
    set -a
    # shellcheck disable=SC1090
    . "$dir/$name.env"
    set +a
  fi
  # Flags AFTER the source: GCC's driver drops library flags (-lm) that
  # appear before the object under --as-needed, so extra_flags must follow
  # the input file (LCCC accepts both orders; GCC does not).
  if $CCC $CCCFLAGS "$src" $extra_flags -o "/tmp/ccc_${name}" 2>/tmp/ccc_err.txt; then
    /tmp/ccc_${name} >/tmp/ccc_out.txt 2>&1; rc=$?
    if [ $rc -eq 0 ]; then
      if [ "$COMPARE_GCC" = 1 ] && [ "$LCCC_NO_COMPARE" = 0 ]; then
        if command -v gcc >/dev/null && gcc -O2 "$src" $extra_flags -o /tmp/gcc_${name} 2>/dev/null \
           && /tmp/gcc_${name} >/tmp/gcc_out.txt 2>&1 \
           && diff -q /tmp/ccc_out.txt /tmp/gcc_out.txt >/dev/null; then
          echo "PASS  $name"; pass=$((pass+1))
        else echo "MISMATCH $name"; fail=$((fail+1)); fi
      else echo "PASS  $name${LCCC_NO_COMPARE:+ (lccc-only)}"; pass=$((pass+1)); fi
    else echo "FAIL  $name (run rc=$rc) out=[$(cat /tmp/ccc_out.txt)]"; fail=$((fail+1)); fi
  else echo "FAIL  $name compile: $(tail -1 /tmp/ccc_err.txt)"; fail=$((fail+1)); fi
  # Undo this test's env file so it cannot affect subsequent tests.
  for _v in $_envfile_vars; do unset "$_v"; done
done

# Optional structural code-generation regressions.  C tests prove semantics;
# these checks also lock in assembly properties whose loss would be a silent
# performance regression.  Each script receives the same compiler via CCC.
for check in "$dir"/check_*.sh; do
  [ -e "$check" ] || continue
  name=$(basename "$check" .sh)
  if CCC="$CCC" bash "$check" >/tmp/ccc_out.txt 2>&1; then
    echo "PASS  $name"; pass=$((pass+1))
  else
    echo "FAIL  $name: $(tail -1 /tmp/ccc_out.txt)"; fail=$((fail+1))
  fi
done

echo "=== Regression: $pass passed, $fail failed ==="
[ $fail -eq 0 ]
