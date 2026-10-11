#!/usr/bin/env bash
# Executed regression for x86-64 named address space __seg_gs (D2, D4, D7).
#
# seg_gs_runtime/seg_gs_runtime.c points the GS base at a buffer and checks
# every __seg_gs load, store, aggregate copy and typedef pointer against values
# computed through ordinary memory. This script:
#
#   1. compiles the program with $CCC at -O0, -O1 and -O2, and assembles and
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
programs=(
    "tests/regression/seg_gs_runtime/seg_gs_runtime.c -O2"
    "tests/regression/seg_gs_runtime/seg_gs_audit.c -O0"
    "tests/regression/seg_gs_runtime/seg_gs_overread.c -O0"
    "tests/regression/seg_gs_runtime/seg_gs_overread.c -O2"
    "tests/regression/seg_gs_runtime/seg_gs_fold.c -O0"
    "tests/regression/seg_gs_runtime/seg_gs_fold.c -O2"
    "tests/regression/seg_gs_runtime/seg_gs_pointee_chain.c -O2"
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
ref=""
if command -v gcc >/dev/null 2>&1; then
    if ! gcc "$oracle_opt" -w -o "$tmp/ref" "$src"; then
        echo "FAIL: gcc $oracle_opt oracle failed to build $src" >&2
        fail=1
        return
    fi
    ref=$("$tmp/ref")
fi
for opt in -O0 -O1 -O2; do
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

if [ "$fail" -ne 0 ]; then
    echo "check_seg_gs_runtime: FAILED" >&2
    exit 1
fi
echo "check_seg_gs_runtime: PASSED"
exit 0
