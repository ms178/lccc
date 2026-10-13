#!/usr/bin/env bash
# Executed regression for x86-64 named address space __seg_gs.
#
# seg_gs_runtime/seg_gs_runtime.c points the GS base at a buffer and checks
# every __seg_gs load, store, aggregate copy and typedef pointer against values
# computed through ordinary memory. This script:
#
#   1. compiles the program with $CCC at -O0, -O1, -O2 and -O3, and assembles and
#      links it through the normal driver (not only -S), so a bad encoding or
#      a bad relocation fails here;
#   2. runs each binary and requires the final PASSED line and exit status 0;
#   3. when gcc is available, requires each lccc run to print exactly the same
#      lines as the gcc-built binary (GCC is the oracle for the accepted
#      behaviour; the self-checks are the pass/fail source of truth).
#
# Skips (exit 0, with a message) on non-x86-64 hosts: the test uses the
# x86-64 GS base and arch_prctl(ARCH_SET_GS).
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
. tests/regression/lib/gcc_oracle.sh
require_gcc_oracle "check_seg_gs_runtime" || exit 1
# Self-checking programs, each paired with the gcc optimisation level used as its
# oracle. Every program is built with $CCC at -O0, -O1 and -O2 and must print
# the PASSED line; when gcc is available its output must match the gcc build
# line for line.
#   seg_gs_runtime.c  offset-pointer forms; oracle gcc -O2.
#   seg_gs_audit.c    object-level forms (block extern, &object, conditional /
#                     comma / assignment aggregates, pointer to array, vector and
#                     complex objects, volatile reads, post-* object qualifier).
#                     Oracle gcc -O0: GCC 14.2 at -O2 stores a whole __seg_gs
#                     complex constant with a generic movaps (verified:
#                     `movaps %xmm0, cg_obj(%rip)`), so -O2 is not a valid oracle.
#   seg_gs_overread.c guard-page test: 1-7 and 9-15 byte struct returns and
#                     arguments must not read past the object (both oracles).
#   seg_gs_pointee_chain.c  a segment qualifier after an earlier `*` (the
#                     pointee of the outer pointer is in %gs); oracle gcc -O2.
#   seg_gs_fold.c     constant-displacement folding of every integer width,
#                     constant and register sources, read-modify-write, bases in
#                     many registers (both oracles).
#   seg_gs_complex_rvalue.c  complex segment rvalues (comma, argument, assignment,
#                     conditional) copied once; __real__/__imag__ at -O0.
#                     Oracle gcc -O0 (GCC 14.2 at -O2 stores a whole __seg_gs
#                     complex with a generic movaps).
#   seg_gs_vector_loop.c reductions, maps and dot products over __seg_gs arrays
#                     (the loops the vectorizer targets at -O3). Oracle gcc -O2.
#   seg_gs_decay.c    pointer arithmetic on decayed __seg_gs arrays and member
#                     arrays (array + n, n + array, array - n, pointer +=, row
#                     decay, conditional offsets), checked through a GS mirror;
#                     oracle gcc -O2.
#   seg_gs_big_copy.c aggregate copies of 24 to 4096 bytes in every direction,
#                     both sides of the 512-byte unroll limit, and a copy in a
#                     function with rsp-addressed locals. Oracle gcc -O2.
#   seg_gs_indexed.c  indexed element accesses `obj[i]`, `obj[i + k]` and
#                     `p[i]` through a parameter base, for every integer width
#                     and pointer elements, loads, stores, immediate stores and
#                     read-modify-write, checked on a GS mirror. Oracle gcc -O2.
programs=(
    "tests/regression/seg_gs_runtime/seg_gs_runtime.c -O2"
    "tests/regression/seg_gs_runtime/seg_gs_audit.c -O0"
    "tests/regression/seg_gs_runtime/seg_gs_overread.c -O0"
    "tests/regression/seg_gs_runtime/seg_gs_overread.c -O2"
    "tests/regression/seg_gs_runtime/seg_gs_fold.c -O0"
    "tests/regression/seg_gs_runtime/seg_gs_fold.c -O2"
    "tests/regression/seg_gs_runtime/seg_gs_pointee_chain.c -O2"
    "tests/regression/seg_gs_runtime/seg_gs_complex_rvalue.c -O0"
    "tests/regression/seg_gs_runtime/seg_gs_big_copy.c -O2"
    "tests/regression/seg_gs_runtime/seg_gs_decay.c -O2"
    "tests/regression/seg_gs_runtime/seg_gs_vector_loop.c -O2"
    "tests/regression/seg_gs_runtime/seg_gs_indexed.c -O0"
    "tests/regression/seg_gs_runtime/seg_gs_indexed.c -O2"
    "tests/regression/seg_gs_runtime/seg_gs_indexed.c -O3"
)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-seg-gs-runtime.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

if [ "$(uname -m)" != "x86_64" ] || [ "$(uname -s)" != "Linux" ]; then
    echo "SKIP: seg_gs runtime test needs x86-64 Linux (arch_prctl ARCH_SET_GS)"
    exit 0
fi

fail=0

# Every seg_gs_*.c program in the directory must be registered, so a new test
# cannot be added and then silently never run.
for f in tests/regression/seg_gs_runtime/seg_gs_*.c; do
    base=$(basename "$f")
    registered=0
    for entry in "${programs[@]}"; do
        case "$entry" in
            *"/$base "*) registered=1 ;;
        esac
    done
    if [ "$registered" -ne 1 ]; then
        echo "FAIL: $base is not registered in programs[]" >&2
        fail=1
    fi
done

run_program() {
src=$1
oracle_opt=$2
if ! "$GCC_ORACLE_BIN" "$oracle_opt" -w -o "$tmp/ref" "$src"; then
    echo "FAIL: gcc $oracle_opt oracle failed to build $src" >&2
    fail=1
    return
fi
ref=$("$tmp/ref")
for opt in -O0 -O1 -O2 -O3; do
    bin="$tmp/lccc$opt"
    if ! "$CCC" "$opt" -o "$bin" "$src"; then
        echo "FAIL: lccc $opt failed to compile or link $src" >&2
        fail=1
        continue
    fi
    set +e
    out=$("$bin")
    rc=$?
    set -e
    if [ "$rc" -ne 0 ] || ! grep -qx 'PASSED (0 failures)' <<<"$out"; then
        echo "FAIL: lccc $opt runtime $src (rc=$rc)" >&2
        grep -v '^ok ' <<<"$out" >&2 || true
        fail=1
        continue
    fi
    if [ -n "$ref" ] && [ "$out" != "$ref" ]; then
        echo "FAIL: lccc $opt output of $src differs from gcc $oracle_opt oracle" >&2
        diff <(printf '%s\n' "$ref") <(printf '%s\n' "$out") >&2 || true
        fail=1
        continue
    fi
    echo "PASS: $(basename "$src") lccc $opt$( [ -n "$ref" ] && echo " (identical to gcc $oracle_opt)")"
done
}

for entry in "${programs[@]}"; do
    # shellcheck disable=SC2086
    run_program $entry
done

# Shape check: a runtime pass cannot show that the indexed fold was used, since
# the remat path computes the same values. Count the SIB segment operands
# (`%gs:(%base, %idx, scale)`) in the -O2 build; the kill switch must remove them.
sib_count() {
    grep -cE '%gs:\(.*, *%[a-z0-9]+, *[1248]\)' "$1" || true
}
idx_src=tests/regression/seg_gs_runtime/seg_gs_indexed.c
"$CCC" -O2 -S -o "$tmp/idx_on.s" "$idx_src"
CCC_NO_SEG_SIB=1 "$CCC" -O2 -S -o "$tmp/idx_off.s" "$idx_src"
on=$(sib_count "$tmp/idx_on.s")
off=$(sib_count "$tmp/idx_off.s")
if [ "$on" -lt 18 ] || [ "$off" -ne 0 ]; then
    echo "FAIL: seg_gs_indexed.c -O2 has $on segment SIB operands (need >= 18) and $off with CCC_NO_SEG_SIB=1 (need 0)" >&2
    fail=1
else
    echo "PASS: seg_gs_indexed.c -O2 indexed segment operands: $on (CCC_NO_SEG_SIB=1: $off)"
fi

if [ "$fail" -ne 0 ]; then
    echo "check_seg_gs_runtime: FAILED" >&2
    exit 1
fi
echo "check_seg_gs_runtime: PASSED"
exit 0
