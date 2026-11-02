#!/usr/bin/env bash
# AUD-VABI-1: a vector crossing a compiler boundary must use the psABI rule
# (SSE + SSEUP = ONE %xmm), in BOTH directions, not lccc's own convention.
#
# The shape this pins: lccc's struct rule has no SSEUP notion, so a 16-byte
# vector was passed and returned as a two-register INTEGER pair.  That is
# invisible in an all-lccc build and fatal the moment a system library compiled
# by gcc returns or accepts a vector: `lccc main.o + gcc ext.o` read the result
# from %rdx:%rax where gcc had written %xmm0.
#
# The oracle is gcc itself, in all four combinations (each side compiled by each
# compiler), so the gate cannot pass by inventing a third convention that only
# lccc understands -- lccc/gcc must agree with gcc/gcc AND with gcc/lccc.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
gcc_bin=${GCC:-gcc}
src=$repo/tests/regression/vector_abi_boundary/vector_abi.c
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

[ -x "$ccc" ] || { echo "SKIP: $ccc not built" >&2; exit 0; }
command -v "$gcc_bin" >/dev/null 2>&1 || {
    echo "SKIP: oracle unavailable ($gcc_bin not found)" >&2; exit 0; }

fail() { echo "FAIL: $*" >&2; exit 1; }

# The callee half is built from the same source with main() compiled out, so
# both compilers see identical declarations on both sides.
for opt in -O0 -O2; do
    tag=${opt#-}
    "$gcc_bin" "$opt" -DCALLEE_ONLY -c "$src" -o "$tmp/callee_gcc$tag.o"
    "$ccc" "$opt" -DCALLEE_ONLY -c "$src" -o "$tmp/callee_lccc$tag.o"
    "$gcc_bin" "$opt" -c "$src" -o "$tmp/caller_gcc$tag.o"
    "$ccc" "$opt" -c "$src" -o "$tmp/caller_lccc$tag.o"

    # gcc/gcc is the reference the other three must match.
    "$gcc_bin" "$tmp/caller_gcc$tag.o" "$tmp/callee_gcc$tag.o" -o "$tmp/ref$tag"
    "$tmp/ref$tag" >"$tmp/ref$tag.out"

    # caller lccc -> callee gcc (lccc must SPEAK the ABI)
    "$gcc_bin" "$tmp/caller_lccc$tag.o" "$tmp/callee_gcc$tag.o" -o "$tmp/c1$tag" ||
        fail "$opt: linking lccc caller with gcc callee failed"
    "$tmp/c1$tag" >"$tmp/c1$tag.out"
    diff -u "$tmp/ref$tag.out" "$tmp/c1$tag.out" >&2 ||
        fail "$opt: lccc caller + gcc callee disagrees with gcc/gcc"

    # caller gcc -> callee lccc (lccc must UNDERSTAND the ABI)
    "$gcc_bin" "$tmp/caller_gcc$tag.o" "$tmp/callee_lccc$tag.o" -o "$tmp/c2$tag" ||
        fail "$opt: linking gcc caller with lccc callee failed"
    "$tmp/c2$tag" >"$tmp/c2$tag.out"
    # Compare THIS leg's output: an earlier version of this gate diffed
    # "$tmp/c1$tag.out" here, so the gcc-caller/lccc-callee leg -- the one that
    # proves lccc UNDERSTANDS the ABI -- passed vacuously on the back of the
    # previous leg's already-verified file.
    diff -u "$tmp/ref$tag.out" "$tmp/c2$tag.out" >&2 ||
        fail "$opt: gcc caller + lccc callee disagrees with gcc/gcc"

    # and lccc/lccc must not have been left behind by the fix
    "$ccc" -I"$repo/include" "$tmp/caller_lccc$tag.o" "$tmp/callee_lccc$tag.o" -o "$tmp/c3$tag" ||
        fail "$opt: linking the two lccc objects failed"
    "$tmp/c3$tag" >"$tmp/c3$tag.out"
    diff -u "$tmp/ref$tag.out" "$tmp/c3$tag.out" >&2 ||
        fail "$opt: lccc/lccc disagrees with gcc/gcc"
    echo "ok: $opt all four caller/callee combinations match gcc/gcc"

    # Liveness of the comparison itself.  A gate that diffs the wrong file
    # passes vacuously -- which is precisely what this gate did before review
    # (the gcc-caller/lccc-callee leg compared the previous leg's output).  A
    # callee that returns the wrong value MUST make one of those diffs fail;
    # if it does not, the comparison is not looking at the callee's result.
    "$ccc" "$opt" -DCALLEE_ONLY -DVABI_CALLEE_MUTATION -c "$src" -o "$tmp/callee_mut$tag.o"
    "$gcc_bin" "$tmp/caller_gcc$tag.o" "$tmp/callee_mut$tag.o" -o "$tmp/mut$tag"
    "$tmp/mut$tag" >"$tmp/mut$tag.out" || true
    if cmp -s "$tmp/ref$tag.out" "$tmp/mut$tag.out"; then
        fail "$opt: a deliberately wrong callee produced the reference output; the comparison is not live"
    fi
done

# Assert the CONVENTION, not the arithmetic: which registers the arguments
# ARRIVE in.  A v2df argument pair must arrive in %xmm1 (one register per
# argument) while the {double,double} control must arrive in %xmm2/%xmm3 (two
# registers per argument).  Checking the arrival registers stays true at every
# optimisation level; an arithmetic-operand match would only hold at -O0, where
# the optimizer has not yet spilled the arguments to their homes.
for opt in -O0 -O2; do
    tag=${opt#-}
    "$ccc" "$opt" -DCALLEE_ONLY -S "$src" -o "$tmp/callee$tag.s"
    awk '/^add_v2df:/{f=1} f{print} f&&/\.size/{exit}' "$tmp/callee$tag.s" >"$tmp/v2df$tag.s"
    grep -qE '%xmm1' "$tmp/v2df$tag.s" ||
        { cat "$tmp/v2df$tag.s" >&2;
          fail "$opt: add_v2df's second argument no longer arrives in %xmm1"; }
    awk '/^add_two_f64:/{f=1} f{print} f&&/\.size/{exit}' "$tmp/callee$tag.s" >"$tmp/two_f64$tag.s"
    grep -qE '%xmm3' "$tmp/two_f64$tag.s" ||
        { cat "$tmp/two_f64$tag.s" >&2;
          fail "$opt: the {double,double} control no longer arrives in four registers"; }
done

echo "PASS: vector arguments and returns use the psABI xmm convention across compilers"
