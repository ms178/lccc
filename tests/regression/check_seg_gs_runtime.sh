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
src=tests/regression/seg_gs_runtime/seg_gs_runtime.c
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-seg-gs-runtime.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

if [ "$(uname -m)" != "x86_64" ] || [ "$(uname -s)" != "Linux" ]; then
    echo "SKIP: seg_gs runtime test needs x86-64 Linux (arch_prctl ARCH_SET_GS)"
    exit 0
fi

fail=0
ref=""
if command -v gcc >/dev/null 2>&1; then
    gcc -O2 -o "$tmp/ref" "$src"
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
        echo "FAIL: lccc $opt runtime (rc=$rc)" >&2
        grep -v '^ok ' <<<"$out" >&2 || true
        fail=1
        continue
    fi
    if [ -n "$ref" ] && [ "$out" != "$ref" ]; then
        echo "FAIL: lccc $opt output differs from gcc -O2 oracle" >&2
        diff <(printf '%s\n' "$ref") <(printf '%s\n' "$out") >&2 || true
        fail=1
        continue
    fi
    echo "PASS: lccc $opt runtime$( [ -n "$ref" ] && echo ' (identical to gcc -O2)')"
done

exit "$fail"
